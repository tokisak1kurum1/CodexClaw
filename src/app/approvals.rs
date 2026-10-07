//! Server-initiated approval requests: routing them to the active turn's QQ
//! user, queueing the pending decisions, and formatting the prompts.

use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use crate::{
    codex::{
        ApprovalOutcome, ApprovalRequest, CommandApprovalEvent, FileChangeApprovalEvent,
        PermissionsApprovalEvent,
    },
    commands::ApprovalIntent,
};

use super::{App, PendingApprovalEntry};

impl App {
    /// Wire the approval broker to forward requests into our QQ prompt +
    /// pending-approval queue.
    pub(super) fn install_approval_handler(self: Arc<Self>) {
        let (tx, mut rx) = mpsc::channel::<ApprovalRequest>(32);
        let broker = self.codex.handle().approvals.clone();
        tokio::spawn(async move {
            broker.install_handler(tx).await;
        });
        let app_for_loop = self.clone();
        tokio::spawn(async move {
            while let Some(request) = rx.recv().await {
                app_for_loop.clone().route_approval_request(request).await;
            }
        });
    }

    async fn route_approval_request(self: Arc<Self>, request: ApprovalRequest) {
        let thread_id = match &request {
            ApprovalRequest::Command { event, .. } => &event.thread_id,
            ApprovalRequest::FileChange { event, .. } => &event.thread_id,
            ApprovalRequest::Permissions { event, .. } => &event.thread_id,
            ApprovalRequest::Elicitation { event, .. } => &event.thread_id,
            ApprovalRequest::Tool { thread_id, .. } => thread_id,
        };
        let owner = self
            .codex
            .handle()
            .supervisor
            .thread_owners
            .lock()
            .await
            .get(thread_id)
            .cloned();
        let ctx = {
            let mut turns = self.active_turns.lock().await;
            if let Some(owner) = owner {
                if let Some(active) = turns.for_user(&owner) {
                    if active.thread_id.is_empty() {
                        turns.bind(&owner, thread_id, None);
                    }
                }
            }
            turns.for_thread(thread_id)
        };
        let Some(ctx) = ctx else {
            decline_approval_request(request);
            return;
        };
        let request_turn = match &request {
            ApprovalRequest::Tool { turn_id, .. } => Some(turn_id.as_str()),
            ApprovalRequest::Command { event, .. } => event.turn_id.as_deref(),
            ApprovalRequest::FileChange { event, .. } => event.turn_id.as_deref(),
            ApprovalRequest::Permissions { event, .. } => event.turn_id.as_deref(),
            ApprovalRequest::Elicitation { event, .. } => event.turn_id.as_deref(),
        };
        if request_turn.is_some_and(|turn| !ctx.turn_id.is_empty() && ctx.turn_id != turn) {
            decline_approval_request(request);
            return;
        }
        let openid = ctx.user_id.clone();
        let reply_id = ctx.message_id.clone();
        match request {
            ApprovalRequest::Tool {
                tool,
                arguments,
                reply,
                ..
            } => {
                let result = if tool.starts_with("schedule_") {
                    crate::scheduler::cli::call(
                        &self.state,
                        &self.config,
                        &openid,
                        &tool,
                        &arguments,
                    )
                } else {
                    crate::memory::tools::call(&self.memory, &openid, &tool, &arguments)
                };
                let (success, text) = match result {
                    Ok(v) => (true, v.to_string()),
                    Err(e) => (false, e.to_string()),
                };
                let _ = reply.send(serde_json::json!({"success":success,"contentItems":[{"type":"inputText","text":text}]}));
            }

            ApprovalRequest::Command { event, reply } => {
                let prompt = format_command_approval(&event);
                self.enqueue_outcome(openid.clone(), reply_id, prompt, reply)
                    .await;
            }
            ApprovalRequest::FileChange { event, reply } => {
                let root = self
                    .config
                    .general
                    .data_dir
                    .parent()
                    .unwrap_or(&self.config.general.data_dir);
                if changes_protected_persona_file(&event.file_changes, root) {
                    tracing::warn!(user=%openid, thread_id=%event.thread_id, "blocked agent write to protected persona file");
                    let _ = reply.send(ApprovalOutcome::Decline);
                    let _ = self
                        .reply_text(
                            &openid,
                            &reply_id,
                            "BEHAVIOR.md / IDENTITY.md / CHARACTER.md are protected and cannot be changed by an agent file write.",
                        )
                        .await;
                } else {
                    let prompt = format_file_change_approval(&event);
                    self.enqueue_outcome(openid.clone(), reply_id, prompt, reply)
                        .await;
                }
            }
            ApprovalRequest::Permissions { event, reply } => {
                let prompt = format_permissions_approval(&event);
                self.enqueue_outcome(openid.clone(), reply_id, prompt, reply)
                    .await;
            }
            ApprovalRequest::Elicitation { event, reply } => {
                // MCP elicitations are free-form — out of scope for MVP.
                warn!(
                    thread_id = %event.thread_id,
                    server = event.server.as_deref().unwrap_or(""),
                    "MCP elicitation received; auto-declining (not yet wired to QQ)"
                );
                let _ = reply.send(None);
            }
        }
    }

