use super::StateDb;
use crate::{
    config::AppConfig,
    model::{cron::CronJob, settings::UserSessionState},
};
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use std::path::PathBuf;
pub fn migrate(db: &StateDb, config: &AppConfig) -> Result<()> {
    let completed = db.with(|d| {
        Ok(d.query_row(
            "SELECT value FROM meta WHERE key='legacy_migration_completed'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .is_some())
    })?;
    if completed {
        return finish_backups(db);
    }
    let root = &config.general.data_dir;
    let files = [
        root.join("session/state.json"),
        root.join("state.json"),
        root.join("scheduler/jobs.json"),
        root.join("jobs.json"),
        root.join("qq/gateway-session.json"),
    ];
    let mut backups = Vec::<PathBuf>::new();
    let mut users = std::collections::BTreeMap::<String, UserSessionState>::new();
    let mut jobs = Vec::<CronJob>::new();
    let mut gateway = None;
    for path in files {
        if !path.exists() {
            continue;
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("read legacy {}", path.display()))?;
        let v: serde_json::Value = serde_json::from_str(&raw)?;
        backups.push(path.clone());
        if path.ends_with("gateway-session.json") {
            gateway = Some(raw);
            continue;
        }
        if let Some(us) = v["users"].as_object() {
            for (user, state) in us {
                let mut state = state.clone();
                state["pending_setting"] = serde_json::Value::Null;
                let mut user_state: UserSessionState = serde_json::from_value(state)?;
                let workspace = root
                    .parent()
                    .unwrap_or(root)
                    .join("users")
                    .join(format!("{:x}", md5::compute(user)))
                    .join("workspace");
                std::fs::create_dir_all(&workspace)?;
                user_state.foreground.workspace_dir = workspace.clone();
                for (alias, dialog) in &mut user_state.background {
                    dialog.workspace_dir = workspace.clone();
                    dialog.alias = Some(alias.clone());
                }
                user_state.pending_setting = None;
                users.insert(user.clone(), user_state);
            }
        }
        let collection = v.get("cron_jobs").or_else(|| v.get("jobs")).unwrap_or(&v);
        let values: Vec<_> = if let Some(map) = collection.as_object() {
            map.values().collect()
        } else if let Some(a) = collection.as_array() {
            a.iter().collect()
        } else {
            Vec::new()
        };
        for value in values {
            if value.get("action").is_some() {
                match serde_json::from_value::<CronJob>(value.clone()) {
                    Ok(job) => jobs.push(job),
                    Err(e) => {
                        tracing::warn!(error=%e,file=%path.display(),"legacy job uses a removed action; preserved in backup")
                    }
                }
            }
        }
    }
    let mut notes = Vec::new();
    let memory = root.join("memory");
    if memory.exists() {
        for entry in std::fs::read_dir(memory)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let user = entry.file_name().to_string_lossy().into_owned();
            for name in ["USER.md", "MEMORY.md"] {
                let path = entry.path().join(name);
                if path.exists() {
                    let text = std::fs::read_to_string(&path)?;
                    notes.push((user.clone(), name.to_owned(), text));
                    backups.push(path);
                }
            }
        }
    }
    db.with(|d|{let tx=d.transaction()?;for(user,state)in &users{super::users::save_user(&tx,user,state)?;}for job in &jobs{super::jobs::save_job(&tx,job,config.scheduler.agent_misfire_grace_secs,config.scheduler.reminder_misfire_grace_secs)?;}let now=chrono::Utc::now().timestamp();for(user,name,text)in &notes{if name=="USER.md"{let text:String=text.chars().take(4000).collect();tx.execute("INSERT OR IGNORE INTO user_profiles(user_id,profile_text,updated_at) VALUES(?1,?2,?3)",params![user,text,now])?;}else{for entry in text.split('§').map(str::trim).filter(|x|!x.is_empty()){let chars:Vec<_>=entry.chars().collect();for chunk in chars.chunks(300){let content:String=chunk.iter().collect();tx.execute("INSERT INTO memories(user_id,kind,scope,content,importance,created_at,updated_at) VALUES(?1,'project','global',?2,3,?3,?3)",params![user,content,now])?;}}}}
  if let Some(gateway)=&gateway{tx.execute("INSERT OR IGNORE INTO meta(key,value) VALUES('gateway_session',?1)",[gateway])?;}
  tx.execute("INSERT INTO meta(key,value) VALUES('legacy_backups',?1)",[serde_json::to_string(&backups)?])?;tx.execute("INSERT INTO meta(key,value) VALUES('legacy_migration_completed','1')",[])?;tx.commit()?;Ok(())})?;
    finish_backups(db)
}
fn finish_backups(db: &StateDb) -> Result<()> {
    let paths = db.with(|d| {
        Ok(d.query_row(
            "SELECT value FROM meta WHERE key='legacy_backups'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?)
    })?;
    if let Some(raw) = paths {
        for p in serde_json::from_str::<Vec<PathBuf>>(&raw)? {
            let target = PathBuf::from(format!("{}.legacy.bak", p.display()));
            if p.exists() && !target.exists() {
                if let Err(e) = std::fs::rename(&p, &target) {
                    tracing::warn!(error=%e,file=%p.display(),"legacy backup rename will be retried next startup");
                }
            }
        }
    }
    Ok(())
}
