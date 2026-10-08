use super::App;
use crate::{
    codex::{CompactRequest, ExecutionRequest, ExecutionUpdate},
    message::IncomingMessage,
    model::settings::SessionState,
    qq::parse_output,
    state::outbox::Delivery,
};
use anyhow::Result;
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
impl App {
    pub(super) async fn run_turn(
        self: &Arc<Self>,
        message: IncomingMessage,
        ids: &[String],
        originals: &[IncomingMessage],
    ) -> Result<()> {
        let user = &message.sender_openid;
        let snapshot = self.session.snapshot_for_user(user).await?;
        let settings = snapshot.effective_settings();
        let expected = snapshot.foreground.clone();
        let dialog = self.state.dialog_id(user, expected.session_id.as_deref())?;
        for original in originals {
            self.state.record_message(
                user,
                dialog,
                "user",
                &original.text,
                Some(&original.message_id),
            )?;
        }
        let relevant =
            self.memory
                .search(user, &message.text, None, self.config.memory.relevant_limit)?;
        let relevant = relevant
            .iter()
            .map(|m| format!("[{}] {}", m.id, m.content))
            .collect::<Vec<_>>()
            .join("\n");
        let mut prompt = crate::codex::build_prompt(&message, Some(&relevant));
        let root = self
            .config
            .general
            .data_dir
            .parent()
            .unwrap_or(&self.config.general.data_dir);
        let loaded = self.state.loaded_versions(user, dialog)?;
        let new_thread = expected.session_id.is_none();
        let (instructions, profile_version, persona_version) = if new_thread {
            let (stable, profile_version, persona_version) = crate::memory::inject::stable(
                &self.memory,
                user,
                root,
                self.config.memory.hot_memory_max_chars,
            )?;
            (Some(stable), profile_version, persona_version)
        } else {
            let (profile, profile_version) = self.memory.profile(user)?;
            if loaded.0 != profile_version {
                prompt = format!(
                    "<profile-update>\nUSER PROFILE\n{profile}\n</profile-update>\n{prompt}"
                );
            }
            (None, profile_version, loaded.1)
        };
        let cancel = self
            .active_turns
            .lock()
            .await
            .install(user, dialog, &message.message_id, ids);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let request = ExecutionRequest {
            prompt,
            workspace_dir: expected.workspace_dir.clone(),
            codex_home: self.session.codex_home().to_owned(),
            config_overrides: Vec::new(),
            add_dirs: vec![
                self.session.workspace_for(user),
                self.session.inbox_for(user),
            ],
            session_state: SessionState {
                session_id: expected.session_id.clone(),
                settings: settings.clone(),
            },
            model: settings.model_override.clone().or_else(|| {
                (!self.config.general.default_model.is_empty())
                    .then(|| self.config.general.default_model.clone())
            }),
            service_tier: settings.service_tier,
            context_mode: settings.context_mode,
            reasoning_effort: settings
                .reasoning_effort
                .unwrap_or(self.config.general.default_reasoning_effort),
            image_paths: message
                .images
                .iter()
                .map(|a| a.local_path.clone())
                .collect(),
            developer_instructions: instructions,
            ephemeral: false,
            owner_user_id: Some(user.clone()),
        };
        let mut task = Box::pin(self.codex.execute(request, Some(cancel), Some(tx)));
        let mut thread = expected.session_id.clone();
        let output = loop {
            tokio::select! {
             result=&mut task=>break result,
             update=rx.recv()=>match update{
              Some(ExecutionUpdate::SessionStarted{session_id})=>{thread=Some(session_id.clone());self.active_turns.lock().await.bind(user,&session_id,None);
               self.session.bind_turn_result(user,&expected,Some(session_id),expected.profile.clone().unwrap_or_default(),None,false).await.unwrap_or_else(|error|{tracing::error!(%error,"failed to persist thread binding");None});
              },
              Some(ExecutionUpdate::TurnStarted{thread_id,turn_id})=>{self.active_turns.lock().await.bind(user,&thread_id,Some(&turn_id));},
              Some(ExecutionUpdate::ToolCall{display})=>{if settings.verbose{let _=self.reply_text(user,&message.message_id,&display).await;}},
              Some(ExecutionUpdate::AgentMessage{text,phase})=>{
               if phase.as_deref()==Some("commentary"){
                if let Err(error)=self.reply_text(user,&message.message_id,&text).await{
                 tracing::warn!(user=%user,%error,"failed to deliver live commentary");
                }
               }
              },None=>{}
             }
            }
        };
        while let Ok(update) = rx.try_recv() {
            if let ExecutionUpdate::SessionStarted { session_id } = update {
                thread = Some(session_id.clone());
                self.session
                    .bind_turn_result(
                        user,
                        &expected,
                        Some(session_id),
                        expected.profile.clone().unwrap_or_default(),
                        None,
                        false,
                    )
                    .await
                    .unwrap_or_else(|error| {
                        tracing::error!(%error, "failed to persist thread binding");
                        None
                    });
            }
        }
        let owned_ids = self.active_turns.lock().await.remove(user);
        let owned_ids = if owned_ids.is_empty() {
            ids.to_vec()
        } else {
            owned_ids
        };
        self.pending_approvals.lock().await.remove(user);
        let finalize: Result<()> = async {
            let output = output?;
            if let Some(t) = &output.session_id {
                thread = Some(t.clone());
            }
            let usage = output.token_usage_info.as_ref().and_then(|info| {
                super::format::build_usage_snapshot(info, output.context_window)
            });
            let warning = usage
                .as_ref()
                .and_then(|u| super::format::build_context_warning(u, &settings.language));
            self.session
                .bind_turn_result(
                    user,
                    &expected,
                    thread.clone(),
                    expected.profile.clone().unwrap_or_default(),
                    usage,
                    false,
                )
                .await?;
            let mut final_text = if let Some(warning) = warning {
                format!("{}\n\n{warning}", output.text)
            } else {
                output.text
            };
            let actual_dialog = self
                .state
                .dialog_id(user, thread.as_deref())
                .unwrap_or(dialog);
            self.state.set_loaded_versions(
                user,
                actual_dialog,
                profile_version,
                persona_version,
            )?;
            if settings.plan_mode {
                if let Some(plan) = super::format::extract_proposed_plan(&final_text) {
                    self.session
                        .update_settings_for_user(user, |s| s.pending_plan = Some(plan))
                        .await?;
                    final_text.push_str("\n\n");
                    final_text.push_str(&super::format::build_plan_followup_prompt(
                        &settings.language,
                    ));
                }
            }
            let parsed = parse_output(&final_text, &expected.workspace_dir);
            let directives = super::outgoing::prepare_directives(
                &self.session.user_root(user),
                &self.session.workspace_for(user),
                self.session.codex_home(),
                thread.as_deref(),
                parsed.directives,
                self.config.attachments.max_file_bytes,
            )?;
            let payload = Delivery {
                text: parsed.text,
                directives,
            };
            self.state.commit_answer(
                user,
                actual_dialog,
                &owned_ids,
                &message.message_id,
                &payload,
            )?;
            if let Err(error) = self.memory.completed_turn(user) {
                tracing::warn!(user=%user,%error,"failed to advance memory cursor after committed answer");
            } else {
                match self.memory.due_users(
                    self.config.memory.distill_after_turns,
                    self.config.memory.distill_idle_secs,
                ) {
                    Ok(users) if users.iter().any(|u| u == user) => self.request_distill(user),
                    Ok(_) => {}
                    Err(error) => tracing::warn!(user=%user,%error,"failed to evaluate memory distill trigger"),
                }
            }
            Ok(())
        }
        .await;
        if let Err(error) = finalize {
            tracing::warn!(user=%user,error=%error,"Codex turn failed or could not be committed");
            self.state.commit_turn_failure(
                user,
                &owned_ids,
                &message.message_id,
                &error.to_string(),
            )?;
        }
        Ok(())
    }
    pub(super) async fn compact(&self, user: &str, message: &str) -> Result<()> {
        let s = self.session.snapshot_for_user(user).await?;
        let thread = s
            .foreground
            .session_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no active conversation"))?;
        let settings = s.effective_settings();
        let req = CompactRequest {
            session_id: thread,
            workspace_dir: s.foreground.workspace_dir,
            config_overrides: Vec::new(),
            add_dirs: vec![
                self.session.workspace_for(user),
                self.session.inbox_for(user),
            ],
            model: settings.model_override,
            service_tier: settings.service_tier,
            context_mode: settings.context_mode,
            reasoning_effort: settings
                .reasoning_effort
                .unwrap_or(self.config.general.default_reasoning_effort),
        };
        tokio::time::timeout(
            Duration::from_secs(180),
            self.codex.compact_session(req, None),
        )
        .await??;
        self.reply_text(user, message, "conversation compacted")
            .await
    }
}