    async fn enqueue_outcome(
        self: Arc<Self>,
        openid: String,
        reply_message_id: String,
        prompt: String,
        tx: oneshot::Sender<ApprovalOutcome>,
    ) {
        let mut guard = self.pending_approvals.lock().await;
        let slot = guard.entry(openid.clone()).or_default();
        slot.push_back(PendingApprovalEntry::Outcome(tx));
        drop(guard);
        if let Err(err) = self.reply_text(&openid, &reply_message_id, &prompt).await {
            warn!(error = %err, openid = %openid, "failed to deliver approval prompt to QQ");
        }
    }

    /// Resolve the oldest pending approval for `openid` with `intent`.
    /// Returns `true` if a pending approval was resolved; `false` if there
    /// was none (caller should tell the user).
    pub(super) async fn resolve_pending_approval(
        &self,
        openid: &str,
        intent: ApprovalIntent,
    ) -> bool {
        let mut guard = self.pending_approvals.lock().await;
        let Some(queue) = guard.get_mut(openid) else {
            return false;
        };
        let Some(entry) = queue.pop_front() else {
            return false;
        };
        let PendingApprovalEntry::Outcome(tx) = entry;
        let outcome = match intent {
            ApprovalIntent::Accept => ApprovalOutcome::Accept,
            ApprovalIntent::AcceptForSession => ApprovalOutcome::AcceptForSession,
            ApprovalIntent::Decline => ApprovalOutcome::Decline,
            ApprovalIntent::Cancel => ApprovalOutcome::Cancel,
        };
        let _ = tx.send(outcome);
        true
    }
}

fn decline_approval_request(request: ApprovalRequest) {
    match request {
        ApprovalRequest::Tool { reply, .. } => {
            let _ = reply.send(serde_json::json!({"success":false,"contentItems":[{"type":"inputText","text":"no active owned turn"}]}));
        }
        ApprovalRequest::Command { reply, .. } => {
            let _ = reply.send(ApprovalOutcome::Decline);
        }
        ApprovalRequest::FileChange { reply, .. } => {
            let _ = reply.send(ApprovalOutcome::Decline);
        }
        ApprovalRequest::Permissions { reply, .. } => {
            let _ = reply.send(ApprovalOutcome::Decline);
        }
        ApprovalRequest::Elicitation { reply, .. } => {
            let _ = reply.send(None);
        }
    }
}

fn changes_protected_persona_file(payload: &serde_json::Value, root: &std::path::Path) -> bool {
    fn matches(path: &str, root: &std::path::Path) -> bool {
        let candidate = std::path::Path::new(path);
        if !candidate.is_absolute() {
            return false;
        }
        candidate == root.join("BEHAVIOR.md")
            || candidate == root.join("IDENTITY.md")
            || candidate == root.join("CHARACTER.md")
    }
    match payload {
        serde_json::Value::Object(map) => map.iter().any(|(path, value)| {
            matches(path, root)
                || value
                    .get("path")
                    .and_then(|value| value.as_str())
                    .is_some_and(|path| matches(path, root))
        }),
        serde_json::Value::Array(entries) => entries.iter().any(|value| {
            value
                .get("path")
                .and_then(|value| value.as_str())
                .is_some_and(|path| matches(path, root))
        }),
        _ => false,
    }
}

