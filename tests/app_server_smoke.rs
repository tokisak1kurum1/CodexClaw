//! End-to-end smoke test: spawn a real `codex app-server` and drive one turn
//! through the new [`AppServerHandle`] / [`CodexExecutor`] pipeline.
//!
//! Requires a working codex binary on $PATH and valid `~/.codex-claw/.codex`
//! auth. Run with:
//!
//! ```
//! cargo test --test app_server_smoke -- --ignored --nocapture
//! ```

use std::{path::PathBuf, sync::Arc, time::Duration};

use codex_claw::codex::{
    AppServerHandle, ClientInfo, ExecutionRequest, ExecutionUpdate, app_server::TurnPolicy,
    build_codex_path_env,
};
use codex_claw::session::state::{ReasoningEffort, SessionState};
use tokio::sync::mpsc;

#[tokio::test]
#[ignore = "requires real codex binary and auth; run manually"]
async fn one_shot_turn_round_trips_through_app_server() {
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex-claw").join(".codex"))
        })
        .expect("CODEX_HOME resolvable");

    let path_env = build_codex_path_env(
        std::env::var_os("PATH").as_ref(),
        std::env::var_os("HOME")
            .as_deref()
            .map(std::path::Path::new),
    );

    let handle = tokio::time::timeout(
        Duration::from_secs(30),
        AppServerHandle::start(
            PathBuf::from("codex"),
            codex_home.clone(),
            codex_home.join("sqlite"),
            path_env,
            ClientInfo {
                name: "codex-claw-smoke".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                title: None,
            },
        ),
    )
    .await
    .expect("app-server initialize timed out")
    .expect("start app-server");

    let (tx, mut rx) = mpsc::unbounded_channel::<ExecutionUpdate>();
    let update_task = tokio::spawn(async move {
        let mut seen = Vec::<ExecutionUpdate>::new();
        while let Some(u) = rx.recv().await {
            seen.push(u);
        }
        seen
    });

    let workspace = tempfile::tempdir().unwrap();
    let request = ExecutionRequest {
        prompt: "Print the literal string EXACTLY: ACK".to_string(),
        workspace_dir: workspace.path().to_path_buf(),
        codex_home: PathBuf::from("/tmp"), // ignored in app-server path
        config_overrides: Vec::new(),
        add_dirs: Vec::new(),
        session_state: SessionState::default(),
        model: std::env::var("CODEX_SMOKE_MODEL").ok(),
        service_tier: None,
        context_mode: None,
        reasoning_effort: ReasoningEffort::Medium,
        image_paths: Vec::new(),
        developer_instructions: None,
        ephemeral: false,
        owner_user_id: None,
    };

    // Inherit sandbox/approval from ~/.codex-claw/.codex/config.toml so the
    // smoke test matches the real bot behaviour.
    let policy = TurnPolicy::inherit_from_config();
    let result = tokio::time::timeout(
        Duration::from_secs(90),
        Arc::new(handle.clone()).execute(request.clone(), policy.clone(), None, Some(tx)),
    )
    .await
    .expect("turn did not complete in time")
    .expect("turn failed");

    let updates = update_task.await.unwrap();
    handle.shutdown().await;
    println!("session_id = {:?}", result.session_id);
    println!("text       = {:?}", result.text);
    println!("updates    = {}", updates.len());
    for u in &updates {
        println!("  {u:?}");
    }

    assert!(result.session_id.is_some(), "session id captured");
    assert_eq!(
        result.text.trim(),
        "ACK",
        "expected the requested final answer"
    );
    assert!(
        !updates.is_empty(),
        "at least one update emitted (bash/agent/reasoning)"
    );

    // Reconnect through a fresh proxy so the next execution must thread/resume.
    let handle = AppServerHandle::start(
        PathBuf::from("codex"),
        codex_home.clone(),
        codex_home.join("sqlite"),
        build_codex_path_env(
            std::env::var_os("PATH").as_ref(),
            std::env::var_os("HOME").map(PathBuf::from).as_deref(),
        ),
        ClientInfo {
            name: "codex-claw-smoke".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            title: None,
        },
    )
    .await
    .expect("initialize after reconnect");
    let mut resumed = request;
    resumed.session_state = serde_json::from_value(serde_json::json!({
        "session_id": result.session_id,
        "settings": {}
    }))
    .expect("resume session state");
    resumed.prompt = "Reply with exactly ACK2".into();
    let resumed_output = tokio::time::timeout(
        Duration::from_secs(90),
        handle.execute(resumed.clone(), policy.clone(), None, None),
    )
    .await
    .expect("resume turn timeout")
    .expect("thread/resume turn");
    assert_eq!(resumed_output.text.trim(), "ACK2");

    resumed.prompt = "Think carefully about a complex scheduling design before answering.".into();
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let (updates_tx, mut updates_rx) = mpsc::unbounded_channel();
    let running_handle = handle.clone();
    let running = tokio::spawn(async move {
        running_handle
            .execute(resumed, policy, Some(cancel_rx), Some(updates_tx))
            .await
    });
    let (thread, turn) = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(update) = updates_rx.recv().await {
            if let ExecutionUpdate::TurnStarted { thread_id, turn_id } = update {
                return (thread_id, turn_id);
            }
        }
        panic!("turn/start did not announce the active turn");
    })
    .await
    .expect("turn/start timeout");
    let steered = handle
        .steer(&thread, &turn, "smoke-steer", "Also consider fairness.")
        .await
        .expect("turn/steer with expectedTurnId");
    assert_eq!(steered, turn);
    cancel_tx.send(()).expect("send turn interruption");
    let interrupted = tokio::time::timeout(Duration::from_secs(20), running)
        .await
        .expect("interrupt timeout")
        .expect("turn task joined");
    assert!(
        interrupted.is_err(),
        "interrupted turn must not finish successfully"
    );
    handle.shutdown().await;
}
