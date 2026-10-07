//! Per-user session settings and the `state.json` document they persist into.
//!
//! These are pure value types: they own their own parsing/formatting but reach
//! for nothing outside `model/`, which is what lets `config` and `codex` depend
//! on them without pulling in `session`.

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    #[default]
    Medium,
    High,
    Xhigh,
    Max,
}

impl ReasoningEffort {
    pub(crate) fn parse_supported(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::Xhigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    pub(crate) fn normalized(self) -> Self {
        match self {
            Self::None | Self::Minimal => Self::Low,
            other => other,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self.normalized() {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
            Self::None | Self::Minimal => "low",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceTier {
    Fast,
    Flex,
}

impl ServiceTier {
    pub(crate) fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "fast" | "on" => Some(Self::Fast),
            "flex" | "off" => Some(Self::Flex),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Flex => "flex",
        }
    }

    /// The user-facing on/off/inherit label for a fast-tier setting. Shared by
    /// the `/fast` status view and the global-setting confirmation reply.
    pub(crate) fn fast_label(tier: Option<ServiceTier>) -> &'static str {
        match tier {
            Some(Self::Fast) => "on",
            Some(Self::Flex) => "off",
            None => "inherit",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ApprovalPolicySetting {
    UnlessTrusted,
    OnRequest,
    Never,
    GuardianSubagent,
}

impl ApprovalPolicySetting {
    pub(crate) fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "never" | "off" | "关" | "关闭" => Some(Self::Never),
            "on-request" | "on" | "开" | "开启" | "ask" => Some(Self::OnRequest),
            "untrusted" | "unless-trusted" | "strict" | "严格" => Some(Self::UnlessTrusted),
            "guardian-subagent" | "guardian" | "guardian_subagent" | "守护" => {
                Some(Self::GuardianSubagent)
            }
            _ => None,
        }
    }

    pub(crate) fn label_zh(self) -> &'static str {
        match self {
            Self::UnlessTrusted => "严格（unless-trusted）",
            Self::OnRequest => "按需（on-request）",
            Self::Never => "关闭（never）",
            Self::GuardianSubagent => "守护子代理（guardian-subagent）",
        }
    }

    pub(crate) fn label_en(self) -> &'static str {
        match self {
            Self::UnlessTrusted => "unless-trusted",
            Self::OnRequest => "on-request",
            Self::Never => "never",
            Self::GuardianSubagent => "guardian-subagent",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ContextMode {
    Standard,
    #[serde(rename = "1m")]
    OneM,
}

impl ContextMode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "standard" | "272k" => Some(Self::Standard),
            "1m" => Some(Self::OneM),
            _ => None,
        }
    }
    #[cfg(test)]
    pub(crate) const STANDARD_CONTEXT_WINDOW: u64 = 272_000;

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Standard => "272K",
            Self::OneM => "1M",
        }
    }

    #[cfg(test)]
    pub(crate) fn from_model_context_window(window: u64) -> Self {
        if window > Self::STANDARD_CONTEXT_WINDOW {
            Self::OneM
        } else {
            Self::Standard
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SessionSettings {
    pub(crate) model_override: Option<String>,
    pub(crate) reasoning_effort: Option<ReasoningEffort>,
    pub(crate) service_tier: Option<ServiceTier>,
    pub(crate) context_mode: Option<ContextMode>,
    #[serde(default)]
    pub(crate) verbose: bool,
    #[serde(default)]
    pub(crate) plan_mode: bool,
    #[serde(default)]
    pub(crate) approval_policy_override: Option<ApprovalPolicySetting>,
    #[serde(default)]
    pub(crate) pending_plan: Option<String>,
    #[serde(default = "default_language")]
    pub(crate) language: String,
}

pub(crate) fn default_language() -> String {
    "zh".to_string()
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            model_override: None,
            reasoning_effort: None,
            service_tier: None,
            context_mode: None,
            verbose: false,
            plan_mode: false,
            approval_policy_override: None,
            pending_plan: None,
            language: default_language(),
        }
    }
}

