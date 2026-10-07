use super::dialogs::Dialogs;
use crate::{model::settings::*, state::StateDb};
use anyhow::Result;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tokio::sync::RwLock;
pub struct SessionStore {
    pub db: StateDb,
    root: PathBuf,
    global_codex_home: PathBuf,
    state: RwLock<BTreeMap<String, UserSessionState>>,
}
pub(crate) struct SwitchResult {
    pub parked_alias: Option<String>,
    pub reserved_alias: Option<String>,
}
pub(crate) struct StopResult {
    pub had_session: bool,
    pub saved: bool,
    pub dropped_unsaved: bool,
    pub restored_alias: Option<String>,
}
impl SessionStore {
    pub async fn load_or_init(data_dir: &Path, global: &Path, _system: &Path) -> Result<Self> {
        let db = StateDb::open(data_dir)?;
        Self::from_db(db, data_dir, global)
    }
    pub fn from_db(db: StateDb, data_dir: &Path, global: &Path) -> Result<Self> {
        let mut users = db.load_users()?;
        let had_stale_picker = users.values().any(|user| user.pending_setting.is_some());
        if had_stale_picker {
            for user in users.values_mut() {
                user.pending_setting = None;
            }
            db.with(|conn| {
                let tx = conn.transaction()?;
                for (user_id, user) in &users {
                    crate::state::users::save_user(&tx, user_id, user)?;
                }
                tx.commit()?;
                Ok(())
            })?;
        }
        Ok(Self {
            db,
            root: data_dir.to_owned(),
            global_codex_home: global.to_owned(),
            state: RwLock::new(users),
        })
    }
    pub(crate) fn user_root(&self, user: &str) -> PathBuf {
        crate::DataLayout::new(&self.root).user_root(user)
    }
    pub(crate) fn workspace_for(&self, user: &str) -> PathBuf {
        self.user_root(user).join("workspace")
    }
    pub(crate) fn inbox_for(&self, user: &str) -> PathBuf {
        self.user_root(user).join("inbox")
    }
    pub(crate) fn codex_home(&self) -> &Path {
        &self.global_codex_home
    }
    fn fresh(&self, user: &str) -> Result<DialogState> {
        let path = self.workspace_for(user);
        std::fs::create_dir_all(&path)?;
        std::fs::create_dir_all(self.inbox_for(user))?;
        Ok(DialogState::new_temporary(path))
    }
    async fn mutate<T>(
        &self,
        user: &str,
        f: impl FnOnce(&mut UserSessionState) -> Result<T>,
    ) -> Result<T> {
        let mut all = self.state.write().await;
        let mut next = all
            .get(user)
            .cloned()
            .unwrap_or(UserSessionState::new(self.workspace_for(user)));
        self.fresh(user)?;
        let result = f(&mut next)?;
        self.db.with(|db| {
            let tx = db.transaction()?;
            crate::state::users::save_user(&tx, user, &next)?;
            tx.commit()?;
            Ok(())
        })?;
        all.insert(user.to_owned(), next);
        Ok(result)
    }
    pub(crate) async fn snapshot_for_user(&self, user: &str) -> Result<UserSessionState> {
        if let Some(x) = self.state.read().await.get(user).cloned() {
            return Ok(x);
        }
        self.mutate(user, |x| Ok(x.clone())).await
    }
    pub(crate) async fn language_for_user(&self, user: &str) -> Option<String> {
        self.state
            .read()
            .await
            .get(user)
            .map(|x| x.settings.language.clone())
    }
    pub(crate) async fn command_locale(&self, user: &str) -> String {
        self.language_for_user(user)
            .await
            .unwrap_or_else(default_language)
    }
    pub(crate) async fn update_settings_for_user(
        &self,
        user: &str,
        f: impl FnOnce(&mut SessionSettings),
    ) -> Result<UserSessionState> {
        self.mutate(user, |x| {
            f(&mut x.settings);
            Ok(x.clone())
        })
        .await
    }
    pub(crate) async fn set_pending_setting(
        &self,
        user: &str,
        p: Option<PendingSetting>,
    ) -> Result<()> {
        self.mutate(user, |x| {
            x.pending_setting = p;
            Ok(())
        })
        .await
    }
    pub(crate) async fn new_foreground(&self, user: &str, reserve: bool) -> Result<SwitchResult> {
        let fresh = self.fresh(user)?;
        self.mutate(user, |x| {
            let p = Dialogs::of(x).park(None, &self.workspace_for(user), fresh)?;
            if reserve {
                x.pending_park_alias = p.reserved_alias.clone();
            }
            let _ = p.cleanup_workspace;
            Ok(SwitchResult {
                parked_alias: p.parked_alias,
                reserved_alias: p.reserved_alias,
            })
        })
        .await
    }
    pub(crate) async fn move_foreground_to_background(
        &self,
        user: &str,
        alias: Option<&str>,
        reserve: bool,
    ) -> Result<SwitchResult> {
        let fresh = self.fresh(user)?;
        self.mutate(user, |x| {
            let p = Dialogs::of(x).park(alias, &self.workspace_for(user), fresh)?;
            if reserve {
                x.pending_park_alias = p.reserved_alias.clone();
            }
            let _ = p.cleanup_workspace;
            Ok(SwitchResult {
                parked_alias: p.parked_alias,
                reserved_alias: p.reserved_alias,
            })
        })
        .await
    }
    pub(crate) async fn foreground_from_background(
        &self,
        user: &str,
        alias: &str,
    ) -> Result<SwitchResult> {
        self.mutate(user, |x| {
            let next = Dialogs::of(x).take_background(alias)?;
            let p = Dialogs::of(x).park(None, &self.workspace_for(user), next)?;
            let _ = p.cleanup_workspace;
            Ok(SwitchResult {
                parked_alias: p.parked_alias,
                reserved_alias: p.reserved_alias,
            })
        })
        .await
    }
    pub(crate) async fn remember_sessions_view(&self, user: &str, ids: Vec<String>) -> Result<()> {
        self.mutate(user, |s| {
            s.last_sessions_view = ids;
            Ok(())
        })
        .await
    }
    pub(crate) async fn resume_owned(&self, user: &str, thread: &str) -> Result<()> {
        let mut dialog = self
            .db
            .owned_dialog(user, thread)?
            .ok_or_else(|| anyhow::anyhow!("conversation not found for owner"))?;
        dialog.workspace_dir = self.workspace_for(user);
        self.mutate(user, |s| {
            if s.foreground.session_id.as_deref() == Some(thread) {
                return Ok(());
            }
            if let Some(alias) = s
                .background
                .iter()
                .find(|(_, d)| d.session_id.as_deref() == Some(thread))
                .map(|(a, _)| a.clone())
            {
                dialog = Dialogs::of(s).take_background(&alias)?;
            }
            dialog.saved = true;
            Dialogs::of(s).park(None, &self.workspace_for(user), dialog)?;
            Ok(())
        })
        .await
    }
    pub(crate) async fn rename_background_alias(
        &self,
        user: &str,
        old: &str,
        new: &str,
    ) -> Result<()> {
        self.mutate(user, |x| Dialogs::of(x).rename_background(old, new))
            .await
    }
    pub(crate) async fn save_foreground(&self, user: &str) -> Result<bool> {
        self.mutate(user, |x| Ok(Dialogs::of(x).save())).await
    }
    pub(crate) async fn stop_foreground(&self, user: &str) -> Result<StopResult> {
        let fresh = self.fresh(user)?;
        self.mutate(user, |x| {
            let current = x.foreground.clone();
            x.pending_park_alias = None;
            let latest = super::dialogs::most_recent_background(x).map(|x| x.0);
            let mut dialogs = Dialogs::of(x);
            let next = if let Some(a) = &latest {
                dialogs.take_background(a)?
            } else {
                fresh
            };
            dialogs.install(next);
            Ok(StopResult {
                had_session: current.session_id.is_some(),
                saved: current.saved,
                dropped_unsaved: !current.saved,
                restored_alias: latest,
            })
        })
        .await
    }
    pub(crate) async fn set_active_profile(
        &self,
        user: &str,
        f: impl FnOnce(&mut DialogProfile),
        defaults: impl FnOnce(&mut SessionSettings),
    ) -> Result<()> {
        self.mutate(user, |x| {
            if !x.foreground.is_temporary() {
                f(x.foreground.profile.get_or_insert_with(Default::default));
            } else {
                defaults(&mut x.settings);
            }
            Ok(())
        })
        .await
    }
    pub(crate) async fn bind_turn_result(
        &self,
        user: &str,
        expected: &DialogState,
        thread: Option<String>,
        profile: DialogProfile,
        usage: Option<TokenUsageSnapshot>,
        park_unclaimed: bool,
    ) -> Result<Option<String>> {
        self.mutate(user, |x| {
            let reserved = x.pending_park_alias.take();
            let Some(thread) = thread else {
                return Ok(None);
            };
            let profile = if x.foreground.generation == expected.generation {
                x.foreground.profile.clone().unwrap_or(profile)
            } else {
                profile
            };
            if Dialogs::of(x).bind_if_current(expected, thread.clone(), profile.clone()) {
                x.foreground.last_usage = usage;
                return Ok(None);
            }
            if Dialogs::of(x)
                .attach_to_parked(expected, &thread, profile.clone(), usage.clone())
                .is_some()
            {
                return Ok(None);
            }
            if park_unclaimed || reserved.is_some() {
                let mut parked = expected.clone();
                parked.session_id = Some(thread);
                parked.profile = Some(profile);
                parked.last_usage = usage;
                parked.saved = true;
                parked.generation = 0;
                parked.alias = None;
                let alias = Dialogs::of(x).add_background(reserved.as_deref(), parked)?;
                return Ok(Some(alias));
            }
            Ok(None)
        })
        .await
    }
}
