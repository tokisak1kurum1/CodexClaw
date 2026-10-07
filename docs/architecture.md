# 当前架构

QQ Gateway 在推进 sequence 前，把 C2C 消息按平台 message id 去重并写入 SQLite inbox。控制命令立即处理；空闲用户的第一条普通消息立即启动 turn，不再做静默等待。活动 turn 期间新到的普通消息留在持久化 inbox，当前回复结束后一次合并为下一轮 follow-up。

所有 Codex 工作共享 WorkQueue：全局 2、每用户 1、定时 agent 1、记忆蒸馏 1。定时任务优先领取其专用上限内的空位，记忆工作最低优先级，可被用户或定时任务中断。App 的 ActiveTurns 按 user/thread 路由审批、动态工具和中断。无 threadId 的普通通知不会归入任何用户 turn；连接断开作为独立连接事件处理。

执行链路为 CodexClaw → Unix socket WebSocket → 官方 Codex daemon。启动检查 CLI 与 daemon 版本都匹配 `[codex].expected_version`（当前 0.159.2）。断线只重连 daemon socket，不自动启动、更新或接管 daemon。所有前台、定时和 ephemeral 记忆工作都走这条链路。

`data/state.db` 是唯一业务状态库，启用 WAL、NORMAL、foreign_keys 和 5 s busy timeout。包含 users、dialogs、inbox、outbox、messages、user_profiles、memories、scheduled_jobs、scheduled_runs、memory_cursors 和 meta；messages 与 memories 使用 FTS5，历史搜索提供子串回退。SQL/store 对象访问均携带 owner。用户目录为 `users/<稳定hash>/workspace` 与 `inbox`，不把共享状态目录作为 writable root。该目录和所有权约束不等同于独立 OS 用户容器。

最终回复、assistant 历史和 inbox 完成状态在同一事务提交。发送失败只重试 outbox，不重新调用模型。定时结果和运行完成状态同样事务提交。QQ 网络发送存在不确定的失败窗口，所以允许出现重复送达，不承诺 exactly-once。

BEHAVIOR.md、IDENTITY.md、CHARACTER.md 是只读角色配置；BEHAVIOR.md 缺失时使用编译内置默认规则，user profile 和原子 memory 在 SQLite。三份角色配置只在新 thread 的 developerInstructions 中注入一次，并固定到该 thread；运行中修改角色文件只影响之后的新会话。user profile 版本变化时，当前 thread 只补充一次 profile-update。后续 turn 默认仅加入当前输入和相关检索。蒸馏在累计 12 轮、空闲 120 s、/new 或 compact 前触发，采用 read-only ephemeral thread，结构化 operations 经事务应用。显式记住、纠正、遗忘走绑定当前 owner 的动态工具。

旧 state.json、jobs.json、USER.md、MEMORY.md 在首次启动事务迁移；完成后写 meta，再改名为 .legacy.bak，失败的备份改名下次启动重试。不导入 Codex rollout。已移除 self-update、rollout import/loadbg、命令宏、交互式定时会话、独立 ShadowWorker、codex exec 子进程。

模块：state 负责 SQL 和迁移；session 保留 Dialogs 状态机并用 SQLite 持久化；work_queue 统一调度；codex/app_server 保留 stdio JSON-RPC 协议层；memory 负责 CRUD、注入、动态工具和蒸馏；scheduler 负责最小任务与 occurrence 运行记录；app 负责组合与 QQ 路由。
