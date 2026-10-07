use crate::state::StateDb;
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
#[derive(Clone)]
pub struct MemoryStore {
    pub(crate) db: StateDb,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: i64,
    pub kind: String,
    pub scope: String,
    pub content: String,
    pub tags: String,
    pub importance: i64,
    pub status: String,
    pub supersedes_id: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryOperation {
    pub op: String,
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default = "project")]
    pub kind: String,
    #[serde(default = "global")]
    pub scope: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "importance")]
    pub importance: i64,
}
fn project() -> String {
    "project".into()
}
fn global() -> String {
    "global".into()
}
fn importance() -> i64 {
    3
}
impl MemoryStore {
    pub fn new(db: StateDb) -> Self {
        Self { db }
    }
    pub fn profile(&self, user: &str) -> Result<(String, i64)> {
        self.db.with(|db| {
            Ok(db
                .query_row(
                    "SELECT profile_text,version FROM user_profiles WHERE user_id=?1",
                    [user],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?
                .unwrap_or_default())
        })
    }
    pub fn set_profile(&self, user: &str, text: &str) -> Result<()> {
        anyhow::ensure!(
            text.chars().count() <= 4000,
            "profile exceeds 4000 characters"
        );
        self.db.with(|db|{db.execute("INSERT INTO user_profiles(user_id,profile_text,updated_at) VALUES(?1,?2,?3) ON CONFLICT(user_id) DO UPDATE SET profile_text=excluded.profile_text,version=version+1,updated_at=excluded.updated_at",params![user,text,chrono::Utc::now().timestamp()])?;Ok(())})
    }
    pub fn get(&self, user: &str, id: i64) -> Result<Option<Memory>> {
        self.db.with(|db|Ok(db.query_row("SELECT id,kind,scope,content,tags,importance,status,supersedes_id FROM memories WHERE user_id=?1 AND id=?2",params![user,id],read_memory).optional()?))
    }
    pub fn search(
        &self,
        user: &str,
        query: &str,
        scope: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Memory>> {
        self.db.with(|db| {
            let mut q = db.prepare(
                "SELECT id,kind,scope,content,tags,importance,status,supersedes_id \
                 FROM memories WHERE user_id=?1 AND status='active' AND (?2 IS NULL OR scope=?2) \
                 ORDER BY importance DESC,updated_at DESC LIMIT 1000",
            )?;
            let candidates = q
                .query_map(params![user, scope], read_memory)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(q);

            let limit = limit.min(100);
            let result = if query.trim().is_empty() {
                candidates.into_iter().take(limit).collect::<Vec<_>>()
            } else {
                let normalized = query.to_lowercase();
                let terms = retrieval_terms(query);
                let mut scored = candidates
                    .into_iter()
                    .enumerate()
                    .filter_map(|(order, memory)| {
                        retrieval_score(&memory, &normalized, &terms)
                            .map(|score| (score, order, memory))
                    })
                    .collect::<Vec<_>>();
                scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
                scored
                    .into_iter()
                    .take(limit)
                    .map(|(_, _, memory)| memory)
                    .collect()
            };
            let now = chrono::Utc::now().timestamp();
            for memory in &result {
                db.execute(
                    "UPDATE memories SET last_used_at=?3 WHERE user_id=?1 AND id=?2",
                    params![user, memory.id, now],
                )?;
            }
            Ok(result)
        })
    }
    pub fn hot(&self, user: &str, max: usize) -> Result<String> {
        self.db.with(|db|{let mut q=db.prepare("SELECT content FROM memories WHERE user_id=?1 AND status='active' AND scope='global' AND importance>=4 AND kind IN ('preference','environment','relationship','profile') ORDER BY importance DESC,updated_at DESC LIMIT 20")?;let mut text=String::new();for r in q.query_map([user],|r|r.get::<_,String>(0))?{let line=r?;if text.chars().count()+line.chars().count()+1>max{break}text.push_str(&line);text.push('\n');}Ok(text)})
    }
    pub fn apply(&self, user: &str, operations: &[MemoryOperation]) -> Result<Vec<i64>> {
        self.db.with(|db| {
            let tx = db.transaction()?;
            let mut ids = Vec::new();
            for op in operations {
                ids.push(apply_one(&tx, user, op)?);
            }
            tx.commit()?;
            Ok(ids)
        })
    }
    pub(crate) fn completed_turn(&self, user: &str) -> Result<()> {
        self.db.with(|db|{db.execute("INSERT INTO memory_cursors(user_id,completed_turns,last_turn_at) VALUES(?1,1,?2) ON CONFLICT(user_id) DO UPDATE SET completed_turns=completed_turns+1,last_turn_at=excluded.last_turn_at",params![user,chrono::Utc::now().timestamp()])?;Ok(())})
    }
    pub(crate) fn due_users(&self, turns: i64, idle: i64) -> Result<Vec<String>> {
        let now = chrono::Utc::now().timestamp();
        self.db.with(|db|{let mut q=db.prepare("SELECT user_id FROM memory_cursors WHERE pending=0 AND completed_turns>0 AND next_distill_at<=?3 AND (completed_turns>=?1 OR last_turn_at<=?2)")?;Ok(q.query_map(params![turns,now-idle,now],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?)})
    }
    pub(crate) fn claim_distill(&self, user: &str) -> Result<bool> {
        let now = chrono::Utc::now().timestamp();
        self.db.with(|db|Ok(db.execute("UPDATE memory_cursors SET pending=1 WHERE user_id=?1 AND pending=0 AND completed_turns>0 AND next_distill_at<=?2",params![user,now])?>0))
    }
    pub(crate) fn transcript(&self, user: &str) -> Result<(String, i64, i64)> {
        self.db.with(|db| {
            let last: i64 = db.query_row(
                "SELECT last_message_id FROM memory_cursors WHERE user_id=?1",
                [user],
                |r| r.get(0),
            )?;
            let mut q = db.prepare("SELECT id,role,content FROM messages WHERE user_id=?1 AND dialog_id<>0 AND id>?2 ORDER BY id LIMIT 500")?;
            let rows = q.query_map(params![user, last], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
            let mut text = String::new();
            let mut end = last;
            let mut remaining = 60000usize;
            for row in rows {
                let (id, role, content) = row?;
                if remaining == 0 { break; }
                let excerpt: String = content.chars().take(remaining).collect();
                remaining = remaining.saturating_sub(excerpt.chars().count() + role.chars().count() + 3);
                text.push_str(&format!("{role}: {excerpt}\n"));
                end = id;
            }
            let turns = db.query_row(
                "SELECT count(*) FROM messages a WHERE a.user_id=?1 AND a.id>?2 AND a.id<=?3 AND a.role='assistant' AND a.platform_message_id IS NOT NULL AND EXISTS(SELECT 1 FROM messages u WHERE u.user_id=a.user_id AND u.role='user' AND u.platform_message_id=a.platform_message_id)",
                params![user, last, end],
                |r| r.get(0),
            )?;
            Ok((text, end, turns))
        })
    }

    pub(crate) fn commit_distill(
        &self,
        user: &str,
        operations: &[MemoryOperation],
        end: i64,
        turns: i64,
    ) -> Result<()> {
        self.db.with(|db|{let tx=db.transaction()?;for op in operations{apply_one(&tx,user,op)?;}tx.execute("UPDATE memory_cursors SET last_message_id=?2,completed_turns=max(0,completed_turns-?3),pending=0,distill_failures=0,next_distill_at=0 WHERE user_id=?1",params![user,end,turns])?;tx.commit()?;Ok(())})
    }
    pub(crate) fn defer_distill(&self, user: &str, seconds: i64) -> Result<()> {
        self.db.with(|db| {
            db.execute(
                "UPDATE memory_cursors SET pending=0,next_distill_at=?2 WHERE user_id=?1",
                params![user, chrono::Utc::now().timestamp() + seconds.max(1)],
            )?;
            Ok(())
        })
    }
    pub(crate) fn fail_distill(&self, user: &str) -> Result<i64> {
        self.db.with(|db| {
            let tx = db.transaction()?;
            let failures: i64 = tx.query_row(
                "SELECT distill_failures FROM memory_cursors WHERE user_id=?1",
                [user],
                |r| r.get(0),
            )?;
            let shift = failures.clamp(0, 7) as u32;
            let delay = (30_i64.saturating_mul(1_i64 << shift)).min(3600);
            tx.execute(
                "UPDATE memory_cursors SET pending=0,distill_failures=distill_failures+1,next_distill_at=?2 WHERE user_id=?1",
                params![user, chrono::Utc::now().timestamp() + delay],
            )?;
            tx.commit()?;
            Ok(delay)
        })
    }
    pub(crate) fn release_distill(&self, user: &str) -> Result<()> {
        self.db.with(|db| {
            db.execute(
                "UPDATE memory_cursors SET pending=0 WHERE user_id=?1",
                [user],
            )?;
            Ok(())
        })
    }
}
fn read_memory(r: &rusqlite::Row) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: r.get(0)?,
        kind: r.get(1)?,
        scope: r.get(2)?,
        content: r.get(3)?,
        tags: r.get(4)?,
        importance: r.get(5)?,
        status: r.get(6)?,
        supersedes_id: r.get(7)?,
    })
}

fn retrieval_terms(query: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "the", "and", "are", "was", "were", "what", "when", "where", "which", "who",
        "why", "how", "does", "did", "this", "that", "with", "from", "have", "has",
        "had", "your", "you", "our", "user",
    ];
    let mut terms = Vec::new();
    let normalized = query.to_lowercase();
    for token in normalized
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|token| !token.is_empty())
    {
        let chars = token.chars().collect::<Vec<_>>();
        if chars.iter().all(|c| c.is_ascii()) {
            if chars.len() >= 3 && !STOP.contains(&token) {
                terms.push(token.to_owned());
            }
        } else if chars.len() <= 3 {
            terms.push(token.to_owned());
        } else {
            for window in chars.windows(3) {
                terms.push(window.iter().collect());
            }
        }
    }
    terms.sort();
    terms.dedup();
    if terms.len() > 48 {
        let len = terms.len();
        terms = (0..48)
            .map(|i| terms[i * len / 48].clone())
            .collect();
        terms.sort();
        terms.dedup();
    }
    terms
}

