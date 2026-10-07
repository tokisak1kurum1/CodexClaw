use crate::{model::settings::ReasoningEffort, util::path::home_dir};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub general: GeneralConfig,
    pub qq: QqConfig,
    #[serde(default)]
    pub codex: CodexConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub scheduler: SchedulerConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub attachments: AttachmentConfig,
}
macro_rules! section{($name:ident{$($field:ident:$ty:ty=$value:expr),*$(,)?})=>{#[derive(Debug,Clone,Serialize,Deserialize)]#[serde(default)]pub struct $name{$(pub $field:$ty),*}impl Default for $name{fn default()->Self{Self{$($field:$value),*}}}};}
section!(CodexConfig {
    expected_version: String = "0.159.2".into()
});
section!(RuntimeConfig {
    max_concurrent_codex: usize = 2,
    max_concurrent_per_user: usize = 1,
    max_concurrent_scheduled: usize = 1,
    max_concurrent_memory: usize = 1
});
section!(SchedulerConfig {
    enabled: bool = true,
    tick_secs: u64 = 5,
    default_tz: String = "Asia/Shanghai".into(),
    max_turn_secs: u64 = 600,
    agent_misfire_grace_secs: i64 = 600,
    reminder_misfire_grace_secs: i64 = 1800,
    catch_up_missed: bool = false
});
section!(MemoryConfig {
    model: String = "gpt-5.6-luna".into(),
    distill_after_turns: i64 = 12,
    distill_idle_secs: i64 = 120,
    relevant_limit: usize = 8,
    hot_memory_max_chars: usize = 4000,
    single_memory_max_chars: usize = 300
});
section!(AttachmentConfig {
    max_file_bytes: u64 = 32 * 1024 * 1024,
    per_user_quota_bytes: u64 = 256 * 1024 * 1024,
    retention_hours: i64 = 24
});
section!(GeneralConfig {
    data_dir: PathBuf = home_dir().join(".codex-claw/data"),
    system_codex_home: PathBuf = home_dir().join(".codex"),
    codex_home_global: PathBuf = home_dir().join(".codex-claw/.codex"),
    default_workspace_dir: PathBuf = home_dir().join(".codex-claw/users"),
    codex_binary: String = "codex".into(),
    default_model: String = String::new(),
    timezone: String = "Asia/Shanghai".into(),
    default_reasoning_effort: ReasoningEffort = ReasoningEffort::Medium
});
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QqConfig {
    pub app_id: String,
    pub app_secret: String,
    #[serde(default)]
    pub allowed_users: Vec<String>,
    #[serde(default = "api_url")]
    pub api_base_url: String,
    #[serde(default = "token_url")]
    pub token_url: String,
}
fn api_url() -> String {
    "https://api.sgroup.qq.com".into()
}
fn token_url() -> String {
    "https://bots.qq.com/app/getAppAccessToken".into()
}
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: Default::default(),
            qq: QqConfig {
                app_id: String::new(),
                app_secret: String::new(),
                allowed_users: Vec::new(),
                api_base_url: api_url(),
                token_url: token_url(),
            },
            codex: Default::default(),
            runtime: Default::default(),
            scheduler: Default::default(),
            memory: Default::default(),
            attachments: Default::default(),
        }
    }
}
impl AppConfig {
    pub fn load() -> Result<Self> {
        let path = std::env::var_os("CODEX_CLAW_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let local = PathBuf::from("codexclaw.toml");
                if local.exists() {
                    local
                } else {
                    home_dir().join(".codex-claw/codexclaw.toml")
                }
            });
        let config: Self = toml::from_str(
            &std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?,
        )?;
        config.validate()?;
        Ok(config)
    }
    fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            (1..=2).contains(&self.runtime.max_concurrent_codex),
            "runtime.max_concurrent_codex must be 1 or 2"
        );
        anyhow::ensure!(
            self.runtime.max_concurrent_per_user == 1
                && self.runtime.max_concurrent_scheduled == 1
                && self.runtime.max_concurrent_memory == 1,
            "per-user, scheduled and memory limits must be 1"
        );
        anyhow::ensure!(!self.scheduler.catch_up_missed, "catch-up is not supported");
        anyhow::ensure!(
            !self.codex.expected_version.is_empty(),
            "codex.expected_version is required"
        );
        let mut allowed = std::collections::HashSet::new();
        anyhow::ensure!(
            self.qq
                .allowed_users
                .iter()
                .all(|user| {
                    !user.is_empty() && user.trim() == user && allowed.insert(user.as_str())
                }),
            "qq.allowed_users must contain unique non-empty openids"
        );
        anyhow::ensure!(
            self.general.timezone.parse::<chrono_tz::Tz>().is_ok(),
            "invalid timezone"
        );
        anyhow::ensure!(
            self.scheduler.tick_secs > 0
                && self.scheduler.max_turn_secs > 0
                && self.scheduler.agent_misfire_grace_secs >= 0
                && self.scheduler.reminder_misfire_grace_secs >= 0,
            "scheduler timers must be positive"
        );
        anyhow::ensure!(
            !self.memory.model.trim().is_empty()
                && self.memory.distill_after_turns > 0
                && self.memory.distill_idle_secs > 0
                && self.memory.relevant_limit <= 8
                && self.memory.hot_memory_max_chars <= 4000
                && self.memory.single_memory_max_chars == 300,
            "invalid memory limits"
        );
        anyhow::ensure!(
            self.attachments.retention_hours > 0
                && self.attachments.max_file_bytes > 0
                && self.attachments.per_user_quota_bytes >= self.attachments.max_file_bytes,
            "invalid attachment limits"
        );
        Ok(())
    }
    pub async fn normalize_paths(&mut self) -> Result<()> {
        self.general.data_dir = normalize_path(&self.general.data_dir)?;
        self.general.codex_home_global = normalize_path(&self.general.codex_home_global)?;
        self.general.system_codex_home = normalize_path(&self.general.system_codex_home)?;
        self.general.default_workspace_dir = normalize_path(&self.general.default_workspace_dir)?;
        Ok(())
    }
}
fn normalize_path(p: &Path) -> Result<PathBuf> {
    let text = p.to_string_lossy();
    let p = if let Some(rest) = text.strip_prefix("~/") {
        home_dir().join(rest)
    } else {
        p.to_owned()
    };
    let p = if p.is_absolute() {
        p
    } else {
        std::env::current_dir()?.join(p)
    };
    std::fs::create_dir_all(&p)?;
    Ok(std::fs::canonicalize(p)?)
}
