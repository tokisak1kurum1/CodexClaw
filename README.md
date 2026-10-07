> 当前版本通过 Unix socket WebSocket 直连官方 Codex daemon，并使用 SQLite 持久化队列和统一两并发调度。请先阅读 [当前架构](docs/architecture.md)、[配置](docs/configuration.md) 与 [命令](docs/commands.md)；旧的独立 worker、导入和自动更新功能已经移除。

<div align="center">

<img src="./assets/banner.svg" width="600" alt="CodexClaw">

由 [OpenAI Codex App-Server](https://developers.openai.com/codex/app-server) 驱动的 QQ 私人 AI 助理

*Read this in: [English](README_en.md) | [中文](README.md)*

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-edition%202024-orange.svg)](https://www.rust-lang.org/)

</div>

---

CodexClaw 是一个构建于 Codex App Server 上、接入 QQ 官方机器人平台的私人 AI 助理。它允许你通过 QQ 来操作你电脑上的 Codex，从而完成各种任务。

## 目录

- [宇宙级安全声明](#宇宙级安全声明)
- [为什么你又整了个 Claw？](#为什么你又整了个-claw)
- [功能亮点](#功能亮点)
- [快速开始](#快速开始)
- [命令速查](#命令速查)
- [定时任务](#定时任务)
- [运行时文件](#运行时文件)
- [文档](#文档)
- [贡献指南](#贡献指南)
- [给 Codex 看的部署说明](#给-codex-看的部署说明)

## 宇宙级安全声明

本项目*<u>**完全以 Vibe Coding 方式完成**</u>*，现以"按现状提供"为原则开源或分发。作者、贡献者及相关第三方不对本项目的**可用性、正确性、安全性、适用性、持续维护状态**或与任何特定用途的兼容性作出任何明示或默示保证。

使用者应自行完成代码审查、环境隔离、权限控制、依赖审计、数据备份与上线验证，并自行判断本项目是否符合其所在地区、平台规则、组织制度及业务场景中的法律、合规、安全和运维要求。

本项目可能**调用外部服务、读写本地文件、处理聊天消息**、上传或下载附件，并可能因模型输出、配置错误、依赖缺陷、平台接口变化或操作失误导致服务中断、数据泄露、误操作、额外费用、账号处罚或其他直接、间接、附带、特殊、惩罚性损失。除法律强制规定外，作者与贡献者对此不承担责任。

在部署或使用本项目时，您**应妥善保管各类账号凭据、访问令牌、聊天数据与服务器权限，并自行承担由此产生的全部风险与后果**。若您不同意上述条件，请不要部署、复制、修改或使用本项目。

## 为什么你又整了个 Claw？

1. 我很讨厌 OpenClaw，它像个玩具，太重，而且一上来就要我电脑的完全访问权限；
2. 比起 OpenClaw，Codex 的模型和 Harness 都很棒，而且 OpenAI 正在积极地维护它；
3. Codex 订阅给的额度真的很多 👍；
4. 这个项目在蹭 OpenClaw 的热度 : (

一些其他的碎碎念可以看这篇 [Blog](https://rhapsody0x1.github.io/p/about-codex-claw)。

## 功能亮点

- 通过 Unix socket WebSocket 直连固定版本的官方 Codex daemon；断线只重连，不重启 daemon。
- 全局两个 Codex 工作槽，每用户一个；审批、中断、设置、会话和目录按用户隔离。
- SQLite durable inbox/outbox、QQ 消息去重、重启恢复与失败投递重试。
- 空闲时首条消息立即启动；回复期间收到的多条普通消息保留在持久化 inbox，并在当前回复结束后合并为下一轮。
- 保留前台/后台、保存、重命名和当前用户会话恢复；模型菜单来自 model/list。
- 纯提醒和 ephemeral Codex 定时任务，带宽限窗口，不补跑历史时间点。
- 按用户隔离的记忆增改删、FTS 历史检索、批量/空闲提炼；IDENTITY / CHARACTER / BEHAVIOR 职责分离。
- 附件大小、配额和保留期限制；未完成请求引用的附件不清理。

## 快速开始

安装配置指定的 Codex 版本（当前 0.159.2），在配置的 `CODEX_HOME` 准备认证，并手工准备 daemon：

```bash
export CODEX_HOME="$HOME/.codex-claw/.codex"
mkdir -p "$CODEX_HOME/app-server-daemon"
printf '%s\n' '{"remoteControlEnabled":false,"shutdownGraceSeconds":60,"updater":{"autoUpdateEnabled":false,"updateIntervalMinutes":120}}' > "$CODEX_HOME/app-server-daemon/settings.json"
codex app-server daemon update --from-cli -y
codex app-server daemon start
codex app-server daemon version
```

在 [QQ 开放平台](https://q.qq.com/) 创建机器人并启用单聊事件。复制 `config/codexclaw.example.toml` 到 `~/.codex-claw/codexclaw.toml`，填写 AppID/AppSecret。

```bash
CARGO_INCREMENTAL=0 cargo build --release --locked
CODEX_CLAW_CONFIG=~/.codex-claw/codexclaw.toml ./target/release/codex-claw
```

CLI、daemon 版本必须与 pin 一致。CodexClaw 不安装、启动或更新 daemon。机器人升级由人工 git pull/checkout、release 构建和服务重启完成。

## 命令与数据

使用 `/help` 或 `/help all`；详见 [命令](docs/commands.md)、[定时任务](docs/scheduler.md)、[配置](docs/configuration.md) 和 [部署](docs/getting-started.md)。

状态统一存于 `~/.codex-claw/data/state.db`。工作目录与附件位于 `~/.codex-claw/users/<hashed-user-id>/workspace/` 和 `inbox/`。角色配置由 `~/.codex-claw/BEHAVIOR.md`、`IDENTITY.md` 与 `CHARACTER.md` 组成：IDENTITY 只定义身份，CHARACTER 定义人物背景、性格、关系与台词样例，BEHAVIOR 定义聊天和任务行为。`BEHAVIOR.md` 缺失时使用编译进二进制的默认中文聊天/任务/anti-slop 规则；创建同名文件即可覆盖。三份角色文件只在新会话创建时注入；旧 `SOUL.md` 不再读取。

旧会话/任务 JSON 与 USER.md/MEMORY.md 只迁移一次，保留 `.legacy.bak` 备份。已删除 Codex rollout 导入、二进制自更新、命令宏、交互式定时会话和 ShadowWorker。

## 开发

详见 [架构](docs/architecture.md) 和 [CONTRIBUTING.md](CONTRIBUTING.md)。CI 包含格式检查、clippy 和单元测试；真实认证 daemon smoke 使用显式工作流任务，Codex pin 升级要求其成功结果。

许可证：[MIT](LICENSE)。
