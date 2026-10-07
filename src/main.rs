use anyhow::Result;
use codex_claw::{
    app::App,
    codex::{
        AppServerHandle, ClientInfo, CodexExecutor, build_codex_path_env, config_snapshot, version,
    },
    config::AppConfig,
    memory::MemoryStore,
    qq::{QqApiClient, spawn_gateway},
    scheduler::{self, SchedulerCtx},
    session::SessionStore,
    state::{StateDb, migrate_legacy},
    work_queue::WorkQueue,
};
use std::{path::PathBuf, sync::Arc};
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|s| matches!(s.as_str(), "--help" | "-h"))
    {
        println!(
            "codex-claw [cron <command>]\nStart the QQ bot or manage an owned scheduled task."
        );
        return Ok(());
    }
    let mut config = AppConfig::load()?;
    config.normalize_paths().await?;
    let state = StateDb::open(&config.general.data_dir)?;
    migrate_legacy::migrate(&state, &config)?;
    if args.first().is_some_and(|s| s == "cron") {
        return scheduler::cli::run(&args[1..], &config).await;
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(config.general.data_dir.join("bot.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| anyhow::anyhow!("another CodexClaw process uses this data directory"))?;
    state.recover()?;
    config_snapshot::bootstrap_codex_home(
        &config.general.codex_home_global,
        &config.general.system_codex_home,
    )
    .await?;
    version::verify(
        &PathBuf::from(&config.general.codex_binary),
        &config.general.codex_home_global,
        &config.codex.expected_version,
    )
    .await?;
    let session = Arc::new(SessionStore::from_db(
        state.clone(),
        &config.general.data_dir,
        &config.general.codex_home_global,
    )?);
    let path_env = build_codex_path_env(
        std::env::var_os("PATH").as_ref(),
        std::env::var_os("HOME")
            .as_deref()
            .map(std::path::Path::new),
    );
    let handle = Arc::new(
        AppServerHandle::start(
            PathBuf::from(&config.general.codex_binary),
            config.general.codex_home_global.clone(),
            config.general.codex_home_global.join("sqlite"),
            path_env,
            ClientInfo {
                name: "codex-claw".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                title: None,
            },
        )
        .await?,
    );
    let codex = Arc::new(CodexExecutor::new(handle));
    let work_queue = WorkQueue::new(config.runtime.max_concurrent_codex);
    let memory = Arc::new(MemoryStore::new(state.clone()));
    let qq = Arc::new(QqApiClient::new(config.qq.clone())?);
    let scheduler = Arc::new(SchedulerCtx {
        config: config.clone(),
        state: state.clone(),
        session: session.clone(),
        codex: codex.clone(),
        work_queue: work_queue.clone(),
    });
    let app = App::new(config, session, qq, codex, memory, work_queue);
    app.start_workers();
    scheduler::Scheduler::spawn(scheduler);
    spawn_gateway(state, app.qq_client.clone());
    wait_for_shutdown().await;
    app.codex.handle().shutdown().await;
    Ok(())
}
async fn wait_for_shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
