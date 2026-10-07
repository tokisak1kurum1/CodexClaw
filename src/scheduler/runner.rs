use super::SchedulerCtx;
use crate::{
    codex::ExecutionRequest,
    model::{cron::JobAction, settings::SessionState},
    state::{jobs::ClaimedRun, outbox::Delivery},
    work_queue::WorkKind,
};
use anyhow::Result;
use std::time::Duration;
pub(super) async fn run(ctx: &SchedulerCtx, run: ClaimedRun) -> Result<()> {
    let user = &run.job.user_id;
    let deadline = run.scheduled_at + run.grace;
    if ctx
        .state
        .job_for(user, &run.job.id)?
        .is_none_or(|job| job.disabled)
    {
        ctx.state
            .mark_run(user, run.id, "missed", Some("job removed or paused"))?;
        return Ok(());
    }
    if chrono::Utc::now().timestamp() > deadline {
        ctx.state.mark_run(user, run.id, "missed", None)?;
        return Ok(());
    }
    let text = match &run.job.action {
        JobAction::Reminder { message } => message.clone(),
        JobAction::CodexTask { prompt, model } => {
            let wait =
                Duration::from_secs((deadline - chrono::Utc::now().timestamp()).max(0) as u64);
            let Some(_permit) = ctx
                .work_queue
                .acquire(
                    user,
                    WorkKind::ScheduledTurn,
                    Some(tokio::time::Instant::now() + wait),
                )
                .await
            else {
                ctx.state.mark_run(user, run.id, "missed", None)?;
                return Ok(());
            };
            if chrono::Utc::now().timestamp() > deadline {
                ctx.state.mark_run(user, run.id, "missed", None)?;
                return Ok(());
            }
            if ctx
                .state
                .job_for(user, &run.job.id)?
                .is_none_or(|job| job.disabled)
            {
                ctx.state.mark_run(
                    user,
                    run.id,
                    "missed",
                    Some("job removed or paused while queued"),
                )?;
                return Ok(());
            }
            ctx.state.mark_run(user, run.id, "running", None)?;
            let snapshot = ctx.session.snapshot_for_user(user).await?;
            let mut settings = snapshot.settings;
            settings.approval_policy_override =
                Some(crate::model::settings::ApprovalPolicySetting::Never);
            settings.plan_mode = false;
            let path = ctx.session.workspace_for(user);
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
            let request=ExecutionRequest{prompt:prompt.clone(),workspace_dir:path.clone(),codex_home:ctx.session.codex_home().to_owned(),config_overrides:Vec::new(),add_dirs:vec![path,ctx.session.inbox_for(user)],session_state:SessionState{session_id:None,settings:settings.clone()},model:model.clone().or(settings.model_override).or_else(||(!ctx.config.general.default_model.is_empty()).then(||ctx.config.general.default_model.clone())),service_tier:settings.service_tier,context_mode:settings.context_mode,reasoning_effort:settings.reasoning_effort.unwrap_or(ctx.config.general.default_reasoning_effort),image_paths:Vec::new(),developer_instructions:Some("Complete this scheduled task and return the result. Do not take over the user's conversation.".into()),ephemeral:true,owner_user_id:Some(user.clone())};
            let mut task = Box::pin(ctx.codex.execute(request, Some(cancel_rx), None));
            tokio::select! {result=&mut task=>result?.text,_=tokio::time::sleep(Duration::from_secs(ctx.config.scheduler.max_turn_secs))=>{let _=cancel_tx.send(());let _=tokio::time::timeout(Duration::from_secs(10),&mut task).await;anyhow::bail!("scheduled turn timed out")}}
        }
    };
    ctx.state.commit_scheduled_answer(
        user,
        run.id,
        &format!("scheduled:{}:{}", run.job.id, run.scheduled_at),
        &Delivery {
            text,
            directives: Vec::new(),
        },
    )?;
    Ok(())
}
