use std::collections::HashMap;
use tokio::sync::oneshot;

#[derive(Clone)]
pub(crate) struct ActiveContext {
    pub user_id: String,
    pub dialog_id: i64,
    pub thread_id: String,
    pub turn_id: String,
    pub message_id: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

struct ActiveTurn {
    context: ActiveContext,
    cancel: Option<oneshot::Sender<()>>,
    inbox_ids: Vec<String>,
}

#[derive(Default)]
pub(crate) struct ActiveTurns {
    by_user: HashMap<String, ActiveTurn>,
    by_thread: HashMap<String, String>,
}

impl ActiveTurns {
    pub fn install(
        &mut self,
        user: &str,
        dialog: i64,
        message: &str,
        inbox_ids: &[String],
    ) -> oneshot::Receiver<()> {
        assert!(!self.by_user.contains_key(user));
        assert!(self.by_user.len() < 2);
        let (tx, rx) = oneshot::channel();
        self.by_user.insert(
            user.to_owned(),
            ActiveTurn {
                context: ActiveContext {
                    user_id: user.to_owned(),
                    dialog_id: dialog,
                    thread_id: String::new(),
                    turn_id: String::new(),
                    message_id: message.to_owned(),
                    started_at: chrono::Utc::now(),
                },
                cancel: Some(tx),
                inbox_ids: inbox_ids.to_vec(),
            },
        );
        rx
    }

    pub fn bind(&mut self, user: &str, thread: &str, turn: Option<&str>) {
        if let Some(a) = self.by_user.get_mut(user) {
            if !a.context.thread_id.is_empty() && a.context.thread_id != thread {
                self.by_thread.remove(&a.context.thread_id);
            }
            a.context.thread_id = thread.to_owned();
            if let Some(turn) = turn {
                a.context.turn_id = turn.to_owned();
            }
            self.by_thread.insert(thread.to_owned(), user.to_owned());
        }
    }

    pub fn user_ids(&self) -> Vec<String> {
        self.by_user.keys().cloned().collect()
    }

    pub fn for_user(&self, user: &str) -> Option<ActiveContext> {
        self.by_user.get(user).map(|a| a.context.clone())
    }

    pub fn for_thread(&self, thread: &str) -> Option<ActiveContext> {
        self.by_thread.get(thread).and_then(|u| self.for_user(u))
    }

    pub fn remove(&mut self, user: &str) -> Vec<String> {
        let Some(a) = self.by_user.remove(user) else {
            return Vec::new();
        };
        if !a.context.thread_id.is_empty() {
            self.by_thread.remove(&a.context.thread_id);
        }
        a.inbox_ids
    }

    pub fn cancel(&mut self, user: &str) {
        if let Some(a) = self.by_user.get_mut(user) {
            if let Some(tx) = a.cancel.take() {
                let _ = tx.send(());
            }
        }
    }
}
