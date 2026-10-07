//! Shared `codex app-server` stdio JSON-RPC backend.
//!
//! Replaces the per-turn `codex exec` subprocess with a single long-lived
//! `codex app-server` child process that serves every QQ user's conversations
//! as independent threads. Exposes [`AppServerHandle`] — the façade used by
//! `CodexExecutor` — which the existing call sites already speak to.

pub(crate) mod approvals;
pub(crate) mod client;
pub(crate) mod events;
pub(crate) mod protocol;
pub(crate) mod session;
pub(crate) mod supervisor;
pub(crate) mod transport;

use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::codex::types::{CompactRequest, ExecutionRequest, ExecutionResult, ExecutionUpdate};

pub use protocol::ClientInfo;
pub use session::TurnPolicy;

pub(crate) use approvals::{
    ApprovalBroker, ApprovalOutcome, ApprovalRequest, CommandApprovalEvent,
    FileChangeApprovalEvent, PermissionsApprovalEvent,
};
pub(crate) use session::{AppServerSession, RuntimeConfigSignature};
pub(crate) use supervisor::AppServerSupervisor;

/// Façade the rest of the project uses: holds the supervisor + broker and
/// exposes a single `execute` method mirroring the legacy executor contract.
#[derive(Clone)]
pub struct AppServerHandle {
    pub(crate) supervisor: Arc<AppServerSupervisor>,
    pub(crate) approvals: Arc<ApprovalBroker>,
    runtime_configs: Arc<Mutex<HashMap<String, RuntimeConfigSignature>>>,
    model_cache: Arc<
        Mutex<
            Option<(
                std::time::Instant,
                u64,
                Vec<crate::codex::runtime::CodexModelEntry>,
            )>,
        >,
    >,
}

impl AppServerHandle {
    pub async fn start(
        codex_binary: std::path::PathBuf,
        codex_home: std::path::PathBuf,
        sqlite_home: std::path::PathBuf,
        path_env: Option<std::ffi::OsString>,
        client_info: ClientInfo,
    ) -> Result<Self> {
        let supervisor =
            AppServerSupervisor::new(codex_binary, codex_home, sqlite_home, path_env, client_info);
        supervisor.start().await?;
        let approvals = ApprovalBroker::new(supervisor.clone());
        approvals.start().await?;
        Ok(Self {
            supervisor,
            approvals,
            runtime_configs: Arc::new(Mutex::new(HashMap::new())),
            model_cache: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn execute(
        &self,
        mut request: ExecutionRequest,
        policy: TurnPolicy,
        cancel_rx: Option<oneshot::Receiver<()>>,
        update_tx: Option<mpsc::UnboundedSender<ExecutionUpdate>>,
    ) -> Result<ExecutionResult> {
        if policy.plan_mode && request.model.is_none() {
            let models = self.models().await?;
            request.model = Some(
                models
                    .iter()
                    .find(|m| m.is_default)
                    .or_else(|| models.first())
                    .ok_or_else(|| anyhow::anyhow!("No available model for plan mode"))?
                    .name
                    .clone(),
            );
        }
        AppServerSession::new(self.supervisor.clone(), self.runtime_configs.clone())
            .execute(request, policy, cancel_rx, update_tx)
            .await
    }

    pub(crate) async fn compact_thread(
        &self,
        request: CompactRequest,
        cancel_rx: Option<oneshot::Receiver<()>>,
    ) -> Result<()> {
        AppServerSession::new(self.supervisor.clone(), self.runtime_configs.clone())
            .compact_thread(request, cancel_rx)
            .await
    }

    pub async fn steer(
        &self,
        thread: &str,
        turn: &str,
        message: &str,
        text: &str,
    ) -> Result<String> {
        let client = self.supervisor.client().await?;
        let params = protocol::TurnSteerParams {
            thread_id: thread.into(),
            expected_turn_id: turn.into(),
            client_user_message_id: Some(message.into()),
            input: vec![protocol::TurnInputItem::Text { text: text.into() }],
        };
        let response: protocol::TurnSteerResponse = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            client.request("turn/steer", &params),
        )
        .await??;
        Ok(response.turn_id)
    }
    pub(crate) async fn models(&self) -> Result<Vec<crate::codex::runtime::CodexModelEntry>> {
        let client = self.supervisor.client().await?;
        let connection = client.connection_id;
        let mut cache = self.model_cache.lock().await;
        if let Some((at, cached_connection, models)) = cache.as_ref() {
            if *cached_connection == connection
                && at.elapsed() < std::time::Duration::from_secs(300)
            {
                return Ok(models.clone());
            }
        }
        let mut models = Vec::new();
        let mut cursor = serde_json::Value::Null;
        let mut pages = 0;
        loop {
            pages += 1;
            anyhow::ensure!(pages <= 16, "model/list pagination exceeded limit");
            let v: serde_json::Value = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                client.request(
                    "model/list",
                    &serde_json::json!({"cursor":cursor,"limit":100}),
                ),
            )
            .await??;
            let data = v["data"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("model/list omitted data"))?;
            for m in data {
                let name = m["model"]
                    .as_str()
                    .or_else(|| m["id"].as_str())
                    .ok_or_else(|| anyhow::anyhow!("model identifier missing"))?;
                models.push(crate::codex::runtime::CodexModelEntry {
                    name: name.into(),
                    is_default: m["isDefault"].as_bool().unwrap_or(false),
                    aliases: Vec::new(),
                    description: m["description"].as_str().map(str::to_owned),
                    description_zh: None,
                    description_en: None,
                });
            }
            cursor = v["nextCursor"].clone();
            if cursor.is_null() {
                break;
            }
        }
        *cache = Some((std::time::Instant::now(), connection, models.clone()));
        Ok(models)
    }
    pub async fn shutdown(&self) {
        self.supervisor.shutdown().await;
    }
}
