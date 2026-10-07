use super::StateDb;
use anyhow::Result;
use rusqlite::params;
use serde::Serialize;
#[derive(Serialize)]
pub struct HistoryMessage {
    pub id: i64,
    pub dialog_id: i64,
    pub role: String,
    pub content: String,
    pub created_at: i64,
}
impl StateDb {
    pub(crate) fn record_message(
        &self,
        user: &str,
        dialog: i64,
        role: &str,
        text: &str,
        platform: Option<&str>,
    ) -> Result<()> {
        self.with(|db|{db.execute("INSERT OR IGNORE INTO messages(user_id,dialog_id,role,content,platform_message_id,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![user,dialog,role,text,platform,chrono::Utc::now().timestamp()])?;Ok(())})
    }
    pub fn session_search(
        &self,
        user: &str,
        query: &str,
        dialog: Option<i64>,
        limit: usize,
    ) -> Result<Vec<HistoryMessage>> {
        self.with(|db|{
  let mut q=db.prepare("SELECT m.id,m.dialog_id,m.role,m.content,m.created_at FROM messages m WHERE m.user_id=?1 AND (?2 IS NULL OR m.dialog_id=?2) AND (m.id IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?3) OR instr(lower(m.content),lower(?4))>0) ORDER BY m.id DESC LIMIT ?5")?;
  let quoted=format!("\"{}\"",if query.is_empty(){"__empty_query__".to_owned()}else{query.replace('"',"\"\"")});
  Ok(q.query_map(params![user,dialog,quoted,query,limit.min(100) as i64],|r|Ok(HistoryMessage{id:r.get(0)?,dialog_id:r.get(1)?,role:r.get(2)?,content:r.get(3)?,created_at:r.get(4)?}))?.collect::<rusqlite::Result<_>>()?)
 })
    }
    pub fn session_get(
        &self,
        user: &str,
        dialog: i64,
        from: i64,
        to: i64,
    ) -> Result<Vec<HistoryMessage>> {
        self.with(|db|{let mut q=db.prepare("SELECT id,dialog_id,role,content,created_at FROM messages WHERE user_id=?1 AND dialog_id=?2 AND id BETWEEN ?3 AND ?4 ORDER BY id LIMIT 200")?;Ok(q.query_map(params![user,dialog,from,to],|r|Ok(HistoryMessage{id:r.get(0)?,dialog_id:r.get(1)?,role:r.get(2)?,content:r.get(3)?,created_at:r.get(4)?}))?.collect::<rusqlite::Result<_>>()?)})
    }
}
