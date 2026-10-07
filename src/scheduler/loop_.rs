use super::{SchedulerCtx, runner};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};
pub struct Scheduler;
impl Scheduler {
    pub fn spawn(ctx: Arc<SchedulerCtx>) {
        if !ctx.config.scheduler.enabled {
            return;
        }
        tokio::spawn(async move {
            let active = Arc::new(Mutex::new(HashSet::new()));
            let mut tick =
                tokio::time::interval(Duration::from_secs(ctx.config.scheduler.tick_secs.max(1)));
            loop {
                tick.tick().await;
                if let Err(e) = ctx.state.claim_due_jobs() {
                    tracing::warn!(error=%e,"scheduler tick failed");
                    continue;
                }
                match ctx.state.claimed_jobs() {
                    Ok(runs) => {
                        for run in runs {
                            if active.lock().unwrap().len() >= 32 {
                                break;
                            }
                            if !active.lock().unwrap().insert(run.id) {
                                continue;
                            }
                            let ctx = ctx.clone();
                            let active = active.clone();
                            tokio::spawn(async move {
                                let id = run.id;
                                let user = run.job.user_id.clone();
                                if let Err(e) = runner::run(&ctx, run).await {
                                    let _ = ctx.state.mark_run(
                                        &user,
                                        id,
                                        "failed",
                                        Some(&e.to_string()),
                                    );
                                    tracing::warn!(error=%e,"scheduled job failed");
                                }
                                active.lock().unwrap().remove(&id);
                            });
                        }
                    }
                    Err(e) => tracing::warn!(error=%e,"load scheduled claims failed"),
                }
            }
        });
    }
}
