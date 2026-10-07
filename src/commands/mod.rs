use crate::{
    codex::CodexModelEntry,
    model::settings::{
        ApprovalPolicySetting, ContextMode, PendingSetting, ReasoningEffort, ServiceTier,
    },
    session::SessionStore,
};
use anyhow::{Result, anyhow};
pub(crate) enum CommandOutcome {
    Reply(String),
    Continue,
    Stop,
    Cancel,
    Compact,
    New,
    Approval(ApprovalIntent),
    RetryResume,
}
#[derive(Clone, Copy)]
pub(crate) enum ApprovalIntent {
    Accept,
    AcceptForSession,
    Decline,
    Cancel,
}
pub(crate) async fn handle(
    text: &str,
    user: &str,
    session: &SessionStore,
    busy: bool,
    models: &[CodexModelEntry],
) -> Result<CommandOutcome> {
    let snapshot = session.snapshot_for_user(user).await?;
    let explicit_command = text.trim_start().starts_with('/');
    if explicit_command && snapshot.pending_setting.is_some() {
        // A fresh slash command always cancels an older numbered picker.  Picker
        // state is UI state, not conversation intent, and must never leak into
        // a later command or ordinary chat message.
        session.set_pending_setting(user, None).await?;
    }
    let expanded = if !explicit_command {
        if let Some(pending) = &snapshot.pending_setting {
            let command = pending.command_name("en");
            let raw = text.trim();
            let options = match pending {
                PendingSetting::Reasoning => vec!["low", "medium", "high", "xhigh", "max", "inherit"],
                PendingSetting::Fast => vec!["on", "off", "inherit"],
                PendingSetting::Context => vec!["standard", "1m", "inherit"],
                PendingSetting::Verbose | PendingSetting::Plan => vec!["on", "off"],
                PendingSetting::Lang => vec!["zh", "en"],
                PendingSetting::Approvals => vec![
                    "untrusted",
                    "on-request",
                    "never",
                    "guardian-subagent",
                    "inherit",
                ],
                _ => Vec::new(),
            };
            let selected = if let Ok(n) = raw.parse::<usize>() {
                if matches!(pending, PendingSetting::Model) {
                    models
                        .get(n.saturating_sub(1))
                        .map(|m| m.name.clone())
                } else if matches!(
                    pending,
                    PendingSetting::Fg
                        | PendingSetting::ResumeProjects
                        | PendingSetting::SessionsProjects
                ) {
                    snapshot
                        .last_sessions_view
                        .get(n.saturating_sub(1))
                        .cloned()
                } else {
                    options.get(n.saturating_sub(1)).map(|v| (*v).to_owned())
                }
            } else if matches!(pending, PendingSetting::Model) {
                (matches!(raw, "inherit" | "default")
                    || models.iter().any(|m| m.name == raw))
                .then(|| raw.to_owned())
            } else if options.contains(&raw) {
                Some(raw.to_owned())
            } else {
                None
            };
            session.set_pending_setting(user, None).await?;
            if let Some(value) = selected {
                format!("{command} {value}")
            } else {
                // An unrelated ordinary message is conversation, not a malformed
                // picker response.  Cancel the stale picker and let the message
                // continue to the model unchanged.
                text.to_owned()
            }
        } else {
            text.to_owned()
        }
    } else {
        text.to_owned()
    };
    let text = expanded.as_str();
    let trimmed = text.trim();
    let Some(first) = trimmed.split_whitespace().next() else {
        return Ok(CommandOutcome::Continue);
    };
    let cmd = canonicalize(first);
    let rest = trimmed[first.len()..].trim();
    if !cmd.starts_with('/') {
        return Ok(CommandOutcome::Continue);
    }
    let reply = |s: String| {
        Ok(CommandOutcome::Reply(if s.is_empty() {
            if snapshot.settings.language == "zh" {
                "没有可用会话或选项。".into()
            } else {
                "No available conversations or options.".into()
            }
        } else {
            s
        }))
    };
    if cmd == "/back" {
        session.set_pending_setting(user, None).await?;
        return reply("cancelled".into());
    }
    if rest.is_empty() {
        let (pending, choices) = match cmd {
            "/model" => (
                Some(PendingSetting::Model),
                models
                    .iter()
                    .enumerate()
                    .map(|(i, m)| {
                        format!(
                            "{}. {} {}",
                            i + 1,
                            m.name,
                            m.description.as_deref().unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            "/reasoning" => (
                Some(PendingSetting::Reasoning),
                "1. low\n2. medium\n3. high\n4. xhigh\n5. max\n6. inherit".into(),
            ),
            "/fast" => (
                Some(PendingSetting::Fast),
                "1. on\n2. off\n3. inherit".into(),
            ),
            "/context" => (
                Some(PendingSetting::Context),
                "1. standard\n2. 1m\n3. inherit".into(),
            ),
            "/verbose" => (Some(PendingSetting::Verbose), "1. on\n2. off".into()),
            "/plan" => (Some(PendingSetting::Plan), "1. on\n2. off".into()),
            "/lang" => (Some(PendingSetting::Lang), "1. zh\n2. en".into()),
            "/approvals" => (
                Some(PendingSetting::Approvals),
                "1. untrusted\n2. on-request\n3. never\n4. guardian-subagent\n5. inherit".into(),
            ),
            _ => (None, String::new()),
        };
        if let Some(pending) = pending {
            session.set_pending_setting(user, Some(pending)).await?;
            return reply(choices);
        }
    }
    match cmd {
        "/approve" => Ok(CommandOutcome::Approval(ApprovalIntent::Accept)),
        "/approve-session" => Ok(CommandOutcome::Approval(ApprovalIntent::AcceptForSession)),
        "/deny" => Ok(CommandOutcome::Approval(ApprovalIntent::Decline)),
        "/cancel" => Ok(CommandOutcome::Approval(ApprovalIntent::Cancel)),
        "/interrupt" => Ok(CommandOutcome::Cancel),
        "/stop" => Ok(CommandOutcome::Stop),
        "/new" => Ok(CommandOutcome::New),
        "/compact" => {
            anyhow::ensure!(!busy, "A turn is active; interrupt it before compacting.");
            Ok(CommandOutcome::Compact)
        },
        "/retry" => Ok(CommandOutcome::RetryResume),
        "/help" if rest == "all" => reply(if snapshot.settings.language == "zh" {
            include_str!("../../docs/commands.md").to_owned()
        } else {
            include_str!("../../docs/commands_en.md").to_owned()
        }),
        "/help" => reply(
            rust_i18n::t!(
                "commands.help.compact",
                locale = snapshot.settings.language.as_str()
            )
            .into_owned(),
        ),
        "/status" => {
            let s = session.snapshot_for_user(user).await?;
            reply(format!(
                "running: {busy}\nthread: {}\nbackground: {}\nsettings: {}",
                s.foreground.session_id.as_deref().unwrap_or("new"),
                s.background_order.join(", "),
                format!(
                    "{}; fast={}",
                    serde_json::to_string(&s.effective_settings())?,
                    ServiceTier::fast_label(s.effective_settings().service_tier)
                )
            ))
        }
        "/sessions" => {
            let dialogs = session.db.owned_dialogs(user)?;
            let ids = dialogs.iter().map(|d| d.0.clone()).collect();
            session.remember_sessions_view(user, ids).await?;
            reply(
                dialogs
                    .iter()
                    .enumerate()
                    .map(|(i, (id, alias, state))| {
                        format!(
                            "{}. {} {} ({state})",
                            i + 1,
                            alias.as_deref().unwrap_or(""),
                            id
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        }
        "/bg" => {
            let r = session
                .move_foreground_to_background(user, (!rest.is_empty()).then_some(rest), busy)
                .await?;
            reply(format!(
                "background: {}",
                r.parked_alias
                    .or(r.reserved_alias)
                    .unwrap_or_else(|| "empty".into())
            ))
        }
        "/resume" => {
            if rest.is_empty() {
                let dialogs = session.db.owned_dialogs(user)?;
                session
                    .remember_sessions_view(user, dialogs.iter().map(|d| d.0.clone()).collect())
                    .await?;
                session
                    .set_pending_setting(user, Some(PendingSetting::ResumeProjects))
                    .await?;
                return reply(
                    dialogs
                        .iter()
                        .enumerate()
                        .map(|(i, d)| {
                            format!("{}. {} {}", i + 1, d.0, d.1.as_deref().unwrap_or(""))
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            let s = session.snapshot_for_user(user).await?;
            let target = rest
                .parse::<usize>()
                .ok()
                .and_then(|n| s.last_sessions_view.get(n.saturating_sub(1)))
                .map(String::as_str)
                .unwrap_or(rest);
            if s.background.contains_key(target) {
                session.foreground_from_background(user, target).await?;
            } else {
                session.resume_owned(user, target).await?;
            }
            reply("conversation resumed".into())
        }
        "/fg" => {
            let s = session.snapshot_for_user(user).await?;
            if rest.is_empty() {
                session
                    .remember_sessions_view(user, s.background_order.clone())
                    .await?;
                session
                    .set_pending_setting(user, Some(PendingSetting::Fg))
                    .await?;
                return reply(
                    s.background_order
                        .iter()
                        .enumerate()
                        .map(|(i, a)| format!("{}. {a}", i + 1))
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            let alias = rest
                .parse::<usize>()
                .ok()
                .and_then(|n| s.last_sessions_view.get(n.saturating_sub(1)))
                .map(String::as_str)
                .unwrap_or(rest);
            session.foreground_from_background(user, alias).await?;
            reply(format!("foreground: {alias}"))
        }
        "/save" => {
            session.save_foreground(user).await?;
            reply("saved".into())
        }
        "/rename" => {
            let parts: Vec<_> = rest.split_whitespace().collect();
            anyhow::ensure!(parts.len() == 2, "usage: /rename <old> <new>");
            session
                .rename_background_alias(user, parts[0], parts[1])
                .await?;
            reply("renamed".into())
        }
        "/model" => {
            if rest.is_empty() {
                return reply(
                    models
                        .iter()
                        .map(|m| {
                            format!(
                                "{} {}",
                                m.name,
                                m.description.as_deref().unwrap_or_default()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            let value = (!matches!(rest, "inherit" | "default")).then(|| rest.to_owned());
            let other = value.clone();
            session
                .set_active_profile(
                    user,
                    |p| p.model_override = value,
                    |s| s.model_override = other,
                )
                .await?;
            reply("model updated".into())
        }
        "/reasoning" => {
            let value = if matches!(rest, "inherit" | "default") {
                None
            } else {
                Some(
                    ReasoningEffort::parse_supported(rest)
                        .ok_or_else(|| anyhow!("use low, medium, high, xhigh, max or inherit"))?,
                )
            };
            session
                .set_active_profile(
                    user,
                    |p| p.reasoning_effort = value,
                    |s| s.reasoning_effort = value,
                )
                .await?;
            reply("reasoning updated".into())
        }
        "/fast" => {
            let value = if matches!(rest, "inherit" | "default") {
                None
            } else {
                Some(ServiceTier::parse(rest).ok_or_else(|| anyhow!("use on, off or inherit"))?)
            };
            session
                .set_active_profile(user, |p| p.service_tier = value, |s| s.service_tier = value)
                .await?;
            reply("service tier updated".into())
        }
        "/context" => {
            let value = if matches!(rest, "inherit" | "default") {
                None
            } else {
                Some(
                    ContextMode::parse(rest)
                        .ok_or_else(|| anyhow!("use standard, 1m or inherit"))?,
                )
            };
            session
                .set_active_profile(user, |p| p.context_mode = value, |s| s.context_mode = value)
                .await?;
            reply("context updated".into())
        }
        "/lang" => {
            anyhow::ensure!(crate::util::lang::is_supported_lang(rest), "use zh or en");
            session
                .update_settings_for_user(user, |s| {
                    s.language = crate::util::lang::normalize_lang(rest).into()
                })
                .await?;
            reply("language updated".into())
        }
        "/verbose" | "/plan" => {
            anyhow::ensure!(matches!(rest, "on" | "off"), "use on or off");
            session
                .update_settings_for_user(user, |s| {
                    if cmd == "/plan" {
                        s.plan_mode = rest == "on"
                    } else {
                        s.verbose = rest == "on"
                    }
                })
                .await?;
            reply("updated".into())
        }
        "/execute-plan" => {
            session
                .update_settings_for_user(user, |s| s.plan_mode = false)
                .await?;
            Ok(CommandOutcome::Continue)
        }
        "/keep-planning" => {
            session
                .update_settings_for_user(user, |s| s.plan_mode = true)
                .await?;
            reply("planning enabled".into())
        }
        "/cancel-plan" => {
            session
                .update_settings_for_user(user, |s| {
                    s.plan_mode = false;
                    s.pending_plan = None;
                })
                .await?;
            reply("plan cancelled".into())
        }
        "/approvals" => {
            let value = if matches!(rest, "inherit" | "default") {
                None
            } else {
                Some(ApprovalPolicySetting::parse(rest).ok_or_else(|| {
                    anyhow!("use inherit, never, on-request, untrusted or guardian-subagent")
                })?)
            };
            session
                .update_settings_for_user(user, |s| s.approval_policy_override = value)
                .await?;
            let label = value
                .map(|p| {
                    if snapshot.settings.language == "zh" {
                        p.label_zh()
                    } else {
                        p.label_en()
                    }
                })
                .unwrap_or("inherit");
            reply(format!("approval policy: {label}"))
        }

        _ => Ok(CommandOutcome::Continue),
    }
}

pub(crate) fn canonicalize(command: &str) -> &str {
    match command {
        "/帮助" => "/help",
        "/状态" => "/status",
        "/会话" => "/sessions",
        "/模型" => "/model",
        "/快速" => "/fast",
        "/上下文" => "/context",
        "/思考" => "/reasoning",
        "/语言" => "/lang",
        "/详细" => "/verbose",
        "/恢复" => "/resume",
        "/后台" => "/bg",
        "/前台" => "/fg",
        "/保存" => "/save",
        "/新建" => "/new",
        "/停止" => "/stop",
        "/中断" => "/interrupt",
        "/压缩" => "/compact",
        "/重命名" => "/rename",
        "/返回" => "/back",
        "/审批" => "/approvals",
        "/定时" => "/cron",
        "/计划" => "/plan",
        "/同意" => "/approve",
        "/同意本会话" => "/approve-session",
        "/拒绝" => "/deny",
        "/取消" => "/cancel",
        "/实施" => "/execute-plan",
        "/继续规划" => "/keep-planning",
        "/取消计划" => "/cancel-plan",
        "/重试" => "/retry",
        other => other,
    }
}
