# 命令

中文同义命令仍然支持；`/model`、`/reasoning`、`/fast`、`/context`、`/lang`、`/verbose`、`/approvals`、`/plan` 不带参数显示数字菜单，`/back` 退出。

| 命令 | 行为 |
| --- | --- |
| `/help` `/status` | 帮助、当前用户状态和活动 turn |
| `/new` `/stop` `/interrupt` | 新建、停止前台、仅中断当前用户 turn |
| `/bg [alias]` `/fg [alias]` | 放后台、选择后台会话到前台；不增加常驻进程 |
| `/sessions` `/resume [编号或thread-id或alias]` | 仅列出、恢复当前用户拥有的数据库会话 |
| `/save` `/rename <old> <new>` | 保存前台、重命名后台 alias |
| `/model <name或inherit>` | 动态 model/list 菜单；设置当前已绑定 dialog 或用户默认 |
| `/reasoning <low或medium或high或xhigh或max或inherit>` | 思考强度 |
| `/fast <on或off或inherit>` | service tier |
| `/context <standard或1m或inherit>` | 上下文配置 |
| `/lang <zh或en>` `/verbose <on或off>` | 语言、工具进度提示 |
| `/approvals <untrusted或on-request或never或guardian-subagent或inherit>` | 当前用户审批策略 |
| `/approve` `/approve-session` `/deny` `/cancel` | 处理当前用户的待决审批 |
| `/plan <on或off>` `/execute-plan` `/keep-planning` `/cancel-plan` | 规划和执行保存的 proposed_plan |
| `/compact` | 手动压缩当前用户前台会话 |
| `/retry` | 重新排队当前用户最近失败的原始请求（重启后仍可用） |
| `/memory search <query>` `/memory get <id>` | 搜索、读取当前用户记忆 |
| `/memory add <text>` `/memory update <id> <text>` `/memory delete <id>` | 新增、以 supersede 修改、软删除 |

单条 memory 最多 300 字符。自然语言记忆和定时请求可由 Codex 使用 owner-bound 动态工具完成。跨用户 ID 不会授予访问。

定时命令见 [scheduler.md](scheduler.md)。rollout 导入、载入后台、命令宏和二进制自更新功能已移除。