fn retrieval_score(memory: &Memory, normalized_query: &str, terms: &[String]) -> Option<i64> {
    let haystack = format!("{} {}", memory.content, memory.tags).to_lowercase();
    let exact = !normalized_query.is_empty() && haystack.contains(normalized_query);
    let matched = terms
        .iter()
        .filter(|term| haystack.contains(term.as_str()))
        .count() as i64;
    if !exact && matched == 0 {
        return None;
    }
    Some((if exact { 1000 } else { 0 }) + matched * 20 + memory.importance * 3)
}

fn apply_one(db: &rusqlite::Connection, user: &str, op: &MemoryOperation) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    if op.op == "delete" {
        let id = op.id.ok_or_else(|| anyhow::anyhow!("memory id required"))?;
        anyhow::ensure!(db.execute("UPDATE memories SET status='deleted',updated_at=?3 WHERE user_id=?1 AND id=?2 AND status='active'",params![user,id,now])?==1,"active memory not found for owner");
        return Ok(id);
    }
    anyhow::ensure!(
        matches!(op.op.as_str(), "add" | "update" | "supersede"),
        "unknown memory operation"
    );
    let content = op.content.trim();
    anyhow::ensure!(
        !content.is_empty() && content.chars().count() <= 300,
        "memory must contain 1–300 characters"
    );
    anyhow::ensure!(
        matches!(
            op.kind.as_str(),
            "profile"
                | "preference"
                | "environment"
                | "project"
                | "decision"
                | "correction"
                | "relationship"
        ),
        "invalid memory kind"
    );
    anyhow::ensure!(
        op.scope == "global"
            || (op.scope.starts_with("project:") && op.scope.len() > "project:".len()),
        "invalid memory scope"
    );
    anyhow::ensure!(op.scope.chars().count() <= 120, "memory scope is too long");
    anyhow::ensure!(
        op.tags.len() <= 8
            && op
                .tags
                .iter()
                .all(|tag| !tag.trim().is_empty() && tag.chars().count() <= 32),
        "memory tags must contain at most 8 non-empty tags of at most 32 characters"
    );
    anyhow::ensure!((1..=5).contains(&op.importance), "importance must be 1–5");
    if op.op == "add" {
        if let Some(id) = db
            .query_row(
                "SELECT id FROM memories WHERE user_id=?1 AND kind=?2 AND scope=?3 AND status='active' AND content=?4 COLLATE NOCASE LIMIT 1",
                params![user, op.kind, op.scope, content],
                |r| r.get(0),
            )
            .optional()?
        {
            return Ok(id);
        }
    }
    let supersedes = if op.op != "add" {
        let id = op.id.ok_or_else(|| anyhow::anyhow!("memory id required"))?;
        anyhow::ensure!(db.execute("UPDATE memories SET status='superseded',updated_at=?3 WHERE user_id=?1 AND id=?2 AND status='active'",params![user,id,now])?==1,"active memory not found for owner");
        Some(id)
    } else {
        None
    };
    let tags = op
        .tags
        .iter()
        .map(|tag| tag.trim())
        .collect::<Vec<_>>()
        .join(" ");
    db.execute("INSERT INTO memories(user_id,kind,scope,content,tags,importance,supersedes_id,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8)",params![user,op.kind,op.scope,content,tags,op.importance,supersedes,now])?;
    Ok(db.last_insert_rowid())
}