impl SessionSettings {
    pub(crate) fn merged_with_profile(&self, profile: Option<&DialogProfile>) -> Self {
        let mut merged = self.clone();
        let Some(profile) = profile else {
            return merged;
        };
        if profile.model_override.is_some() {
            merged.model_override = profile.model_override.clone();
        }
        if profile.reasoning_effort.is_some() {
            merged.reasoning_effort = profile.reasoning_effort;
        }
        if profile.service_tier.is_some() {
            merged.service_tier = profile.service_tier;
        }
        if profile.context_mode.is_some() {
            merged.context_mode = profile.context_mode;
        }
        merged
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SessionState {
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) settings: SessionSettings,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum DialogOrigin {
    #[default]
    Local,
    Global,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) struct DialogProfile {
    pub(crate) model_override: Option<String>,
    pub(crate) reasoning_effort: Option<ReasoningEffort>,
    pub(crate) service_tier: Option<ServiceTier>,
    pub(crate) context_mode: Option<ContextMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct TokenUsageSnapshot {
    pub(crate) total_tokens: u64,
    pub(crate) window: u64,
    #[serde(default)]
    pub(crate) input_tokens: u64,
    #[serde(default)]
    pub(crate) cached_input_tokens: u64,
    #[serde(default)]
    pub(crate) output_tokens: u64,
    pub(crate) updated_at: chrono::DateTime<chrono::Utc>,
}

impl TokenUsageSnapshot {
    pub(crate) fn context_tokens(&self) -> Option<u64> {
        if self.window > 0 && self.total_tokens > self.window {
            return None;
        }
        Some(self.total_tokens)
    }

    pub(crate) fn percent_remaining(&self) -> Option<u64> {
        if self.window == 0 {
            return None;
        }

        const BASELINE_TOKENS: u64 = 12_000;
        if self.window <= BASELINE_TOKENS {
            return Some(0);
        }

        let effective_window = self.window - BASELINE_TOKENS;
        let used = self.context_tokens()?.saturating_sub(BASELINE_TOKENS);
        let remaining = effective_window.saturating_sub(used);
        Some(
            ((remaining as f64 / effective_window as f64) * 100.0)
                .clamp(0.0, 100.0)
                .round() as u64,
        )
    }

    pub(crate) fn percent_used(&self) -> Option<u64> {
        self.percent_remaining()
            .map(|value| 100_u64.saturating_sub(value))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct DialogState {
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) origin: DialogOrigin,
    pub(crate) workspace_dir: PathBuf,
    #[serde(default)]
    pub(crate) saved: bool,
    #[serde(default)]
    pub(crate) profile: Option<DialogProfile>,
    #[serde(default)]
    pub(crate) last_usage: Option<TokenUsageSnapshot>,
    /// Monotonically bumped every time this dialog is installed as the
    /// foreground. Two fresh temporary dialogs are value-identical in every
    /// other field, so an interrupted turn's compare-and-set binding needs
    /// this to notice that /stop or /new swapped the foreground mid-turn.
    /// Absent in state files written before the field existed (reads as 0).
    #[serde(default)]
    pub(crate) generation: u64,
    /// The background alias this dialog answers to. Set the first time it is
    /// parked and carried along into the foreground, so `/fg` then `/bg` puts
    /// it back under the name the user gave it instead of a fresh random one.
    /// `None` for a dialog that has never been in the background.
    #[serde(default)]
    pub(crate) alias: Option<String>,
}

impl DialogState {
    pub(crate) fn new_temporary(workspace_dir: PathBuf) -> Self {
        Self {
            session_id: None,
            origin: DialogOrigin::Local,
            workspace_dir,
            saved: false,
            profile: None,
            last_usage: None,
            generation: 0,
            alias: None,
        }
    }

    pub(crate) fn is_temporary(&self) -> bool {
        self.session_id.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PendingSetting {
    Model,
    Reasoning,
    Fast,
    Context,
    Verbose,
    Lang,
    SessionsProjects,
    SessionsSessions { project_key: String, page: usize },
    Fg,
    ResumeProjects,
    ResumeSessions { project_key: String, page: usize },
    Approvals,
    Plan,
    ResumeRecovery,
}

impl PendingSetting {
    pub(crate) fn command_name(&self, locale: &str) -> &'static str {
        use PendingSetting::*;
        let zh = locale.eq_ignore_ascii_case("zh");
        match self {
            Model => {
                if zh {
                    "/模型"
                } else {
                    "/model"
                }
            }
            Reasoning => {
                if zh {
                    "/思考"
                } else {
                    "/reasoning"
                }
            }
            Fast => {
                if zh {
                    "/快速"
                } else {
                    "/fast"
                }
            }
            Context => {
                if zh {
                    "/上下文"
                } else {
                    "/context"
                }
            }
            Verbose => {
                if zh {
                    "/详细"
                } else {
                    "/verbose"
                }
            }
            Lang => {
                if zh {
                    "/语言"
                } else {
                    "/lang"
                }
            }
            SessionsProjects | SessionsSessions { .. } => {
                if zh {
                    "/会话"
                } else {
                    "/sessions"
                }
            }
            Fg => {
                if zh {
                    "/前台"
                } else {
                    "/fg"
                }
            }
            ResumeProjects | ResumeSessions { .. } => {
                if zh {
                    "/恢复"
                } else {
                    "/resume"
                }
            }
            Approvals => {
                if zh {
                    "/审批"
                } else {
                    "/approvals"
                }
            }
            Plan => {
                if zh {
                    "/计划"
                } else {
                    "/plan"
                }
            }
            ResumeRecovery => {
                if zh {
                    "/恢复"
                } else {
                    "/resume"
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct UserSessionState {
    pub(crate) foreground: DialogState,
    #[serde(default)]
    pub(crate) background: BTreeMap<String, DialogState>,
    #[serde(default)]
    pub(crate) background_order: Vec<String>,
    #[serde(default)]
    pub(crate) settings: SessionSettings,
    #[serde(default)]
    pub(crate) alias_seq: u64,
    #[serde(default)]
    pub(crate) last_projects_view: Vec<String>,
    #[serde(default)]
    pub(crate) last_sessions_view: Vec<String>,
    /// Job ids as rendered by the latest `/cron list`, so numeric arguments
    /// keep meaning the row the user actually saw.
    #[serde(default)]
    pub(crate) last_cron_view: Vec<String>,
    #[serde(default)]
    pub(crate) saved_local_session_ids: Vec<String>,
    #[serde(default)]
    pub(crate) pending_setting: Option<PendingSetting>,
    /// Alias reserved by `/bg <alias>` while the foreground's first turn was
    /// still running (nothing to park yet); consumed by the turn-end park so
    /// the finished conversation lands under the name the user asked for.
    #[serde(default)]
    pub(crate) pending_park_alias: Option<String>,
}

impl UserSessionState {
    /// Canonical "fresh user" constructor: the single source of truth for the
    /// initial field values. Callers that need a different foreground or
    /// settings (legacy migration) construct via `new` and overwrite just the
    /// fields that differ.
    pub(crate) fn new(default_workspace_dir: PathBuf) -> Self {
        Self {
            foreground: DialogState::new_temporary(default_workspace_dir),
            background: BTreeMap::new(),
            background_order: Vec::new(),
            settings: SessionSettings::default(),
            alias_seq: 0,
            last_projects_view: Vec::new(),
            last_sessions_view: Vec::new(),
            last_cron_view: Vec::new(),
            saved_local_session_ids: Vec::new(),
            pending_setting: None,
            pending_park_alias: None,
        }
    }

    /// The dialog profile that participates in effective-settings resolution:
    /// only a *saved* foreground dialog carries one; temporary dialogs always
    /// resolve against the user's own settings.
    pub(crate) fn foreground_profile(&self) -> Option<&DialogProfile> {
        if self.foreground.session_id.is_some() {
            self.foreground.profile.as_ref()
        } else {
            None
        }
    }

    /// The settings a turn actually runs with: the user's settings with the
    /// four runtime overrides (model / reasoning / tier / context) replaced by
    /// the saved foreground dialog profile, if any.
    pub(crate) fn effective_settings(&self) -> SessionSettings {
        self.settings.merged_with_profile(self.foreground_profile())
    }
}

#[cfg(test)]
mod tests {
    use super::ContextMode;

    #[test]
    fn context_window_above_standard_is_one_m() {
        assert_eq!(
            ContextMode::from_model_context_window(ContextMode::STANDARD_CONTEXT_WINDOW + 1),
            ContextMode::OneM
        );
        assert_eq!(
            ContextMode::from_model_context_window(950_000),
            ContextMode::OneM
        );
    }

    #[test]
    fn standard_context_window_stays_standard() {
        assert_eq!(
            ContextMode::from_model_context_window(ContextMode::STANDARD_CONTEXT_WINDOW),
            ContextMode::Standard
        );
        assert_eq!(
            ContextMode::from_model_context_window(128_000),
            ContextMode::Standard
        );
    }
}

#[cfg(test)]
mod reasoning_effort_tests {
    use super::ReasoningEffort;

    #[test]
    fn reasoning_effort_supports_max() {
        assert_eq!(ReasoningEffort::parse_supported("max"), Some(ReasoningEffort::Max));
        assert_eq!(ReasoningEffort::Max.as_str(), "max");
    }
}
