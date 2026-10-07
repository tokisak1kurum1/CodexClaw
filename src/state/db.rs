use anyhow::{Context, Result};
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Clone)]
pub struct StateDb {
    inner: Arc<Mutex<Connection>>,
    pub path: PathBuf,
}
impl StateDb {
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join("state.db");
        let db = Connection::open(&path)?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;",
        )?;
        db.execute_batch(super::schema::SCHEMA)?;
        ensure_memory_cursor_columns(&db)?;
        // Trigram supports Chinese substrings; standard FTS5 plus LIKE is the fallback.
        if db.execute_batch("CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(content,content='messages',content_rowid='id',tokenize='trigram');").is_err(){
   db.execute_batch("CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(content,content='messages',content_rowid='id');")?;
  }
        db.execute_batch("CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN INSERT INTO messages_fts(rowid,content) VALUES(new.id,new.content); END; CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN INSERT INTO messages_fts(messages_fts,rowid,content) VALUES('delete',old.id,old.content); END; CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN INSERT INTO messages_fts(messages_fts,rowid,content) VALUES('delete',old.id,old.content); INSERT INTO messages_fts(rowid,content) VALUES(new.id,new.content); END;")?;
        Ok(Self {
            inner: Arc::new(Mutex::new(db)),
            path,
        })
    }
    pub fn with<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        {
            let mut db = self
                .inner
                .lock()
                .map_err(|_| anyhow::anyhow!("state lock poisoned"))?;
            f(&mut db)
        }
    }
    pub fn recover(&self) -> Result<()> {
        self.with(|db|{db.execute_batch("UPDATE inbox SET state='pending',started_at=NULL WHERE state='running'; UPDATE outbox SET state='pending',last_error='restart during delivery; possible duplicate' WHERE state='sending'; UPDATE scheduled_runs SET state='failed',finished_at=strftime('%s','now'),last_error='restart during scheduled execution; not replayed' WHERE state='running'; UPDATE memory_cursors SET pending=0;").context("recover durable state")?;Ok(())})
    }
}

fn ensure_memory_cursor_columns(db: &Connection) -> Result<()> {
    for (column, ddl) in [
        (
            "distill_failures",
            "ALTER TABLE memory_cursors ADD COLUMN distill_failures INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "next_distill_at",
            "ALTER TABLE memory_cursors ADD COLUMN next_distill_at INTEGER NOT NULL DEFAULT 0",
        ),
    ] {
        let exists = {
            let mut q = db.prepare("PRAGMA table_info(memory_cursors)")?;
            let rows = q.query_map([], |r| r.get::<_, String>(1))?;
            let mut found = false;
            for row in rows {
                if row? == column {
                    found = true;
                    break;
                }
            }
            found
        };
        if !exists {
            db.execute(ddl, [])?;
        }
    }
    Ok(())
}
