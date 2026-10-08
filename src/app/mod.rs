mod active_turns;
mod approvals;
mod format;
mod inbound;
mod outgoing;
mod turn;
use crate::{
    codex::{ApprovalOutcome, CodexExecutor},
    config::AppConfig,
    memory::MemoryStore,
    qq::{Directive, QqApiClient},
    session::SessionStore,
    state::StateDb,
    work_queue::WorkQueue,
};
use active_turns::ActiveTurns;
use anyhow::Result;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};
use tokio::sync::{Mutex, oneshot};
pub struct App {
    pub config: AppConfig,
    pub(crate) session: Arc<SessionStore>,
    pub qq_client: Arc<QqApiClient>,
    pub codex: Arc<CodexExecutor>,
    pub(crate) memory: Arc<MemoryStore>,
    pub state: StateDb,
    pub work_queue: WorkQueue,
    pub(crate) active_turns: Mutex<ActiveTurns>,
    pending_approvals: Mutex<HashMap<String, VecDeque<PendingApprovalEntry>>>,
}
enum PendingApprovalEntry {
    Outcome(oneshot::Sender<ApprovalOutcome>),
}
impl App {
    pub fn new(
        config: AppConfig,
        session: Arc<SessionStore>,
        qq_client: Arc<QqApiClient>,
        codex: Arc<CodexExecutor>,
        memory: Arc<MemoryStore>,
        work_queue: WorkQueue,
    ) -> Arc<Self> {
        let state = session.db.clone();
        let app = Arc::new(Self {
            config,
            session,
            qq_client,
            codex,
            memory,
            state,
            work_queue,
            active_turns: Mutex::new(ActiveTurns::default()),
            pending_approvals: Mutex::new(HashMap::new()),
        });
        app.clone().install_approval_handler();
        app
    }
    pub(crate) async fn command_locale(&self, user: &str) -> String {
        self.session.command_locale(user).await
    }
    async fn reply_text(&self, user: &str, message: &str, text: &str) -> Result<()> {
        let locale = self.command_locale(user).await;
        let text = if locale == "zh" {
            match text {
                "stopped" => "已停止",
                "interrupted" => "已中断",
                "new conversation" => "已新建会话",
                "approval resolved" => "已处理审批",
                "no pending approval" => "当前没有待处理的审批",
                "conversation compacted" => "会话已压缩",
                "conversation resumed" => "会话已恢复",
                "model updated" => "模型已更新",
                "reasoning updated" => "思考强度已更新",
                "service tier updated" => "快速模式已更新",
                "context updated" => "上下文设置已更新",
                "language updated" => "语言已更新",
                "updated" => "设置已更新",
                "saved" => "已保存",
                "renamed" => "已重命名",
                "cancelled" => "已取消",
                other => other,
            }
        } else {
            text
        };
        self.qq_client
            .send_text(user, message, text)
            .await
    }
    async fn send_directive(&self, user: &str, message: &str, d: Directive) -> Result<()> {
        let (path, name, kind) = match d {
            Directive::Image { path } => (path, None, 1),
            Directive::File { path, name } => (path, name, 4),
        };
        let root = self.session.user_root(user);
        let directive = match kind {
            1 => Directive::Image { path: path.clone() },
            _ => Directive::File { path: path.clone(), name: name.clone() },
        };
        outgoing::validate_directive(&root, &directive, self.config.attachments.max_file_bytes)?;
        let canonical = std::fs::canonicalize(&path)?;
        let uploaded = self
            .qq_client
            .upload_file(user, &canonical, kind, name.as_deref())
            .await?;
        self.qq_client.send_media(user, message, &uploaded).await
    }
    pub fn start_workers(self: &Arc<Self>) {
        inbound::spawn_inbox(self.clone());
        inbound::spawn_outbox(self.clone());
        crate::memory::distill::spawn(self.clone());
        inbound::spawn_attachment_cleanup(self.clone());
    }
}
