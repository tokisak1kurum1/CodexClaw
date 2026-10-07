---
name: claw-cron
description: Create and manage this QQ user's reminders and isolated scheduled Codex tasks.
---
Use the `schedule_create` and `schedule_manage` dynamic tools supplied by CodexClaw. Ownership is bound by the application to the active thread; never accept or invent a user ID. A task can be a plain reminder or an isolated, noninteractive Codex turn. Use an explicit RFC3339 time for one-time tasks or a six-field cron expression and timezone for recurring tasks. Do not run a shell command or write jobs.json. Use schedule_manage to list, pause, resume, remove, run-now or inspect recent runs. Missed occurrences beyond the grace window are skipped, not replayed.
