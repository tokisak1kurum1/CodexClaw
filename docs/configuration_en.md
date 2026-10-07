# Configuration

Load order: CODEX_CLAW_CONFIG, ./codexclaw.toml, then ~/.codex-claw/codexclaw.toml.

```toml
[qq]
app_id = "YOUR_QQ_APP_ID"
app_secret = "YOUR_QQ_APP_SECRET"
allowed_users = [] # Recommended: QQ openids allowed to use the bot; empty allows all C2C users

[codex]
expected_version = "0.159.2"

[general]
data_dir = "~/.codex-claw/data"
codex_home_global = "~/.codex-claw/.codex"
system_codex_home = "~/.codex"
codex_binary = "codex"
timezone = "Asia/Shanghai"

[runtime]
max_concurrent_codex = 2
max_concurrent_per_user = 1
max_concurrent_scheduled = 1
max_concurrent_memory = 1

[scheduler]
agent_misfire_grace_secs = 600
reminder_misfire_grace_secs = 1800
catch_up_missed = false

[memory]
model = "gpt-5.6-luna"
distill_after_turns = 12
distill_idle_secs = 120
relevant_limit = 8
hot_memory_max_chars = 4000
single_memory_max_chars = 300

[attachments]
max_file_bytes = 33554432
per_user_quota_bytes = 268435456
retention_hours = 24

```

Global concurrency is at most 2; per-user, scheduled and memory limits must be 1. Catch-up must remain false. An empty model uses the daemon default. Model menus use model/list with a 5-minute, connection-sensitive cache. QQ settings write SQLite, not shared config.toml.

BEHAVIOR.md is limited to 8000 characters, IDENTITY.md to 500, CHARACTER.md to 16000, profile to 4000, and hot memory to 4000; a memory entry has at most 300. The three character files are injected only when a new thread is created, so an existing thread stays pinned to its original character version; legacy SOUL.md is no longer read. If BEHAVIOR.md is missing, the embedded default is used; a runtime file overrides it. Attachments default to 32 MiB each, 256 MiB per user and 24-hour retention. Output files must stay in the current user's directories. Operators prepare and start the pinned official daemon separately with the same CODEX_HOME.
