use super::StateDb;
use crate::model::cron::{CronJob, CronKind, JobAction};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};
pub(crate) struct ClaimedRun {
    pub id: i64,
    pub scheduled_at: i64,
    pub job: CronJob,
    pub grace: i64,
}
impl StateDb {
    pub(crate) fn save_job(
        &self,
        job: &CronJob,
        agent_grace: i64,
        reminder_grace: i64,
    ) -> Result<()> {
        self.with(|db| save_job(db, job, agent_grace, reminder_grace))
    }
    pub(crate) fn jobs_for(&self, user: &str) -> Result<Vec<CronJob>> {
        self.with(|db|{let mut q=db.prepare("SELECT job_json,enabled,next_run_at FROM scheduled_jobs WHERE user_id=?1 ORDER BY created_at,id")?;let mut rows=q.query([user])?;let mut jobs=Vec::new();while let Some(r)=rows.next()?{jobs.push(read_job(r)?);}Ok(jobs)})
    }
    pub(crate) fn job_for(&self, user: &str, id: &str) -> Result<Option<CronJob>> {
        self.with(|db|{let mut q=db.prepare("SELECT job_json,enabled,next_run_at FROM scheduled_jobs WHERE user_id=?1 AND id=?2")?;let mut rows=q.query(params![user,id])?;if let Some(r)=rows.next()?{Ok(Some(read_job(r)?))}else{Ok(None)}})
    }
    pub(crate) fn remove_job(&self, user: &str, id: &str) -> Result<bool> {
        self.with(|db| {
            Ok(db.execute(
                "DELETE FROM scheduled_jobs WHERE user_id=?1 AND id=?2",
                params![user, id],
            )? > 0)
        })
    }
    pub(crate) fn claim_due_jobs(&self) -> Result<()> {
        self.with(|db|{let now=Utc::now();let tx=db.transaction()?;let jobs={let mut q=tx.prepare("SELECT job_json,enabled,next_run_at,grace_secs FROM scheduled_jobs WHERE enabled=1 AND next_run_at<=?1")?;let mut rows=q.query([now.timestamp()])?;let mut v=Vec::new();while let Some(r)=rows.next()?{v.push((read_job(r)?,r.get::<_,i64>(3)?));}v};for (job,grace)in jobs{let scheduled=job.next_run_at.unwrap().timestamp();let state=if now.timestamp()-scheduled>grace{"missed"}else{"claimed"};tx.execute("INSERT OR IGNORE INTO scheduled_runs(job_id,scheduled_at,state,claimed_at,finished_at) VALUES(?1,?2,?3,?4,?5)",params![job.id,scheduled,state,now.timestamp(),if state=="missed"{Some(now.timestamp())}else{None}])?;let next=crate::scheduler::cron_expr::next_after(&job.kind,now)?.map(|t|t.timestamp());tx.execute("UPDATE scheduled_jobs SET next_run_at=?2,updated_at=?3 WHERE id=?1",params![job.id,next,now.timestamp()])?;}tx.commit()?;Ok(())})
    }
    pub(crate) fn claimed_jobs(&self) -> Result<Vec<ClaimedRun>> {
        self.with(|db|{let mut q=db.prepare("SELECT r.id,r.scheduled_at,j.job_json,j.grace_secs FROM scheduled_runs r JOIN scheduled_jobs j ON r.job_id=j.id WHERE r.state='claimed' ORDER BY r.scheduled_at LIMIT 64")?;let rows=q.query_map([],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?)))?;let mut v=Vec::new();for row in rows{let(id,scheduled_at,raw,grace)=row?;v.push(ClaimedRun{id,scheduled_at,job:serde_json::from_str(&raw)?,grace});}Ok(v)})
    }
    pub(crate) fn mark_run(
        &self,
        user: &str,
        id: i64,
        state: &str,
        error: Option<&str>,
    ) -> Result<()> {
        self.with(|db|{db.execute("UPDATE scheduled_runs SET state=?3,started_at=CASE WHEN ?3='running' THEN ?4 ELSE started_at END,finished_at=CASE WHEN ?3 IN ('success','failed','missed') THEN ?4 ELSE finished_at END,last_error=?5 WHERE id=?2 AND job_id IN (SELECT id FROM scheduled_jobs WHERE user_id=?1)",params![user,id,state,Utc::now().timestamp(),error])?;Ok(())})
    }
    pub(crate) fn run_history(&self, user: &str, job: &str) -> Result<Vec<String>> {
        self.with(|db|{let mut q=db.prepare("SELECT r.scheduled_at,r.state,coalesce(r.last_error,'') FROM scheduled_runs r JOIN scheduled_jobs j ON j.id=r.job_id WHERE j.user_id=?1 AND j.id=?2 ORDER BY r.id DESC LIMIT 20")?;Ok(q.query_map(params![user,job],|r|Ok(format!("{} {} {}",r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<_>>()?)})
    }
}
fn read_job(r: &rusqlite::Row) -> Result<CronJob> {
    let mut j: CronJob = serde_json::from_str(&r.get::<_, String>(0)?)?;
    j.disabled = r.get::<_, i64>(1)? == 0;
    j.next_run_at = r
        .get::<_, Option<i64>>(2)?
        .and_then(|v| DateTime::from_timestamp(v, 0));
    Ok(j)
}
pub(crate) fn save_job(
    db: &rusqlite::Connection,
    j: &CronJob,
    agent: i64,
    reminder: i64,
) -> Result<()> {
    let (kind, prompt, grace) = match &j.action {
        JobAction::Reminder { message } => ("reminder", message.as_str(), reminder),
        JobAction::CodexTask { prompt, .. } => ("codex", prompt.as_str(), agent),
    };
    let (schedule, cron, tz, at) = match &j.kind {
        CronKind::OneShot { at } => ("once", None, "UTC", Some(at.timestamp())),
        CronKind::Recurring { cron, tz } => ("cron", Some(cron.as_str()), tz.as_str(), None),
    };
    let existing: Option<String> = db
        .query_row(
            "SELECT user_id FROM scheduled_jobs WHERE id=?1",
            [&j.id],
            |r| r.get(0),
        )
        .optional()?;
    anyhow::ensure!(
        existing.as_deref().is_none_or(|u| u == j.user_id),
        "job belongs to another user"
    );
    db.execute("INSERT INTO scheduled_jobs(id,user_id,kind,schedule_kind,cron_expr,timezone,run_at,prompt,enabled,next_run_at,grace_secs,created_at,updated_at,job_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14) ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled,next_run_at=excluded.next_run_at,prompt=excluded.prompt,updated_at=excluded.updated_at,job_json=excluded.job_json",params![j.id,j.user_id,kind,schedule,cron,tz,at,prompt,!j.disabled,j.next_run_at.map(|t|t.timestamp()),grace,j.created_at.timestamp(),Utc::now().timestamp(),serde_json::to_string(j)?])?;
    Ok(())
}
