use crate::{
    codex::CodexExecutor, config::AppConfig, session::SessionStore, state::StateDb,
    work_queue::WorkQueue,
};
use std::sync::Arc;
pub struct SchedulerCtx {
    pub config: AppConfig,
    pub state: StateDb,
    pub session: Arc<SessionStore>,
    pub codex: Arc<CodexExecutor>,
    pub work_queue: WorkQueue,
}
