use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{oneshot, watch};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkKind {
    UserTurn,
    ScheduledTurn,
    MemoryDistill,
}
struct Waiting {
    user: String,
    kind: WorkKind,
    enqueued_at: tokio::time::Instant,
    deadline: Option<tokio::time::Instant>,
    tx: oneshot::Sender<WorkPermit>,
}

const SCHEDULED_FAIR_WAIT: Duration = Duration::from_secs(15);
const SCHEDULED_DEADLINE_URGENCY: Duration = Duration::from_secs(30);
struct QueueState {
    running: usize,
    users: HashSet<String>,
    scheduled: usize,
    memory: usize,
    waiting: VecDeque<Waiting>,
    memory_cancel: Option<watch::Sender<bool>>,
}
#[derive(Clone)]
pub struct WorkQueue {
    inner: Arc<Mutex<QueueState>>,
    max: usize,
}
pub struct WorkPermit {
    armed: bool,
    queue: WorkQueue,
    user: String,
    kind: WorkKind,
    pub preempt: watch::Receiver<bool>,
}
impl WorkQueue {
    pub fn new(max: usize) -> Self {
        Self {
            max: max.clamp(1, 2),
            inner: Arc::new(Mutex::new(QueueState {
                running: 0,
                users: HashSet::new(),
                scheduled: 0,
                memory: 0,
                waiting: VecDeque::new(),
                memory_cancel: None,
            })),
        }
    }
    pub async fn acquire(
        &self,
        user: &str,
        kind: WorkKind,
        deadline: Option<tokio::time::Instant>,
    ) -> Option<WorkPermit> {
        let (tx, rx) = oneshot::channel();
        {
            let mut s = self.inner.lock().expect("queue lock");
            if kind != WorkKind::MemoryDistill {
                if let Some(cancel) = &s.memory_cancel {
                    let _ = cancel.send(true);
                }
            }
            s.waiting.push_back(Waiting {
                user: user.to_owned(),
                kind,
                enqueued_at: tokio::time::Instant::now(),
                deadline,
                tx,
            });
        }
        self.drain();
        if let Some(deadline) = deadline {
            tokio::time::timeout_at(deadline, rx).await.ok()?.ok()
        } else {
            rx.await.ok()
        }
    }
    fn drain(&self) {
        let mut s = self.inner.lock().expect("queue lock");
        s.waiting.retain(|w| !w.tx.is_closed());
        while s.running < self.max {
            let eligible = |w: &Waiting| {
                !s.users.contains(&w.user)
                    && (w.kind != WorkKind::ScheduledTurn || s.scheduled == 0)
                    && (w.kind != WorkKind::MemoryDistill || s.memory == 0)
            };
            // Users win by default. A scheduled task gets one fair slot after
            // a short wait or when it approaches its grace deadline.
            let now = tokio::time::Instant::now();
            let urgent_scheduled = s.waiting.iter().position(|w| {
                w.kind == WorkKind::ScheduledTurn
                    && eligible(w)
                    && (now.duration_since(w.enqueued_at) >= SCHEDULED_FAIR_WAIT
                        || w.deadline.is_some_and(|deadline| {
                            deadline <= now + SCHEDULED_DEADLINE_URGENCY
                        }))
            });
            let user = s
                .waiting
                .iter()
                .position(|w| w.kind == WorkKind::UserTurn && eligible(w));
            let scheduled = s
                .waiting
                .iter()
                .position(|w| w.kind == WorkKind::ScheduledTurn && eligible(w));
            let pick = urgent_scheduled.or(user).or(scheduled).or_else(|| {
                if s.waiting.iter().any(|w| w.kind != WorkKind::MemoryDistill) {
                    None
                } else {
                    s.waiting.iter().position(eligible)
                }
            });
            let Some(i) = pick else { break };
            let w = s.waiting.remove(i).unwrap();
            let (cancel, preempt) = watch::channel(false);
            s.running += 1;
            s.users.insert(w.user.clone());
            match w.kind {
                WorkKind::ScheduledTurn => s.scheduled += 1,
                WorkKind::MemoryDistill => {
                    s.memory += 1;
                    s.memory_cancel = Some(cancel);
                }
                _ => {}
            }
            let permit = WorkPermit {
                armed: true,
                queue: self.clone(),
                user: w.user,
                kind: w.kind,
                preempt,
            };
            // Receiver cancellation must not drop a permit while holding this lock.
            if let Err(mut permit) = w.tx.send(permit) {
                s.running -= 1;
                s.users.remove(&permit.user);
                match permit.kind {
                    WorkKind::ScheduledTurn => s.scheduled -= 1,
                    WorkKind::MemoryDistill => {
                        s.memory -= 1;
                        s.memory_cancel = None;
                    }
                    _ => {}
                }
                permit.armed = false;
            }
        }
    }
    pub async fn memory_slot(&self, user: &str) -> Option<WorkPermit> {
        self.acquire(
            user,
            WorkKind::MemoryDistill,
            Some(tokio::time::Instant::now() + Duration::from_secs(1)),
        )
        .await
    }
}
impl Drop for WorkPermit {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        {
            let mut s = self.queue.inner.lock().expect("queue lock");
            s.running -= 1;
            s.users.remove(&self.user);
            match self.kind {
                WorkKind::ScheduledTurn => s.scheduled -= 1,
                WorkKind::MemoryDistill => {
                    s.memory -= 1;
                    s.memory_cancel = None;
                }
                _ => {}
            }
        }
        self.queue.drain();
    }
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;
    use futures_util::FutureExt;
    #[tokio::test]
    async fn two_users_run_while_same_owner_waits() {
        let q = WorkQueue::new(2);
        let a = q.acquire("a", WorkKind::UserTurn, None).await.unwrap();
        let b = q.acquire("b", WorkKind::UserTurn, None).await.unwrap();
        let mut a2 = Box::pin(q.acquire("a", WorkKind::UserTurn, None));
        assert!(a2.as_mut().now_or_never().is_none());
        drop(b);
        assert!(a2.as_mut().now_or_never().is_none());
        drop(a);
        let _a2 = a2.await.unwrap();
    }
    #[tokio::test]
    async fn user_waiting_behind_busy_slot_runs_before_fresh_scheduled_job() {
        let q = WorkQueue::new(1);
        let blocker = q.acquire("blocker", WorkKind::UserTurn, None).await.unwrap();
        let mut scheduled = Box::pin(q.acquire(
            "scheduled",
            WorkKind::ScheduledTurn,
            Some(tokio::time::Instant::now() + Duration::from_secs(600)),
        ));
        let mut user = Box::pin(q.acquire("user", WorkKind::UserTurn, None));
        assert!(scheduled.as_mut().now_or_never().is_none());
        assert!(user.as_mut().now_or_never().is_none());
        drop(blocker);
        let permit = user.await.unwrap();
        assert!(scheduled.as_mut().now_or_never().is_none());
        drop(permit);
        let _scheduled = scheduled.await.unwrap();
    }

    #[tokio::test]
    async fn scheduled_has_one_slot_and_memory_yields_to_user() {
        let q = WorkQueue::new(2);
        let s = q.acquire("s", WorkKind::ScheduledTurn, None).await.unwrap();
        let mut s2 = Box::pin(q.acquire("t", WorkKind::ScheduledTurn, None));
        assert!(s2.as_mut().now_or_never().is_none());
        let u = q.acquire("u", WorkKind::UserTurn, None).await.unwrap();
        let mut m = Box::pin(q.acquire("m", WorkKind::MemoryDistill, None));
        assert!(m.as_mut().now_or_never().is_none());
        drop(s);
        let s2 = s2.await.unwrap();
        drop(u);
        let mut m = m.await.unwrap();
        let mut next = Box::pin(q.acquire("v", WorkKind::UserTurn, None));
        assert!(next.as_mut().now_or_never().is_none());
        m.preempt.changed().await.unwrap();
        assert!(*m.preempt.borrow());
        drop(m);
        let _next = next.await.unwrap();
        drop(s2);
    }
}
