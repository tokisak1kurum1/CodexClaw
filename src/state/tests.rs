use super::*;
use crate::{
    config::AppConfig,
    memory::{MemoryStore, store::MemoryOperation},
    model::cron::{CronJob, CronKind, JobAction},
};
use rusqlite::params;
fn database() -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path()).unwrap();
    (dir, db)
}
fn inbox(db: &StateDb, user: &str, id: &str) {
    db.with(|sql| {
        sql.execute(
            "INSERT INTO inbox(message_id,user_id,received_at,payload_json) VALUES(?1,?2,?3,?4)",
            params![
                id,
                user,
                chrono::Utc::now().timestamp_millis(),
                r#"{"content":"hello"}"#
            ],
        )?;
        Ok(())
    })
    .unwrap();
}
#[test]
fn retry_survives_restart_and_success_stops_model_replay() {
    let (dir, db) = database();
    inbox(&db, "a", "m");
    db.claim_inbox("a", &["m".into()]).unwrap();
    db.commit_turn_failure("a", &["m".into()], "m", "disconnect")
        .unwrap();
    drop(db);
    let db = StateDb::open(dir.path()).unwrap();
    db.recover().unwrap();
    assert!(!db.retry_turn("b").unwrap());
    assert!(db.retry_turn("a").unwrap());
    assert!(!db.retry_turn("a").unwrap());
    db.claim_inbox("a", &["m".into()]).unwrap();
    db.commit_answer(
        "a",
        0,
        &["m".into()],
        "m",
        &outbox::Delivery {
            text: "answer".into(),
            directives: vec![],
        },
    )
    .unwrap();
    db.recover().unwrap();
    assert!(db.pending_inbox(10).unwrap().is_empty());
    assert!(!db.retry_turn("a").unwrap());
    let deliveries: i64 = db
        .with(|sql| Ok(sql.query_row("SELECT count(*) FROM outbox", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(deliveries, 2);
}
#[test]
fn sqlite_recovers_queues_and_protects_attachment_references() {
    let (_dir, db) = database();
    inbox(&db, "a", "m");
    assert!(db.claim_inbox("b", &["m".into()]).is_err());
    db.claim_inbox("a", &["m".into()]).unwrap();
    db.enqueue_delivery(
        "a",
        Some("m"),
        "key",
        &outbox::Delivery {
            text: "hello".into(),
            directives: vec![],
        },
    )
    .unwrap();
    let _sending = db.next_delivery().unwrap().unwrap();
    db.recover().unwrap();
    assert_eq!(db.pending_inbox(10).unwrap().len(), 1);
    assert!(db.next_delivery().unwrap().is_none());
    assert_eq!(
        db.with(|sql| Ok(sql.query_row("SELECT state FROM outbox WHERE logical_key='key'", [], |r| r.get::<_, String>(0))?)).unwrap(),
        "paused"
    );
    assert!(db.retained_attachment_prefixes().unwrap().contains(&(
        format!("{:x}", md5::compute("a")),
        format!("{:x}_", md5::compute("m"))
    )));
    let mode: String = db
        .with(|s| Ok(s.query_row("PRAGMA journal_mode", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(mode, "wal");
}
#[test]
fn memories_are_owner_bound_and_superseding_is_atomic() {
    let (_dir, db) = database();
    let store = MemoryStore::new(db);
    let op = MemoryOperation {
        op: "add".into(),
        id: None,
        kind: "preference".into(),
        scope: "global".into(),
        content: "Use PowerShell".into(),
        tags: vec!["shell".into()],
        importance: 5,
    };
    let id = store.apply("a", &[op.clone()]).unwrap()[0];
    assert_eq!(store.apply("a", &[op.clone()]).unwrap()[0], id);
    let active_count: i64 = store
        .db
        .with(|sql| {
            Ok(sql.query_row(
                "SELECT count(*) FROM memories WHERE user_id='a' AND status='active'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(active_count, 1);
    assert!(store.get("b", id).unwrap().is_none());
    assert!(store.search("b", "PowerShell", None, 8).unwrap().is_empty());
    assert_eq!(store.search("a", "PowerShell", None, 8).unwrap().len(), 1);
    let mut update = op.clone();
    update.op = "supersede".into();
    update.id = Some(id);
    update.content = "Use bash".into();
    let mut invalid = op.clone();
    invalid.content = "x".repeat(301);
    assert!(store.apply("a", &[update.clone(), invalid]).is_err());
    assert_eq!(store.get("a", id).unwrap().unwrap().status, "active");
    let version = store.profile("a").unwrap().1;
    let new = store.apply("a", &[update]).unwrap()[0];
    assert_eq!(store.get("a", id).unwrap().unwrap().status, "superseded");
    assert_eq!(
        store.get("a", new).unwrap().unwrap().supersedes_id,
        Some(id)
    );
    assert_eq!(store.profile("a").unwrap().1, version);
    let mut delete = op;
    delete.op = "delete".into();
    delete.id = Some(new);
    assert!(store.apply("b", &[delete.clone()]).is_err());
    store.apply("a", &[delete]).unwrap();
    assert!(store.search("a", "bash", None, 8).unwrap().is_empty());
}
#[test]
fn memory_retrieval_matches_natural_language_and_chinese_substrings() {
    let (_dir, db) = database();
    let store = MemoryStore::new(db);
    for content in [
        "User prefers PowerShell",
        "用户以后不要自动 git commit",
        "数据库迁移使用 SQLAlchemy",
    ] {
        store
            .apply(
                "a",
                &[MemoryOperation {
                    op: "add".into(),
                    id: None,
                    kind: "preference".into(),
                    scope: "global".into(),
                    content: content.into(),
                    tags: Vec::new(),
                    importance: 4,
                }],
            )
            .unwrap();
    }
    assert_eq!(
        store
            .search("a", "What shell does the user prefer?", None, 8)
            .unwrap()[0]
            .content,
        "User prefers PowerShell"
    );
    assert_eq!(
        store.search("a", "不要自动提交 git", None, 8).unwrap()[0].content,
        "用户以后不要自动 git commit"
    );
    assert_eq!(
        store
            .search("a", "我们之前数据库迁移用了什么", None, 8)
            .unwrap()[0]
            .content,
        "数据库迁移使用 SQLAlchemy"
    );
}

#[test]
fn migration_is_idempotent_and_keeps_backups() {
    let (dir, db) = database();
    let mut c = AppConfig::default();
    c.general.data_dir = dir.path().to_owned();
    let note = dir.path().join("memory/a");
    std::fs::create_dir_all(&note).unwrap();
    std::fs::write(note.join("USER.md"), "Shell preference").unwrap();
    std::fs::write(note.join("MEMORY.md"), "first fact§second fact").unwrap();
    migrate_legacy::migrate(&db, &c).unwrap();
    migrate_legacy::migrate(&db, &c).unwrap();
    assert!(note.join("USER.md.legacy.bak").exists());
    let store = MemoryStore::new(db);
    assert_eq!(store.search("a", "fact", None, 8).unwrap().len(), 2);
    assert_eq!(store.profile("a").unwrap().0, "Shell preference");
}
#[test]
fn scheduled_occurrences_respect_agent_and_reminder_grace_without_catch_up() {
    let (_dir, db) = database();
    let now = chrono::Utc::now();
    for (name, action) in [
        (
            "agent",
            JobAction::CodexTask {
                prompt: "task".into(),
                model: None,
            },
        ),
        (
            "reminder",
            JobAction::Reminder {
                message: "ping".into(),
            },
        ),
    ] {
        let at = now - chrono::Duration::seconds(700);
        db.save_job(
            &CronJob {
                id: name.into(),
                user_id: "a".into(),
                title: name.into(),
                kind: CronKind::OneShot { at },
                action,
                created_at: now,
                next_run_at: Some(at),
                disabled: false,
            },
            600,
            1800,
        )
        .unwrap();
    }
    db.claim_due_jobs().unwrap();
    db.claim_due_jobs().unwrap();
    let rows = db.claimed_jobs().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].job.id, "reminder");
    let states: Vec<(String, String)> = db
        .with(|s| {
            let mut q = s.prepare("SELECT job_id,state FROM scheduled_runs ORDER BY job_id")?;
            Ok(q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?)
        })
        .unwrap();
    assert_eq!(
        states,
        vec![
            ("agent".into(), "missed".into()),
            ("reminder".into(), "claimed".into())
        ]
    );
}
#[test]
fn history_fts_never_crosses_owner() {
    let (_dir, db) = database();
    db.record_message("a", 1, "user", "PowerShell preference", Some("m"))
        .unwrap();
    assert_eq!(
        db.session_search("a", "PowerShell", None, 8).unwrap().len(),
        1
    );
    assert!(
        db.session_search("b", "PowerShell", None, 8)
            .unwrap()
            .is_empty()
    );
    assert!(db.session_get("b", 1, 0, i64::MAX).unwrap().is_empty());
}

#[test]
fn memory_distill_only_consumes_turns_present_in_the_bounded_transcript() {
    let (_dir, db) = database();
    let store = MemoryStore::new(db.clone());
    for i in 0..25 {
        let message_id = format!("memory-{i}");
        let body = "x".repeat(2_000);
        db.record_message("a", 1, "user", &body, Some(&message_id))
            .unwrap();
        db.record_message("a", 1, "assistant", &body, Some(&message_id))
            .unwrap();
        store.completed_turn("a").unwrap();
    }

    let (_transcript, end, processed_turns) = store.transcript("a").unwrap();
    assert!(processed_turns > 0);
    assert!(processed_turns < 25);
    store.commit_distill("a", &[], end, processed_turns).unwrap();

    let remaining: i64 = db
        .with(|sql| {
            Ok(sql.query_row(
                "SELECT completed_turns FROM memory_cursors WHERE user_id='a'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(remaining, 25 - processed_turns);
}

#[test]
fn memory_distill_backoff_and_transcript_exclude_scheduled_output() {
    let (_dir, db) = database();
    let store = MemoryStore::new(db.clone());
    db.record_message("a", 0, "assistant", "scheduled summary", Some("cron-1"))
        .unwrap();
    db.record_message("a", 1, "user", "remember this project choice", Some("turn-1"))
        .unwrap();
    db.record_message("a", 1, "assistant", "ack", Some("turn-1"))
        .unwrap();
    store.completed_turn("a").unwrap();

    let (transcript, _end, turns) = store.transcript("a").unwrap();
    assert!(!transcript.contains("scheduled summary"));
    assert!(transcript.contains("remember this project choice"));
    assert_eq!(turns, 1);

    assert!(store.claim_distill("a").unwrap());
    let delay = store.fail_distill("a").unwrap();
    assert_eq!(delay, 30);
    assert!(store.due_users(1, 0).unwrap().is_empty());
    let cursor: (i64, i64, i64) = db
        .with(|sql| {
            Ok(sql.query_row(
                "SELECT pending,distill_failures,next_distill_at FROM memory_cursors WHERE user_id='a'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .unwrap();
    assert_eq!(cursor.0, 0);
    assert_eq!(cursor.1, 1);
    assert!(cursor.2 > chrono::Utc::now().timestamp());
}

#[test]
fn running_scheduled_occurrence_is_failed_not_replayed_after_restart() {
    let (_dir, db) = database();
    let now = chrono::Utc::now();
    let job = CronJob {
        id: "agent-running".into(),
        user_id: "a".into(),
        title: "agent-running".into(),
        kind: CronKind::OneShot { at: now },
        action: JobAction::CodexTask {
            prompt: "task".into(),
            model: None,
        },
        created_at: now,
        next_run_at: Some(now),
        disabled: false,
    };
    db.save_job(&job, 600, 1800).unwrap();
    db.claim_due_jobs().unwrap();
    let run = db.claimed_jobs().unwrap().pop().unwrap();
    db.mark_run("a", run.id, "running", None).unwrap();
    db.recover().unwrap();
    assert!(db.claimed_jobs().unwrap().is_empty());
    let state: String = db
        .with(|sql| {
            Ok(sql.query_row(
                "SELECT state FROM scheduled_runs WHERE id=?1",
                [run.id],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(state, "failed");
}

#[test]
fn outbox_remembers_partial_progress_and_bounds_retries() {
    let (_dir, db) = database();
    let payload = outbox::Delivery {
        text: "Done".into(),
        directives: vec![crate::qq::Directive::Image { path: "/tmp/example.png".into() }],
    };
    db.enqueue_delivery("a", Some("msg"), "partial", &payload).unwrap();
    let first = db.next_delivery().unwrap().unwrap();
    assert!(!first.text_sent);
    assert_eq!(first.directives_sent, 0);
    db.mark_delivery_text_sent("a", first.id).unwrap();
    db.finish_delivery("a", first.id, Some("temporary image upload failure"), false).unwrap();
    // force the retry window open without sleeping
    db.with(|c| { c.execute("UPDATE outbox SET next_attempt_at=0 WHERE id=?1", [first.id])?; Ok(()) }).unwrap();
    let second = db.next_delivery().unwrap().unwrap();
    assert!(second.text_sent);
    assert_eq!(second.directives_sent, 0);
    db.mark_delivery_directive_sent("a", second.id, 1).unwrap();
    db.finish_delivery("a", second.id, None, false).unwrap();
    assert!(db.next_delivery().unwrap().is_none());
    db.enqueue_delivery("a", Some("msg"), "bad", &payload).unwrap();
    let bad = db.next_delivery().unwrap().unwrap();
    db.finish_delivery("a", bad.id, Some("40034128"), true).unwrap();
    let state = db.with(|c| Ok(c.query_row("SELECT state FROM outbox WHERE id=?1", [bad.id], |r| r.get::<_, String>(0))?)).unwrap();
    assert_eq!(state, "failed");
}
