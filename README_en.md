> This version connects directly to the official Codex daemon over WebSocket on its Unix socket, with SQLite durable queues and a shared two-slot scheduler. See the updated architecture, configuration and command documentation for current behavior.

<div align="center">

<img src="./assets/banner.svg" width="600" alt="CodexClaw">

A private QQ AI assistant powered by [OpenAI Codex App-Server](https://developers.openai.com/codex/app-server)

*Read this in: [English](#table-of-contents) | [中文](README.md)*

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-edition%202024-orange.svg)](https://www.rust-lang.org/)

</div>

---

CodexClaw is a private AI assistant built on the Codex App Server and connected to the QQ official bot platform. It allows you to operate Codex on your computer through QQ to complete various tasks.

## Table of Contents

- [Universe-Level Security Disclaimer](#universe-level-security-disclaimer)
- [Why Did You Make Another Claw?](#why-did-you-make-another-claw)
- [Feature Highlights](#feature-highlights)
- [Quick Start](#quick-start)
- [Command Cheat Sheet](#command-cheat-sheet)
- [Scheduled Tasks](#scheduled-tasks)
- [Runtime Files](#runtime-files)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [Deployment Instructions for Codex](#deployment-instructions-for-codex)

## Universe-Level Security Disclaimer

This project was *<u>**entirely built using Vibe Coding**</u>* and is now open-sourced or distributed on an "as-is" basis. The author, contributors, and related third parties make no express or implied warranties regarding the **availability, correctness, security, suitability, ongoing maintenance status**, or compatibility with any particular use of this project.

Users should independently perform code review, environment isolation, permission control, dependency auditing, data backup, and deployment verification, and judge for themselves whether this project complies with the laws, regulations, security requirements, and operational standards of their region, platform rules, organizational policies, and business scenarios.

This project may **call external services, read/write local files, process chat messages**, upload or download attachments, and may cause service interruptions, data leaks, unintended operations, additional charges, account penalties, or other direct, indirect, incidental, special, or punitive losses due to model output, configuration errors, dependency defects, platform API changes, or operational mistakes. Except where required by law, the author and contributors bear no responsibility for such consequences.

When deploying or using this project, you **should properly safeguard all account credentials, access tokens, chat data, and server permissions, and bear all risks and consequences arising therefrom**. If you do not agree to the above conditions, please do not deploy, copy, modify, or use this project.

## Why Did You Make Another Claw?

1. I really dislike OpenClaw -- it feels like a toy, is too heavy, and demands full access to my computer right from the start;
2. Compared to OpenClaw, Codex has a great model and harness, and OpenAI is actively maintaining it;
3. The credits you get with a Codex subscription are truly generous;
4. This project is riding on OpenClaw's popularity : (

Some other ramblings can be found in this [Blog](https://rhapsody0x1.github.io/p/about-codex-claw).

## Feature Highlights

- Direct WebSocket-over-UDS connection to the pinned Codex daemon, with connection recovery.
- Two global Codex slots and one per user; owner-bound approval, interrupt, settings and workspaces.
- SQLite durable inbox/outbox, QQ event deduplication and restart recovery.
- The first idle message starts immediately; ordinary messages received during a reply stay in the durable inbox and are merged into the next follow-up turn.
- Foreground/background sessions, save, rename and owner-only resume; dynamic model menus.
- Plain reminders and ephemeral Codex tasks, with grace windows and no historical catch-up.
- Owner-bound memory CRUD, FTS history search and batch/idle distillation; separate IDENTITY / CHARACTER / BEHAVIOR responsibilities.
- Attachment quotas and retention that protects unfinished requests.

## Setup

Install the configured Codex version (currently 0.159.2), prepare authentication in the configured `CODEX_HOME`, then prepare the daemon manually:

```bash
export CODEX_HOME="$HOME/.codex-claw/.codex"
mkdir -p "$CODEX_HOME/app-server-daemon"
printf '%s\n' '{"remoteControlEnabled":false,"shutdownGraceSeconds":60,"updater":{"autoUpdateEnabled":false,"updateIntervalMinutes":120}}' > "$CODEX_HOME/app-server-daemon/settings.json"
codex app-server daemon update --from-cli -y
codex app-server daemon start
codex app-server daemon version
```

Create a bot on the [QQ platform](https://q.qq.com/) and enable direct-message events. Copy `config/codexclaw.example.toml` to `~/.codex-claw/codexclaw.toml` and fill in your credentials.

```bash
CARGO_INCREMENTAL=0 cargo build --release --locked
CODEX_CLAW_CONFIG=~/.codex-claw/codexclaw.toml ./target/release/codex-claw
```

CLI and daemon versions must match the pin. CodexClaw does not install, start or update the daemon. Update the bot manually with git pull/checkout, a release build and service restart.

## Commands and Data

Use `/help` or `/help all`; see [commands](docs/commands_en.md), [scheduler](docs/scheduler_en.md), [configuration](docs/configuration_en.md) and [setup](docs/getting-started_en.md).

State is `~/.codex-claw/data/state.db`. Workspaces and attachments are under `~/.codex-claw/users/<hashed-user-id>/workspace/` and `inbox/`. Character configuration uses `~/.codex-claw/BEHAVIOR.md`, `IDENTITY.md`, and `CHARACTER.md`: IDENTITY defines who the character is, CHARACTER holds background/personality/relationship/voice examples, and BEHAVIOR defines chat/task behavior. If `BEHAVIOR.md` is absent, CodexClaw uses the embedded default chat/task/anti-slop rules; creating the file overrides that default. Character files are injected only when a new conversation starts; legacy `SOUL.md` is no longer read.

Legacy session/job JSON and USER.md/MEMORY.md migrate once and retain `.legacy.bak` backups. Local Codex rollout import, binary self-update, command macros, interactive scheduled conversations and ShadowWorker are removed.

## Development

See [architecture](docs/architecture_en.md) and [CONTRIBUTING.md](CONTRIBUTING.md). CI includes formatting, clippy and unit tests. Authenticated daemon smoke is an explicit workflow job; a Codex pin upgrade requires its successful result.

License: [MIT](LICENSE).
