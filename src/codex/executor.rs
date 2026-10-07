use std::{env, ffi::OsString, path::Path, sync::Arc};

use anyhow::Result;
use tokio::sync::{mpsc, oneshot};

use crate::{
    codex::{
        app_server::{
            AppServerHandle, TurnPolicy,
            protocol::{ApprovalPolicy, ApprovalsReviewer, SandboxPolicy},
        },
        types::{CompactRequest, ExecutionRequest, ExecutionResult, ExecutionUpdate},
    },
    model::settings::ApprovalPolicySetting,
    util::path::search_path_dirs,
};

#[derive(Clone)]
pub struct CodexExecutor {
    handle: Arc<AppServerHandle>,
}

impl CodexExecutor {
    pub fn new(handle: Arc<AppServerHandle>) -> Self {
        Self { handle }
    }

    pub fn handle(&self) -> Arc<AppServerHandle> {
        self.handle.clone()
    }

    /// Execute one turn against the shared app-server. Chooses a per-turn
    /// [`TurnPolicy`] from the current session settings (plan mode + approval
    /// policy override).
    pub(crate) async fn execute(
        &self,
        request: ExecutionRequest,
        cancel_rx: Option<oneshot::Receiver<()>>,
        update_tx: Option<mpsc::UnboundedSender<ExecutionUpdate>>,
    ) -> Result<ExecutionResult> {
        let policy = build_turn_policy(&request);
        self.handle
            .execute(request, policy, cancel_rx, update_tx)
            .await
    }

    pub(crate) async fn compact_session(
        &self,
        request: CompactRequest,
        cancel_rx: Option<oneshot::Receiver<()>>,
    ) -> Result<()> {
        self.handle.compact_thread(request, cancel_rx).await
    }
}

/// Given an [`ExecutionRequest`], pick the appropriate turn policy.
///
/// Approval policy stays user/config driven. Filesystem sandboxing is fixed to
/// workspace-write for every normal QQ turn with only that user's workspace/inbox
/// roots; this also resets a thread after a read-only plan turn.
///
/// We only override the per-turn policy when:
/// - plan mode is active → force `ReadOnly` + `Never` approvals + Plan collab;
/// - the user explicitly set an approval override via `/approvals`.
fn build_turn_policy(request: &ExecutionRequest) -> TurnPolicy {
    if request.session_state.settings.plan_mode {
        return TurnPolicy::plan_mode();
    }
    let mut policy = if let Some(setting) = request.session_state.settings.approval_policy_override {
        match setting {
            ApprovalPolicySetting::GuardianSubagent => {
                TurnPolicy::with_approvals_reviewer(ApprovalsReviewer::GuardianSubagent)
            }
            _ => TurnPolicy::with_approval_policy(approval_setting_to_protocol(setting)),
        }
    } else {
        TurnPolicy::inherit_from_config()
    };
    let mut roots = request
        .add_dirs
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let cwd = request.workspace_dir.to_string_lossy().into_owned();
    if !roots.contains(&cwd) {
        roots.push(cwd);
    }
    policy.sandbox_policy = Some(SandboxPolicy::WorkspaceWrite {
        writable_roots: roots,
        network_access: true,
        exclude_tmpdir_env_var: false,
        exclude_slash_tmp: false,
    });
    policy
}

fn approval_setting_to_protocol(setting: ApprovalPolicySetting) -> ApprovalPolicy {
    match setting {
        ApprovalPolicySetting::UnlessTrusted => ApprovalPolicy::UnlessTrusted,
        ApprovalPolicySetting::OnRequest => ApprovalPolicy::OnRequest,
        ApprovalPolicySetting::Never => ApprovalPolicy::Never,
        ApprovalPolicySetting::GuardianSubagent => ApprovalPolicy::OnRequest,
    }
}

/// Directories a codex turn should be able to find binaries in, on top of the
/// inherited `PATH`. Wider than the self-update search list on purpose: a turn
/// may shell out to anything the user has installed.
const CODEX_HOME_BIN_DIRS: &[&str] = &[".cargo/bin", ".local/bin"];
const CODEX_SYSTEM_BIN_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
    "/usr/local/bin",
    "/usr/local/sbin",
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
];

pub fn build_codex_path_env(current: Option<&OsString>, home: Option<&Path>) -> Option<OsString> {
    let dirs = search_path_dirs(current, home, CODEX_HOME_BIN_DIRS, CODEX_SYSTEM_BIN_DIRS);
    if dirs.is_empty() {
        return None;
    }
    env::join_paths(dirs).ok()
}

#[cfg(test)]
mod tests {
    use std::{env, ffi::OsString};

    use tempfile::tempdir;

    use crate::codex::executor::build_codex_path_env;

    #[test]
    fn normal_turn_policy_is_workspace_write() {
        let request = crate::codex::types::ExecutionRequest {
            prompt: String::new(),
            workspace_dir: std::path::PathBuf::from("/tmp/user/workspace"),
            codex_home: std::path::PathBuf::from("/tmp/codex"),
            config_overrides: Vec::new(),
            add_dirs: vec![std::path::PathBuf::from("/tmp/user/inbox")],
            session_state: Default::default(),
            model: None,
            service_tier: None,
            context_mode: None,
            reasoning_effort: crate::model::settings::ReasoningEffort::Medium,
            image_paths: Vec::new(),
            developer_instructions: None,
            ephemeral: false,
            owner_user_id: Some("a".into()),
        };
        let policy = super::build_turn_policy(&request);
        assert!(matches!(
            policy.sandbox_policy,
            Some(crate::codex::app_server::protocol::SandboxPolicy::WorkspaceWrite { .. })
        ));
    }

    #[test]
    fn path_env_includes_home_bin_fallbacks() {
        let home = tempdir().unwrap();
        let joined =
            build_codex_path_env(Some(&OsString::from("/usr/bin")), Some(home.path())).unwrap();
        let paths = env::split_paths(&joined).collect::<Vec<_>>();
        assert!(paths.contains(&home.path().join(".cargo").join("bin")));
        assert!(paths.contains(&home.path().join(".local").join("bin")));
    }
}
