use super::App;
use crate::{
    commands::{self, CommandOutcome},
    message::{IncomingAttachment, IncomingMessage, QuotedMessage},
    qq::{C2CMessageEvent, MSG_TYPE_QUOTE, MessageAttachment, MsgElement},
    state::inbox::InboxRow,
    work_queue::WorkKind,
};
use anyhow::Result;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
pub(super) fn spawn_inbox(app: Arc<App>) {
    tokio::spawn(async move {
        let running = Arc::new(tokio::sync::Mutex::new(HashSet::<String>::new()));
        // A user enters this set only when ordinary input arrives while that
        // user's turn is already running. Those pending rows are released as
        // one follow-up batch after the active turn finishes. There is no
        // idle-time debounce: an idle user's first message starts immediately.
        let mut deferred_users = HashSet::<String>::new();
        let mut controls = tokio::task::JoinSet::new();
        let mut control_users = HashSet::<String>::new();
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        loop {
            tick.tick().await;
            while let Some(done) = controls.try_join_next() {
                if let Ok(user) = done {
                    control_users.remove(&user);
                }
            }
            let rows = match app.state.pending_inbox(512) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error=%e,"inbox read failed");
                    continue;
                }
            };
            let mut normal = Vec::new();
            for row in rows {
                let event: C2CMessageEvent = match serde_json::from_str(&row.payload) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = app
                            .state
                            .finish_inbox(&row.user, &[row.id], Some(&e.to_string()));
                        continue;
                    }
                };
                if immediate(&event.content) {
                    if controls.len() >= 32 || control_users.contains(&row.user) {
                        continue;
                    }
                    let ids = vec![row.id.clone()];
                    if app.state.claim_inbox(&row.user, &ids).is_err() {
                        continue;
                    }
                    let user = row.user.clone();
                    control_users.insert(user.clone());
                    let app = app.clone();
                    controls.spawn(async move {
                        let result = app.handle_command_event(event).await;
                        let _ = app.state.finish_inbox(
                            &user,
                            &ids,
                            result.as_ref().err().map(|e| e.to_string()).as_deref(),
                        );
                        user
                    });
                } else {
                    normal.push(row)
                }
            }

            for mut batch in pending_by_user(normal) {
                let user = batch[0].user.clone();
                if control_users.contains(&user) {
                    continue;
                }
                let snapshot = match app.session.snapshot_for_user(&user).await {
                    Ok(snapshot) => snapshot,
                    Err(e) => {
                        tracing::warn!(error=%e,user=%user,"failed to read user session before dispatch");
                        continue;
                    }
                };

                // A picker response is control-plane input, not model input. Consume exactly
                // one pending row immediately; later rows stay durable and are reconsidered
                // after the picker clears.
                if snapshot.pending_setting.is_some() {
                    let row = &batch[0];
                    if let Ok(event) = serde_json::from_str::<C2CMessageEvent>(&row.payload) {
                        if event.attachments.is_empty()
                            && !event.content.trim_start().starts_with('/')
                        {
                            let ids = vec![row.id.clone()];
                            if app.state.claim_inbox(&user, &ids).is_ok() {
                                match app.handle_command_event(event).await {
                                    Ok(true) => {
                                        let _ = app.state.finish_inbox(&user, &ids, None);
                                        continue;
                                    }
                                    Ok(false) => {
                                        // The text was not a valid picker choice. commands::handle
                                        // clears the stale picker and returns Continue; put the row
                                        // back so the normal chat path can process it next tick.
                                        let _ = app.state.reset_inbox(&user, &ids);
                                        continue;
                                    }
                                    Err(e) => {
                                        let message = e.to_string();
                                        let _ = app.state.finish_inbox(
                                            &user,
                                            &ids,
                                            Some(message.as_str()),
                                        );
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                }

                // Ordinary input received while Codex is working is deliberately not
                // steered into the live turn. Keep it durable and merge all such rows
                // into the next turn once this one completes.
                if app.active_turns.lock().await.for_user(&user).is_some() {
                    deferred_users.insert(user.clone());
                    continue;
                }

                {
                    let mut in_flight = running.lock().await;
                    if in_flight.contains(&user) {
                        // A worker exists but there is no active Codex turn yet (or it is
                        // already finalizing). Do not classify these rows as reply-time
                        // follow-ups; only ActiveTurns does that.
                        continue;
                    }
                    if in_flight.len() >= app.config.runtime.max_concurrent_codex {
                        continue;
                    }
                    in_flight.insert(user.clone());
                }

                // No artificial idle batching. If this input was not accumulated behind
                // an active turn, dispatch only the oldest row now. Any immediately
                // following rows will observe the newly running turn and become the
                // merged follow-up batch instead.
                if !deferred_users.remove(&user) {
                    batch.truncate(1);
                }

                let ids = batch.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
                if let Err(e) = app.state.claim_inbox(&user, &ids) {
                    tracing::warn!(error=%e,"inbox claim failed");
                    running.lock().await.remove(&user);
                    continue;
                }
                let app = app.clone();
                let running = running.clone();
                tokio::spawn(async move {
                    let result = app.handle_batch(batch).await;
                    if let Err(e) = result {
                        tracing::warn!(error=%e,user=%user,"inbound batch failed");
                        let _ = app
                            .state
                            .commit_turn_failure(&user, &ids, &ids[0], &e.to_string());
                    }
                    running.lock().await.remove(&user);
                });
            }
        }
    });
}

fn pending_by_user(rows: Vec<InboxRow>) -> Vec<Vec<InboxRow>> {
    let mut by_user = std::collections::BTreeMap::<String, Vec<InboxRow>>::new();
    for row in rows {
        by_user.entry(row.user.clone()).or_default().push(row);
    }
    let mut batches = by_user.into_values().collect::<Vec<_>>();
    batches.sort_by_key(|batch| batch[0].received_at);
    batches
}

fn immediate(text: &str) -> bool {
    matches!(
        commands::canonicalize(text.split_whitespace().next().unwrap_or_default()),
        "/stop"
            | "/停止"
            | "/approve"
            | "/同意"
            | "/approve-session"
            | "/同意本会话"
            | "/deny"
            | "/拒绝"
            | "/cancel"
            | "/取消"
            | "/status"
            | "/状态"
            | "/interrupt"
            | "/new"
            | "/新建"
            | "/bg"
            | "/后台"
            | "/fg"
            | "/前台"
            | "/resume"
            | "/恢复"
            | "/rename"
            | "/save"
            | "/sessions"
            | "/会话"
            | "/model"
            | "/模型"
            | "/reasoning"
            | "/思考"
            | "/fast"
            | "/context"
            | "/lang"
            | "/语言"
            | "/verbose"
            | "/plan"
            | "/approvals"
            | "/help"
            | "/帮助"
            | "/cron"
            | "/back"
            | "/memory"
            | "/retry"
    )
}
impl App {
    async fn handle_command_event(self: &Arc<Self>, event: C2CMessageEvent) -> Result<bool> {
        let message = IncomingMessage {
            sender_openid: event.author.user_openid,
            message_id: event.id,
            text: event.content.trim().to_owned(),
            quote: None,
            images: Vec::new(),
            files: Vec::new(),
            mentions: Vec::new(),
        };
        match self.dispatch_command(&message).await {
            Ok(handled) => Ok(handled),
            Err(e) => {
                self.reply_text(&message.sender_openid, &message.message_id, &e.to_string())
                    .await?;
                Ok(true)
            }
        }
    }
    async fn dispatch_command(self: &Arc<Self>, message: &IncomingMessage) -> Result<bool> {
        let user = &message.sender_openid;
        let id = &message.message_id;
        let first =
            commands::canonicalize(message.text.split_whitespace().next().unwrap_or_default());
        if first == "/cron" {
            let args = shlex::split(
                message
                    .text
                    .split_once(char::is_whitespace)
                    .map(|(_, r)| r)
                    .unwrap_or("")
                    .trim(),
            )
            .ok_or_else(|| anyhow::anyhow!("invalid command quoting"))?;
            let args: Vec<_> = args.iter().map(String::as_str).collect();
            let text = crate::scheduler::cli::command(&self.state, &self.config, user, &args)?;
            self.reply_text(user, id, &text).await?;
            return Ok(true);
        }
        if first == "/memory" {
            let args = message
                .text
                .split_once(char::is_whitespace)
                .map(|(_, r)| r)
                .unwrap_or("")
                .trim();
            let (action, rest) = args.split_once(' ').unwrap_or((args, ""));
            let value = match action {
                "search" => serde_json::to_value(self.memory.search(user, rest, None, 8)?)?,
                "get" => serde_json::to_value(self.memory.get(user, rest.parse()?)?)?,
                "delete" => {
                    let a = serde_json::json!({"id":rest.parse::<i64>()?});
                    crate::memory::tools::call(&self.memory, user, "memory_delete", &a)?
                }
                "add" => crate::memory::tools::call(
                    &self.memory,
                    user,
                    "memory_append",
                    &serde_json::json!({"content":rest}),
                )?,
                "update" => {
                    let (i, text) = rest
                        .split_once(' ')
                        .ok_or_else(|| anyhow::anyhow!("usage: /memory update <id> <text>"))?;
                    crate::memory::tools::call(
                        &self.memory,
                        user,
                        "memory_update",
                        &serde_json::json!({"id":i.parse::<i64>()?,"content":text}),
                    )?
                }
                _ => anyhow::bail!("usage: /memory search|get|add|update|delete"),
            };
            self.reply_text(user, id, &serde_json::to_string_pretty(&value)?)
                .await?;
            return Ok(true);
        }
        let busy = self.active_turns.lock().await.for_user(user).is_some();
        let pending = self.session.snapshot_for_user(user).await?.pending_setting;
        let models = if matches!(first, "/model" | "/模型")
            || (!message.text.trim_start().starts_with('/')
                && matches!(pending, Some(crate::model::settings::PendingSetting::Model)))
        {
            self.codex.handle().models().await?
        } else {
            Vec::new()
        };
        match commands::handle(&message.text, user, &self.session, busy, &models).await? {
            CommandOutcome::Continue => return Ok(false),
            CommandOutcome::Reply(mut text) => {
                if first == "/status" {
                    if let Some(active) = self.active_turns.lock().await.for_user(user) {
                        text.push_str(&format!(
                            "\nturn: {}\nelapsed: {}s",
                            active.turn_id,
                            (chrono::Utc::now() - active.started_at).num_seconds()
                        ));
                    }
                }
                self.reply_text(user, id, &text).await?;
            }
            CommandOutcome::Stop => {
                self.active_turns.lock().await.cancel(user);
                let stopped = self.session.stop_foreground(user).await?;
                tracing::debug!(had_session=stopped.had_session,saved=stopped.saved,dropped=stopped.dropped_unsaved,restored=?stopped.restored_alias,"foreground stopped");
                self.reply_text(user, id, "stopped").await?;
            }
            CommandOutcome::Cancel => {
                self.active_turns.lock().await.cancel(user);
                self.reply_text(user, id, "interrupted").await?;
            }
            CommandOutcome::New => {
                self.request_distill(user);
                self.session.new_foreground(user, busy).await?;
                self.reply_text(user, id, "new conversation").await?;
            }
            CommandOutcome::Approval(intent) => {
                let done = self.resolve_pending_approval(user, intent).await;
                self.reply_text(
                    user,
                    id,
                    if done {
                        "approval resolved"
                    } else {
                        "no pending approval"
                    },
                )
                .await?;
            }
            CommandOutcome::Compact => {
                self.request_distill(user);
                self.compact(user, id).await?;
            }
            CommandOutcome::RetryResume => {
                anyhow::ensure!(!busy, "A turn is active; interrupt it before retrying.");
                let queued = self.state.retry_turn(user)?;
                self.reply_text(
                    user,
                    id,
                    if queued {
                        "Retry queued."
                    } else {
                        "No failed request to retry."
                    },
                )
                .await?;
            }
        }
        Ok(true)
    }
    async fn handle_batch(self: &Arc<Self>, rows: Vec<InboxRow>) -> Result<()> {
        let user = &rows[0].user;
        let ids: Vec<_> = rows.iter().map(|r| r.id.clone()).collect();
        let _permit = self
            .work_queue
            .acquire(user, WorkKind::UserTurn, None)
            .await
            .ok_or_else(|| anyhow::anyhow!("work queue closed"))?;
        let mut messages = Vec::new();
        for row in &rows {
            messages.push(
                self.normalize_message(serde_json::from_str(&row.payload)?)
                    .await?,
            );
        }
        if messages.len() == 1 && self.dispatch_command(&messages[0]).await? {
            self.state.finish_inbox(user, &ids, None)?;
            return Ok(());
        }
        let mut combined = messages[0].clone();
        if matches!(
            commands::canonicalize(combined.text.trim()),
            "/execute-plan"
        ) {
            let s = self.session.snapshot_for_user(user).await?;
            if let Some(plan) = s.settings.pending_plan {
                combined.text = format!("Execute this approved plan:\n{plan}");
                self.session
                    .update_settings_for_user(user, |s| {
                        s.pending_plan = None;
                        s.plan_mode = false;
                    })
                    .await?;
            }
        }
        for m in messages.iter().skip(1) {
            combined.text.push('\n');
            combined.text.push_str(&m.text);
            combined.images.extend(m.images.clone());
            combined.files.extend(m.files.clone());
        }
        self.run_turn(combined, &ids, &messages).await
    }
    async fn normalize_message(&self, event: C2CMessageEvent) -> Result<IncomingMessage> {
        let mut images = Vec::new();
        let mut files = Vec::new();
        let user = &event.author.user_openid;
        for a in &event.attachments {
            let local = self.download_attachment(user, &event.id, a).await?;
            let item = IncomingAttachment {
                filename: a.filename.clone(),
                content_type: Some(a.content_type.clone()),
                source_url: a.url.clone(),
                local_path: local,
            };
            if a.content_type.starts_with("image/") {
                images.push(item)
            } else {
                files.push(item)
            }
        }
        Ok(IncomingMessage {
            sender_openid: event.author.user_openid,
            message_id: event.id,
            text: event.content.trim().into(),
            quote: extract_quote(event.message_type, &event.msg_elements),
            images,
            files,
            mentions: Vec::new(),
        })
    }
    async fn download_attachment(
        &self,
        user: &str,
        message: &str,
        a: &MessageAttachment,
    ) -> Result<PathBuf> {
        let dir = self.session.inbox_for(user);
        std::fs::create_dir_all(&dir)?;
        let filename = a
            .filename
            .as_deref()
            .unwrap_or("attachment.bin")
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("attachment.bin");
        let filename = if matches!(filename, "" | "." | "..") {
            "attachment.bin"
        } else {
            filename
        };
        let destination = dir.join(format!(
            "{:x}_{:x}_{filename}",
            md5::compute(message),
            md5::compute(&a.url)
        ));
        if destination.exists() {
            anyhow::ensure!(
                std::fs::canonicalize(&destination)?.starts_with(std::fs::canonicalize(&dir)?),
                "attachment path escapes inbox"
            );
            return Ok(destination);
        }
        let used = std::fs::read_dir(&dir)?
            .filter_map(Result::ok)
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum::<u64>();
        let max = self.config.attachments.max_file_bytes.min(
            self.config
                .attachments
                .per_user_quota_bytes
                .saturating_sub(used),
        );
        anyhow::ensure!(max > 0, "attachment quota exceeded");
        self.qq_client
            .download_attachment_limited(&a.url, &destination, max)
            .await?;
        Ok(destination)
    }
}
fn extract_quote(kind: Option<u32>, elements: &[MsgElement]) -> Option<QuotedMessage> {
    if kind != Some(MSG_TYPE_QUOTE) {
        return None;
    }
    let text = elements
        .iter()
        .filter_map(|e| e.content.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(QuotedMessage {
        message_id: elements.first().and_then(|e| e.msg_idx.clone()),
        text,
    })
}
pub(super) fn spawn_outbox(app: Arc<App>) {
    tokio::spawn(async move {
        loop {
            match app.state.next_delivery() {
                Ok(Some(row)) => {
                    // Validate EVERY attachment before sending text. An invalid
                    // generated-image path must never cause text-only spam.
                    let result: Result<()> = async {
                        for directive in &row.payload.directives {
                            super::outgoing::validate_directive(
                                &app.session.user_root(&row.user),
                                directive,
                                app.config.attachments.max_file_bytes,
                            )?;
                        }
                        if !row.payload.directives.is_empty() && row.reply_to.is_none() {
                            anyhow::bail!("proactive file delivery requires a recent user message");
                        }
                        if !row.payload.text.is_empty() && !row.text_sent {
                            if let Some(reply) = &row.reply_to {
                                app.qq_client.send_markdown(
                                    &row.user, reply, &row.payload.text
                                ).await?;
                            } else {
                                app.qq_client.send_markdown_proactive(
                                    &row.user, &row.payload.text
                                ).await?;
                            }
                            app.state.mark_delivery_text_sent(&row.user, row.id)?;
                        }
                        for (index, directive) in row.payload.directives.iter().enumerate().skip(row.directives_sent) {
                            let reply = row.reply_to.as_deref().ok_or_else(|| {
                                anyhow::anyhow!("proactive file delivery requires a recent user message")
                            })?;
                            app.send_directive(&row.user, reply, directive.clone()).await?;
                            app.state.mark_delivery_directive_sent(&row.user, row.id, index + 1)?;
                        }
                        Ok(())
                    }.await;
                    let permanent = result.as_ref().err()
                        .is_some_and(crate::qq::is_permanent_delivery_error);
                    if let Err(err) = &result {
                        tracing::warn!(error=%err, permanent, "QQ outbox delivery failed");
                    }
                    if let Err(err) = app.state.finish_delivery(
                        &row.user,
                        row.id,
                        result.as_ref().err().map(|e| e.to_string()).as_deref(),
                        permanent,
                    ) {
                        tracing::error!(error=%err, "failed to persist QQ outbox result");
                    }
                }
                Ok(None) => tokio::time::sleep(Duration::from_secs(1)).await,
                Err(e) => {
                    tracing::warn!(error=%e,"outbox read failed");
                    tokio::time::sleep(Duration::from_secs(5)).await
                }
            }
        }
    });
}

pub(super) fn spawn_attachment_cleanup(app: Arc<App>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let roots = app
                .config
                .general
                .data_dir
                .parent()
                .unwrap_or(&app.config.general.data_dir)
                .join("users");
            let Ok(users) = std::fs::read_dir(roots) else {
                continue;
            };
            let cutoff = std::time::SystemTime::now()
                - Duration::from_secs(app.config.attachments.retention_hours as u64 * 3600);
            let protected = match app.state.retained_attachment_prefixes() {
                Ok(v) => v,
                Err(error) => {
                    tracing::warn!(%error,"attachment cleanup skipped");
                    continue;
                }
            };
            for user in users.flatten() {
                let name = user.file_name().to_string_lossy().into_owned();
                let active = app
                    .active_turns
                    .lock()
                    .await
                    .user_ids()
                    .iter()
                    .any(|u| format!("{:x}", md5::compute(u)) == name);
                if active {
                    continue;
                }
                let inbox = user.path().join("inbox");
                let Ok(files) = std::fs::read_dir(inbox) else {
                    continue;
                };
                for file in files.flatten() {
                    let filename = file.file_name().to_string_lossy().into_owned();
                    if protected
                        .iter()
                        .any(|(owner, prefix)| owner == &name && filename.starts_with(prefix))
                    {
                        continue;
                    }
                    if file.file_type().is_ok_and(|t| t.is_file())
                        && file
                            .metadata()
                            .and_then(|m| m.modified())
                            .is_ok_and(|t| t < cutoff)
                    {
                        let _ = std::fs::remove_file(file.path());
                    }
                }
            }
        }
    });
}
