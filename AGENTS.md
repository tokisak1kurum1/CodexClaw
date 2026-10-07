# Repository Guidelines

## Project Structure

See docs/architecture.md for the current daemon proxy + SQLite architecture. Keep Dialogs as the session topology state machine. state owns SQL/migration; work_queue owns bounded scheduling; app composes inbox/outbox, controls and owner-bound ActiveTurns; codex/app_server owns wire parsing and proxy reconnection; memory owns atomic CRUD/injection/ephemeral distillation; scheduler owns reminder/agent occurrences. Do not reintroduce self-update, rollout import, aliases, codex exec or independent shadow/scheduler daemons. Keep store APIs owner-bound and user workspace roots separate.

## Build, Test, and Development Commands
Use standard Cargo workflows from the repo root:

- `cargo check --all-targets` verifies the crate quickly without producing a release binary.
- `cargo test --all-targets` runs unit, integration, and doc tests.
- `cargo test --test app_server_smoke -- --ignored --nocapture` runs the ignored app-server smoke test when a real Codex app-server path is needed.
- `cargo fmt` applies Rust formatting.
- `cargo clippy --all-targets --all-features` catches common lint issues before review (must be 0 warnings).
- `cargo run` starts the bot with `codexclaw.toml` in the current directory.
- `CODEX_CLAW_CONFIG=./config/codexclaw.example.toml cargo run` runs with an explicit config path.
- `codex-claw cron add|once|list|rm|pause|resume|run-now|tail` manages scheduled tasks from the CLI when the binary is on `PATH`.

## Coding Style & Naming Conventions
Follow `rustfmt` defaults: 4-space indentation, trailing commas where formatter inserts them, and one module per file. Prefer `snake_case` for functions, modules, and test names, `PascalCase` for types, and concise enums/structs that mirror QQ or Codex payloads. Keep async boundaries explicit and return `anyhow::Result` at application edges where the project already does so.

Use `#[serde(default)]` on all new fields added to persisted types to maintain backward compatibility with existing on-disk state.

## Testing Guidelines
Write async tests with `#[tokio::test]` when exercising runtime behavior. Prefer focused unit tests beside the owning module; keep real app-server or end-to-end smoke checks in `tests/app_server_smoke.rs` and mark them ignored unless they are safe for default CI. Use descriptive names such as `qq_text_send_falls_back_to_plain_text_when_markdown_is_rejected`. Mock network calls with `wiremock` and temporary filesystem state with `tempfile`. Scheduler changes need more than green tests: explicitly review timeout cancellation, foreground restoration, interactive cleanup, delivery fallback, file-locking behavior, and one-shot lifecycle semantics.

## Scheduler & Cron Operations

Tasks and unique occurrence runs live in SQLite. Use the minimal CLI or dynamic schedule tools, never hand-edit legacy JSON. Maintain grace windows, no catch-up, cancellation, and transactional final result/outbox/run completion. All agent work uses WorkQueue and the official daemon proxy.

## Commit & Pull Request Guidelines
Follow the Conventional Commits style established in the repository: use prefixes like `feat`, `fix`, `refactor`, `doc` with an optional scope in parentheses (e.g., `feat(scheduler): add cron support`). Keep each commit scoped to one logical change. Pull requests should describe the behavior change, list the commands you ran (`cargo test`, `cargo clippy`), link related issues, and include screenshots only when README or user-facing message formatting changes.

## Configuration & Security Tips
Do not commit real QQ credentials or Codex auth material. Keep secrets in a local TOML file and load it with `CODEX_CLAW_CONFIG`; config loading checks that variable first, then `./codexclaw.toml`, then `~/.codex-claw/codexclaw.toml`. Treat `data/` and `~/.codex-claw/.codex/` as sensitive runtime state: they can contain session settings, downloaded attachments, scheduler jobs, cron workspaces, run logs, pending deliveries, memory notes, skill files, and copied Codex config/auth files. Inspect and back up these directories before deleting or sharing them. Scheduled Codex tasks can execute within the configured sandbox and store prompts and outputs, so avoid placing credentials in prompts, messages, job metadata, or run output.
