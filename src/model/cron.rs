use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct CronJob {
    pub id: String,
    #[serde(alias = "owner_openid")]
    pub user_id: String,
    pub title: String,
    pub kind: CronKind,
    pub action: JobAction,
    pub created_at: DateTime<Utc>,
    pub next_run_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub disabled: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum CronKind {
    Recurring { cron: String, tz: String },
    OneShot { at: DateTime<Utc> },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum JobAction {
    Reminder {
        message: String,
    },
    #[serde(alias = "codex-turn", alias = "codex-exec")]
    CodexTask {
        prompt: String,
        #[serde(default)]
        model: Option<String>,
    },
}
