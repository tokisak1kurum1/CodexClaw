# 配置

加载顺序：CODEX_CLAW_CONFIG → 当前目录 codexclaw.toml → ~/.codex-claw/codexclaw.toml。以下为完整示例：

```toml
[qq]
app_id = "YOUR_QQ_APP_ID"
app_secret = "YOUR_QQ_APP_SECRET"
allowed_users = [] # 建议填写允许使用机器人的 QQ openid；空数组允许所有 C2C 用户

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

并发上限为全局最多 2，每用户、定时 agent、memory 各 1。catch_up_missed 必须为 false。模型空值表示 daemon 默认；菜单从 model/list 获取，缓存 5 分钟，并在 daemon 连接变化后刷新。QQ 用户设置只写 SQLite，不修改共享 config.toml。

BEHAVIOR.md 最多 8000 字符，IDENTITY.md 500，CHARACTER.md 16000，profile 4000，热记忆默认 4000；单条 memory 300。三份角色文件仅在新 thread 创建时注入，当前 thread 固定使用创建时的角色版本；旧 SOUL.md 不再读取。BEHAVIOR.md 缺失时使用编译内置默认规则；运行时同名文件会覆盖默认内容。附件单文件默认 32 MiB，单用户 256 MiB，24 h 清理。附件和输出文件必须位于当前用户的目录。

官方 daemon 使用相同 CODEX_HOME，必须由运维单独准备和启动。CodexClaw 仅检查固定版本并通过 Unix socket WebSocket 连接 daemon。