fn format_command_approval(event: &CommandApprovalEvent) -> String {
    let mut lines = vec!["[审批请求] Codex 想执行 shell 命令".to_string()];
    if let Some(cmd) = event.command.as_deref() {
        lines.push("命令：".to_string());
        lines.push(format!("```shell\n{cmd}\n```"));
    }
    if let Some(cwd) = event.cwd.as_deref() {
        lines.push(format!("目录：`{cwd}`"));
    }
    if let Some(reason) = event.reason.as_deref().filter(|r| !r.trim().is_empty()) {
        lines.push(format!("原因：{}", reason.trim()));
    }
    lines.push(
        "——\n/同意            仅本次放行\n/同意本会话      本轮后续同类命令自动放行\n/拒绝            拒绝，Codex 会尝试别的方式\n/取消            拒绝并终止当前回合"
            .to_string(),
    );
    lines.join("\n")
}

fn format_file_change_approval(event: &FileChangeApprovalEvent) -> String {
    let mut lines = vec!["[审批请求] Codex 想写入/修改文件".to_string()];
    if let Some(reason) = event.reason.as_deref().filter(|r| !r.trim().is_empty()) {
        lines.push(format!("原因：{}", reason.trim()));
    }
    if let Some(root) = event.grant_root.as_deref() {
        lines.push(format!("授权目录：`{root}`"));
    }
    let summary = summarize_file_changes(&event.file_changes);
    if !summary.is_empty() {
        lines.push(format!("变更：{summary}"));
    }
    lines.push("——\n/同意 /同意本会话 /拒绝 /取消".to_string());
    lines.join("\n")
}

fn format_permissions_approval(event: &PermissionsApprovalEvent) -> String {
    let mut lines = vec!["[审批请求] Codex 请求权限升级".to_string()];
    if let Some(reason) = event.reason.as_deref().filter(|r| !r.trim().is_empty()) {
        lines.push(format!("原因：{}", reason.trim()));
    }
    let summary = serde_json::to_string(&event.permissions).unwrap_or_default();
    if !summary.is_empty() && summary != "null" {
        let trimmed: String = summary.chars().take(400).collect();
        lines.push(format!("请求：{trimmed}"));
    }
    lines.push("——\n/同意 /拒绝 /取消".to_string());
    lines.join("\n")
}

fn summarize_file_changes(payload: &serde_json::Value) -> String {
    // Payload shape is a map of path -> change descriptor or an array.
    let mut paths: Vec<String> = Vec::new();
    match payload {
        serde_json::Value::Object(map) => {
            for (k, _) in map.iter().take(6) {
                paths.push(k.clone());
            }
        }
        serde_json::Value::Array(arr) => {
            for entry in arr.iter().take(6) {
                if let Some(p) = entry.get("path").and_then(|v| v.as_str()) {
                    paths.push(p.to_string());
                }
            }
        }
        _ => {}
    }
    if paths.is_empty() {
        return String::new();
    }
    paths.join(", ")
}


#[cfg(test)]
mod tests {
    use super::changes_protected_persona_file;

    #[test]
    fn protected_persona_file_detection_is_exact() {
        let root = std::path::Path::new("/srv/codex-claw");
        assert!(changes_protected_persona_file(
            &serde_json::json!({"/srv/codex-claw/BEHAVIOR.md":{"type":"update"}}),
            root,
        ));
        assert!(changes_protected_persona_file(
            &serde_json::json!({"/srv/codex-claw/CHARACTER.md":{"type":"update"}}),
            root,
        ));
        assert!(changes_protected_persona_file(
            &serde_json::json!([{"path":"/srv/codex-claw/IDENTITY.md"}]),
            root,
        ));
        assert!(!changes_protected_persona_file(
            &serde_json::json!({"CHARACTER.md":{"type":"update"}}),
            root,
        ));
        assert!(!changes_protected_persona_file(
            &serde_json::json!({"/tmp/project/CHARACTER.md":{"type":"update"}}),
            root,
        ));
    }
}
