pub mod app_server;
pub mod config_snapshot;
pub(crate) mod display;
pub(crate) mod executor;
pub(crate) mod prompt;
pub(crate) mod runtime;
pub(crate) mod types;
pub mod version;

// Façade: the symbols the rest of the crate reaches for, re-exported so callers
// import `crate::codex::X` instead of spelling out the submodule layout.
//
// The `pub` block is the part the binary and the app-server smoke test consume;
// everything else is `pub(crate)` so the compiler keeps reporting unused items
// instead of treating "some external crate might want it" as a use.
pub use app_server::{AppServerHandle, ClientInfo};
pub use executor::{CodexExecutor, build_codex_path_env};
pub use types::TokenUsageInfo;
pub use types::{ExecutionRequest, ExecutionResult, ExecutionUpdate};

pub(crate) use app_server::{
    ApprovalOutcome, ApprovalRequest, CommandApprovalEvent, FileChangeApprovalEvent,
    PermissionsApprovalEvent,
};

pub(crate) use prompt::build_prompt;
pub(crate) use runtime::CodexModelEntry;
pub(crate) use types::CompactRequest;
