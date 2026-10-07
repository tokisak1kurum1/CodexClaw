use crate::{
    config::AppConfig,
    model::cron::{CronJob, CronKind, JobAction},
    state::StateDb,
};
use anyhow::Result;
use chrono::Utc;
use serde_json::{Value, json};
pub async fn run(args: &[String], config: &AppConfig) -> Result<()> {
    let user = std::env::var("CODEX_CLAW_USER_ID")
        .map_err(|_| anyhow::anyhow!("CODEX_CLAW_USER_ID is required"))?;
    let db = StateDb::open(&config.general.data_dir)?;
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    println!("{}", command(&db, config, &user, &args)?);
    Ok(())
}
pub(crate) fn command(
    db: &StateDb,
    config: &AppConfig,
    user: &str,
    args: &[&str],
) -> Result<String> {
    let op = args.first().copied().unwrap_or("list");
    let result = match op {
        "list" => serde_json::to_value(db.jobs_for(user)?)?,
        "rm" => {
            json!({"removed":db.remove_job(user,args.get(1).ok_or_else(||anyhow::anyhow!("job id required"))?)?})
        }
        "tail" => json!(
            db.run_history(
                user,
                args.get(1)
                    .ok_or_else(|| anyhow::anyhow!("job id required"))?
            )?
        ),
        "pause" | "resume" | "run-now" => {
            let id = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("job id required"))?;
            let mut job = db
                .job_for(user, id)?
                .ok_or_else(|| anyhow::anyhow!("job not found for owner"))?;
            job.disabled = op == "pause";
            if op == "run-now" {
                job.next_run_at = Some(Utc::now());
            } else if op == "resume" {
                job.next_run_at = super::cron_expr::next_after(&job.kind, Utc::now())?;
            }
            db.save_job(
                &job,
                config.scheduler.agent_misfire_grace_secs,
                config.scheduler.reminder_misfire_grace_secs,
            )?;
            json!({"updated":job.id})
        }
        "once" | "add" => {
            anyhow::ensure!(
                args.len() >= 4,
                "usage: /cron once <RFC3339> <reminder|codex> <text>; /cron add '<cron expression>' <reminder|codex> <text>"
            );
            let a = json!({"schedule_kind":if op=="once"{"once"}else{"cron"},"schedule":args[1],"kind":args[2],"prompt":args[3..].join(" "),"timezone":config.scheduler.default_tz});
            create(db, config, user, &a)?
        }
        _ => anyhow::bail!("unknown cron command"),
    };
    Ok(serde_json::to_string_pretty(&result)?)
}
pub(crate) fn tool_specs() -> Vec<Value> {
    vec![
        json!({"type":"function","name":"schedule_create","description":"Create a reminder or isolated Codex task at the user's request. No missed-slot catch-up.","inputSchema":{"type":"object","properties":{"kind":{"type":"string","enum":["reminder","codex"]},"schedule_kind":{"type":"string","enum":["once","cron"]},"schedule":{"type":"string","description":"RFC3339 for once; six-field cron expression for cron"},"timezone":{"type":"string"},"prompt":{"type":"string"},"title":{"type":"string"}},"required":["kind","schedule_kind","schedule","prompt"],"additionalProperties":false}}),
        json!({"type":"function","name":"schedule_manage","description":"List, pause, resume, remove or run this user's scheduled task.","inputSchema":{"type":"object","properties":{"action":{"type":"string","enum":["list","pause","resume","rm","run-now","tail"]},"id":{"type":"string"}},"required":["action"],"additionalProperties":false}}),
    ]
}
pub(crate) fn call(
    db: &StateDb,
    config: &AppConfig,
    user: &str,
    name: &str,
    a: &Value,
) -> Result<Value> {
    if name == "schedule_create" {
        create(db, config, user, a)
    } else {
        let action = a["action"].as_str().unwrap_or("list");
        let id = a["id"].as_str().unwrap_or_default();
        Ok(serde_json::from_str(&command(
            db,
            config,
            user,
            &[action, id],
        )?)?)
    }
}
fn create(db: &StateDb, config: &AppConfig, user: &str, a: &Value) -> Result<Value> {
    let prompt = a["prompt"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("prompt required"))?;
    let schedule = a["schedule"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("schedule required"))?;
    let kind = match a["schedule_kind"].as_str() {
        Some("once") => CronKind::OneShot {
            at: chrono::DateTime::parse_from_rfc3339(schedule)?.with_timezone(&Utc),
        },
        Some("cron") => CronKind::Recurring {
            cron: schedule.into(),
            tz: a["timezone"]
                .as_str()
                .unwrap_or(&config.scheduler.default_tz)
                .into(),
        },
        _ => anyhow::bail!("invalid schedule kind"),
    };
    let next = match &kind {
        CronKind::OneShot { at } => Some(*at),
        _ => super::cron_expr::next_after(&kind, Utc::now())?,
    };
    let action = match a["kind"].as_str() {
        Some("reminder") => JobAction::Reminder {
            message: prompt.into(),
        },
        Some("codex") => JobAction::CodexTask {
            prompt: prompt.into(),
            model: None,
        },
        _ => anyhow::bail!("invalid job kind"),
    };
    let job = CronJob {
        id: ulid::Ulid::new().to_string(),
        user_id: user.into(),
        title: a["title"]
            .as_str()
            .unwrap_or(prompt)
            .chars()
            .take(80)
            .collect(),
        kind,
        action,
        created_at: Utc::now(),
        next_run_at: next,
        disabled: false,
    };
    db.save_job(
        &job,
        config.scheduler.agent_misfire_grace_secs,
        config.scheduler.reminder_misfire_grace_secs,
    )?;
    Ok(json!({"id":job.id,"next_run_at":job.next_run_at}))
}
