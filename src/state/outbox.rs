use super::StateDb;
use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
pub struct Delivery {
    pub text: String,
    pub directives: Vec<crate::qq::Directive>,
}
pub struct OutboxRow {
    pub id: i64,
    pub user: String,
    pub reply_to: Option<String>,
    pub payload: Delivery,
}
impl StateDb {
    pub fn enqueue_delivery(
        &self,
        user: &str,
        reply: Option<&str>,
        key: &str,
        payload: &Delivery,
    ) -> Result<()> {
        self.with(|db|{db.execute("INSERT OR IGNORE INTO outbox(user_id,reply_to_message_id,payload_json,created_at,logical_key) VALUES(?1,?2,?3,?4,?5)",params![user,reply,serde_json::to_string(payload)?,chrono::Utc::now().timestamp(),key])?;Ok(())})
    }
    pub fn next_delivery(&self) -> Result<Option<OutboxRow>> {
        self.with(|db|{let tx=db.transaction()?;let row={let mut q=tx.prepare("SELECT id,user_id,reply_to_message_id,payload_json FROM outbox WHERE state='pending' AND next_attempt_at<=?1 ORDER BY id LIMIT 1")?;let mut rows=q.query([chrono::Utc::now().timestamp()])?;if let Some(r)=rows.next()?{Some(OutboxRow{id:r.get(0)?,user:r.get(1)?,reply_to:r.get(2)?,payload:serde_json::from_str(&r.get::<_,String>(3)?)?})}else{None}};if let Some(r)=&row{tx.execute("UPDATE outbox SET state='sending',attempts=attempts+1 WHERE id=?1",[r.id])?;}tx.commit()?;Ok(row)})
    }
    pub fn finish_delivery(&self, user: &str, id: i64, error: Option<&str>) -> Result<()> {
        self.with(|db|{if let Some(error)=error{db.execute("UPDATE outbox SET state='pending',last_error=?3,next_attempt_at=?4 WHERE user_id=?1 AND id=?2",params![user,id,error,chrono::Utc::now().timestamp()+30])?;}else{db.execute("UPDATE outbox SET state='delivered',delivered_at=?3 WHERE user_id=?1 AND id=?2",params![user,id,chrono::Utc::now().timestamp()])?;}Ok(())})
    }
}
impl StateDb {
    pub(crate) fn commit_answer(
        &self,
        user: &str,
        dialog: i64,
        ids: &[String],
        reply: &str,
        payload: &Delivery,
    ) -> Result<()> {
        self.with(|db|{let tx=db.transaction()?;let now=chrono::Utc::now().timestamp();tx.execute("INSERT OR IGNORE INTO outbox(user_id,reply_to_message_id,payload_json,created_at,logical_key) VALUES(?1,?2,?3,?4,?5)",params![user,reply,serde_json::to_string(payload)?,now,format!("inbound:{}",ids[0])])?;tx.execute("INSERT OR IGNORE INTO messages(user_id,dialog_id,role,content,platform_message_id,created_at) VALUES(?1,?2,'assistant',?3,?4,?5)",params![user,dialog,payload.text,reply,now])?;tx.execute("DELETE FROM turn_retries WHERE user_id=?1 AND message_id=?2",params![user,reply])?;for id in ids{tx.execute("UPDATE inbox SET state='done',finished_at=?3 WHERE user_id=?1 AND message_id=?2",params![user,id,now])?;}tx.commit()?;Ok(())})
    }
}

impl StateDb {
    pub(crate) fn commit_scheduled_answer(
        &self,
        user: &str,
        run: i64,
        key: &str,
        payload: &Delivery,
    ) -> Result<()> {
        self.with(|db|{let tx=db.transaction()?;let now=chrono::Utc::now().timestamp();let owned:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM scheduled_runs r JOIN scheduled_jobs j ON j.id=r.job_id WHERE r.id=?1 AND j.user_id=?2)",params![run,user],|r|r.get(0))?;anyhow::ensure!(owned,"scheduled run not owned");tx.execute("INSERT OR IGNORE INTO outbox(user_id,payload_json,created_at,logical_key) VALUES(?1,?2,?3,?4)",params![user,serde_json::to_string(payload)?,now,key])?;tx.execute("INSERT OR IGNORE INTO messages(user_id,dialog_id,role,content,platform_message_id,created_at) VALUES(?1,0,'assistant',?2,?3,?4)",params![user,payload.text,key,now])?;tx.execute("UPDATE scheduled_runs SET state='success',finished_at=?2 WHERE id=?1",params![run,now])?;tx.commit()?;Ok(())})
    }
}

impl StateDb {
    pub(crate) fn commit_turn_failure(
        &self,
        user: &str,
        ids: &[String],
        reply: &str,
        error: &str,
    ) -> Result<()> {
        self.with(|db|{let tx=db.transaction()?;let now=chrono::Utc::now().timestamp();let attempt:i64=tx.query_row("SELECT attempts FROM inbox WHERE user_id=?1 AND message_id=?2",params![user,ids[0]],|r|r.get(0))?;let payload=Delivery{text:format!("{error}\n/retry · /重试"),directives:Vec::new()};tx.execute("INSERT OR IGNORE INTO outbox(user_id,reply_to_message_id,payload_json,created_at,logical_key) VALUES(?1,?2,?3,?4,?5)",params![user,reply,serde_json::to_string(&payload)?,now,format!("failure:{}:{attempt}",ids[0])])?;for id in ids{tx.execute("UPDATE inbox SET state='failed',finished_at=?3,last_error=?4 WHERE user_id=?1 AND message_id=?2",params![user,id,now,error])?;}tx.execute("INSERT INTO turn_retries(user_id,message_id,ids_json,created_at) VALUES(?1,?2,?3,?4) ON CONFLICT(user_id) DO UPDATE SET message_id=excluded.message_id,ids_json=excluded.ids_json,created_at=excluded.created_at",params![user,reply,serde_json::to_string(ids)?,now])?;tx.commit()?;Ok(())})
    }
    pub(crate) fn retry_turn(&self, user: &str) -> Result<bool> {
        self.with(|db|{use rusqlite::OptionalExtension;let tx=db.transaction()?;let raw:Option<String>=tx.query_row("SELECT ids_json FROM turn_retries WHERE user_id=?1",[user],|r|r.get(0)).optional()?;let Some(raw)=raw else{return Ok(false)};let ids:Vec<String>=serde_json::from_str(&raw)?;for id in ids{anyhow::ensure!(tx.execute("UPDATE inbox SET state='pending',started_at=NULL,finished_at=NULL,last_error=NULL WHERE user_id=?1 AND message_id=?2 AND state='failed'",params![user,id])?==1,"retry already queued or request is no longer retryable");}tx.execute("DELETE FROM turn_retries WHERE user_id=?1",[user])?;tx.commit()?;Ok(true)})
    }
}
