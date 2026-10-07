use super::StateDb;
use crate::model::settings::{DialogState, UserSessionState};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
impl StateDb {
    pub(crate) fn load_users(
        &self,
    ) -> Result<std::collections::BTreeMap<String, UserSessionState>> {
        self.with(|db| {
            let mut q = db.prepare("SELECT user_id,state_json FROM users")?;
            let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            let mut users = std::collections::BTreeMap::new();
            for row in rows {
                let (id, raw) = row?;
                users.insert(id, serde_json::from_str(&raw)?);
            }
            Ok(users)
        })
    }
    pub(crate) fn dialog_id(&self, user: &str, thread: Option<&str>) -> Result<i64> {
        self.with(|db|{let id=if let Some(thread)=thread{db.query_row("SELECT id FROM dialogs WHERE user_id=?1 AND codex_thread_id=?2 ORDER BY id DESC LIMIT 1",params![user,thread],|r|r.get(0)).optional()?}else{db.query_row("SELECT id FROM dialogs WHERE user_id=?1 AND state='foreground'",[user],|r|r.get(0)).optional()?};id.ok_or_else(||anyhow::anyhow!("dialog not found for owner"))})
    }
    pub(crate) fn owned_dialogs(
        &self,
        user: &str,
    ) -> Result<Vec<(String, Option<String>, String)>> {
        self.with(|db|{let mut q=db.prepare("SELECT codex_thread_id,alias,state FROM dialogs WHERE user_id=?1 AND codex_thread_id IS NOT NULL ORDER BY updated_at DESC,id DESC LIMIT 100")?;Ok(q.query_map([user],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?.collect::<rusqlite::Result<_>>()?)})
    }
    pub(crate) fn owned_dialog(&self, user: &str, thread: &str) -> Result<Option<DialogState>> {
        self.with(|db|{let raw=db.query_row("SELECT dialog_json FROM dialogs WHERE user_id=?1 AND codex_thread_id=?2 ORDER BY id DESC LIMIT 1",params![user,thread],|r|r.get::<_,String>(0)).optional()?;raw.map(|r|serde_json::from_str(&r).map_err(Into::into)).transpose()})
    }
    pub(crate) fn loaded_versions(&self, user: &str, id: i64) -> Result<(i64, i64)> {
        self.with(|db|Ok(db.query_row("SELECT loaded_profile_version,loaded_persona_version FROM dialogs WHERE user_id=?1 AND id=?2",params![user,id],|r|Ok((r.get(0)?,r.get(1)?)))?))
    }
    pub(crate) fn set_loaded_versions(
        &self,
        user: &str,
        id: i64,
        profile: i64,
        persona: i64,
    ) -> Result<()> {
        self.with(|db|{db.execute("UPDATE dialogs SET loaded_profile_version=?3,loaded_persona_version=?4 WHERE user_id=?1 AND id=?2",params![user,id,profile,persona])?;Ok(())})
    }
}
pub(crate) fn save_user(
    db: &rusqlite::Connection,
    id: &str,
    user: &UserSessionState,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let s = &user.settings;
    db.execute("INSERT INTO users(user_id,language,model,reasoning_effort,service_tier,context_mode,created_at,updated_at,state_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?7,?8) ON CONFLICT(user_id) DO UPDATE SET language=excluded.language,model=excluded.model,reasoning_effort=excluded.reasoning_effort,service_tier=excluded.service_tier,context_mode=excluded.context_mode,updated_at=excluded.updated_at,state_json=excluded.state_json",params![id,s.language,s.model_override,s.reasoning_effort.map(|x|x.as_str()),s.service_tier.map(|x|x.as_str()),s.context_mode.map(|x|x.label()),now,serde_json::to_string(user)?])?;
    // Archive first so swaps cannot violate the partial foreground/alias indexes.
    db.execute("UPDATE dialogs SET state='archived' WHERE user_id=?1", [id])?;
    save_dialog(db, id, None, "foreground", &user.foreground)?;
    for (alias, d) in &user.background {
        save_dialog(db, id, Some(alias), "background", d)?;
    }
    Ok(())
}
fn save_dialog(
    db: &rusqlite::Connection,
    user: &str,
    alias: Option<&str>,
    state: &str,
    d: &DialogState,
) -> Result<()> {
    let old: Option<i64> = if let Some(thread) = &d.session_id {
        db.query_row("SELECT id FROM dialogs WHERE user_id=?1 AND codex_thread_id=?2 ORDER BY id DESC LIMIT 1",params![user,thread],|r|r.get(0)).optional()?.or(db.query_row("SELECT id FROM dialogs WHERE user_id=?1 AND codex_thread_id IS NULL AND json_extract(dialog_json,'$.generation')=?2 ORDER BY id DESC LIMIT 1",params![user,i64::try_from(d.generation)?],|r|r.get(0)).optional()?)
    } else {
        db.query_row("SELECT id FROM dialogs WHERE user_id=?1 AND codex_thread_id IS NULL AND json_extract(dialog_json,'$.generation')=?2 ORDER BY id DESC LIMIT 1",params![user,i64::try_from(d.generation)?],|r|r.get(0)).optional()?
    };
    let now = chrono::Utc::now().timestamp();
    let profile = d.profile.clone().unwrap_or_default();
    let raw = serde_json::to_string(d)?;
    if let Some(id) = old {
        db.execute("UPDATE dialogs SET alias=?3,state=?4,codex_thread_id=?5,model=?6,reasoning_effort=?7,service_tier=?8,context_mode=?9,updated_at=?10,dialog_json=?11 WHERE user_id=?1 AND id=?2",params![user,id,alias,state,d.session_id,profile.model_override,profile.reasoning_effort.map(|x|x.as_str()),profile.service_tier.map(|x|x.as_str()),profile.context_mode.map(|x|x.label()),now,raw])?;
    } else {
        db.execute("INSERT INTO dialogs(user_id,alias,state,codex_thread_id,model,reasoning_effort,service_tier,context_mode,created_at,updated_at,dialog_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9,?10)",params![user,alias,state,d.session_id,profile.model_override,profile.reasoning_effort.map(|x|x.as_str()),profile.service_tier.map(|x|x.as_str()),profile.context_mode.map(|x|x.label()),now,raw])?;
    }
    Ok(())
}
