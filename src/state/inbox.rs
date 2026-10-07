use super::StateDb;
use crate::qq::C2CMessageEvent;
use anyhow::Result;
use rusqlite::params;
#[derive(Clone)]
pub struct InboxRow {
    pub id: String,
    pub user: String,
    pub received_at: i64,
    pub payload: String,
}
impl StateDb {
    pub fn accept_event(&self, event: &C2CMessageEvent) -> Result<bool> {
        self.with(|db|Ok(db.execute("INSERT OR IGNORE INTO inbox(message_id,user_id,received_at,payload_json) VALUES(?1,?2,?3,?4)",params![event.id,event.author.user_openid,chrono::Utc::now().timestamp_millis(),serde_json::to_string(event)?])?>0))
    }
    pub fn pending_inbox(&self, limit: usize) -> Result<Vec<InboxRow>> {
        self.with(|db|{let mut q=db.prepare("WITH queued AS (SELECT message_id,user_id,received_at,payload_json,rowid AS seq,CASE WHEN ltrim(json_extract(payload_json,'$.content')) LIKE '/%' THEN 0 ELSE 1 END AS priority,row_number() OVER(PARTITION BY user_id ORDER BY CASE WHEN ltrim(json_extract(payload_json,'$.content')) LIKE '/%' THEN 0 ELSE 1 END,received_at,rowid) AS owner_rank FROM inbox WHERE state='pending') SELECT message_id,user_id,received_at,payload_json FROM queued WHERE owner_rank<=64 ORDER BY priority,received_at,seq LIMIT ?1")?;Ok(q.query_map([limit as i64],|r|Ok(InboxRow{id:r.get(0)?,user:r.get(1)?,received_at:r.get(2)?,payload:r.get(3)?}))?.collect::<rusqlite::Result<_>>()?)})
    }
    pub fn claim_inbox(&self, user: &str, ids: &[String]) -> Result<()> {
        self.with(|db|{let tx=db.transaction()?;for id in ids{anyhow::ensure!(tx.execute("UPDATE inbox SET state='running',attempts=attempts+1,started_at=?3 WHERE user_id=?1 AND message_id=?2 AND state='pending'",params![user,id,chrono::Utc::now().timestamp()])?==1,"inbox entry is not pending for owner");}tx.commit()?;Ok(())})
    }
    pub fn finish_inbox(&self, user: &str, ids: &[String], error: Option<&str>) -> Result<()> {
        self.with(|db|{let tx=db.transaction()?;for id in ids{tx.execute("UPDATE inbox SET state=?3,finished_at=?4,last_error=?5 WHERE user_id=?1 AND message_id=?2",params![user,id,if error.is_some(){"failed"}else{"done"},chrono::Utc::now().timestamp(),error])?;}tx.commit()?;Ok(())})
    }
    pub fn reset_inbox(&self, user: &str, ids: &[String]) -> Result<()> {
        self.with(|db|{for id in ids{db.execute("UPDATE inbox SET state='pending',started_at=NULL WHERE user_id=?1 AND message_id=?2",params![user,id])?;}Ok(())})
    }
}

impl StateDb {
    pub(crate) fn retained_attachment_prefixes(
        &self,
    ) -> Result<std::collections::HashSet<(String, String)>> {
        self.with(|db|{let mut q=db.prepare("SELECT user_id,message_id FROM inbox WHERE state IN ('pending','running','failed')")?;let rows=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?;let mut prefixes=std::collections::HashSet::new();for row in rows{let(user,id)=row?;prefixes.insert((format!("{:x}",md5::compute(user)),format!("{:x}_",md5::compute(id))));}Ok(prefixes)})
    }
}
