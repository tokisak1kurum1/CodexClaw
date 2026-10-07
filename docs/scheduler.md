# 定时任务

只有纯提醒和非交互式 Codex task。提醒不占 Codex slot；Codex task 通过统一 WorkQueue，使用用户独立 workspace 和 ephemeral thread，不更改前台 dialog。

```text
/cron list
/cron once 2026-10-06T09:00:00+08:00 reminder 喝水
/cron add '0 0 9 * * *' codex '整理今天的待办'
/cron pause <id>
/cron resume <id>
/cron run-now <id>
/cron tail <id>
/cron rm <id>
```

cron 使用六个字段（秒、分、时、日、月、星期），默认 Asia/Shanghai；一次任务使用 RFC3339。模型提供 schedule_create / schedule_manage 动态工具，owner 由 App 绑定，不接受参数中的 user_id。命令行 `CODEX_CLAW_USER_ID=<openid> codex-claw cron ...` 使用相同的最小 CRUD。

任务持久化于 scheduled_jobs，执行槽位持久化于 scheduled_runs，(job_id, scheduled_at) 唯一。默认 grace：agent 600 s、reminder 1800 s，超过窗口标记 missed，不补跑。等待 slot 时 grace 仍继续计时。只计算 recurring 的下一个未来时间点。最终输出与 outbox、run success 在同一事务提交，QQ 失败只重试投递。单次 agent 上限默认 600 s，超时发 turn interrupt。

重启恢复未完成 claim；已完成结果不会重新运行模型。没有 interactive、reply TTL、persistent scheduler session、shell action、job workspace 回收或独立 scheduler daemon。
