# Current architecture

QQ Gateway persists and deduplicates each platform message before advancing its sequence. SQLite is the queue of record. Control commands are handled immediately; the first ordinary message for an idle user starts a turn without a quiet-window delay. Ordinary messages arriving during an active turn stay in the durable inbox and are merged into one follow-up turn after the current reply finishes.

One WorkQueue limits Codex work to 2 globally, 1 per user, 1 scheduled agent and 1 memory distillation. Memory work has lowest priority and can be interrupted. ActiveTurns routes approvals, tools and cancellation by owner and thread. Ordinary unscoped notifications do not belong to a user turn; daemon connection loss is a separate connection event.

All turns use the official daemon through a direct WebSocket-over-UDS connection, including ephemeral scheduled and memory work. Startup requires both CLI and daemon versions to equal codex.expected_version (0.159.2). CodexClaw reconnects only the daemon socket and never automatically starts or updates the daemon.

The single data/state.db uses WAL, NORMAL, foreign keys and a 5 s busy timeout. Tables store users, dialogs, inbox, outbox, messages, profiles, memories, jobs, occurrence runs, memory cursors and metadata. FTS5 search has a substring fallback. Every object access is owner-bound. Workspaces and inboxes use users/<stable hash>; shared state directories are not writable roots. These constraints do not provide separate OS-level user containers.

Final answers, history and inbox completion commit together; delivery retries never rerun the model. Scheduled result delivery and run completion also commit together. Ambiguous QQ failures can duplicate delivery; exactly-once is not promised.

BEHAVIOR.md, IDENTITY.md, and CHARACTER.md are read-only character configuration. Missing BEHAVIOR.md falls back to the embedded default behavior rules. Profiles and atomic memories live in SQLite. The three character files are injected once through developerInstructions when a new thread is created and remain pinned to that thread; edits take effect on the next new conversation. A profile version change adds only one profile-update to the current thread. Later turns otherwise add only the current input and relevant retrieval. Distillation runs after 12 new turns, 120 seconds idle, /new or manual compaction, using read-only ephemeral threads and transactional structured operations. Explicit remember/correct/forget requests use owner-bound dynamic tools.

Legacy state/jobs JSON and USER.md/MEMORY.md migrate once in a transaction and are renamed .legacy.bak after commit. Codex rollout import is removed. Self-update, command macros, interactive scheduled conversations, ShadowWorker and codex exec subprocess backends are removed.
