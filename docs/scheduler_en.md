# Scheduled tasks

Only plain reminders and isolated noninteractive Codex tasks are supported. Reminders use no model slot. Agent tasks use the shared WorkQueue, the owner's workspace and an ephemeral thread without replacing the foreground conversation.

```
/cron list
/cron once 2026-10-06T09:00:00+08:00 reminder Drink water
/cron add '0 0 9 * * *' codex 'Summarize my tasks'
/cron pause <id>
/cron resume <id>
/cron run-now <id>
/cron tail <id>
/cron rm <id>
```

Cron has six fields including seconds; default timezone is Asia/Shanghai. Once uses RFC3339. Dynamic schedule_create/manage tools bind ownership in App. The CLI requires CODEX_CLAW_USER_ID and exposes the same CRUD.

SQLite enforces a unique (job_id, scheduled_at) occurrence. Default misfire grace is 600 s for agents and 1800 s for reminders, including queue wait. Expired slots are missed, with no catch-up. Recurring schedules advance to the next future slot. Result/outbox/run-success commit atomically; delivery errors do not rerun the model. Agent timeout defaults to 600 s and sends an interrupt. Restart recovers unfinished claims. Interactive jobs, persistent job sessions, shell actions and separate scheduler daemons are removed.
