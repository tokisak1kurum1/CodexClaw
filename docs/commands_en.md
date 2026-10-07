# Commands

Chinese command aliases remain available. Bare /model, /reasoning, /fast, /context, /lang, /verbose, /approvals and /plan display numbered choices; /back exits.

- Conversations: /new, /stop, /interrupt, /bg [alias], /fg [alias], /sessions, /resume [number|thread-id|alias], /save, /rename <old> <new>, /compact. Only the current user's database conversations are listed or restored.
- Settings: /model <name|inherit>, /reasoning <low|medium|high|xhigh|max|inherit>, /fast <on|off|inherit>, /context <standard|1m|inherit>, /lang <zh|en>, /verbose <on|off>. Dialog overrides precede user defaults and system defaults; QQ settings never rewrite shared Codex config.
- Approval: /approvals <untrusted|on-request|never|guardian-subagent|inherit>, /approve, /approve-session, /deny, /cancel.
- Plans: /plan <on|off>, /execute-plan, /keep-planning, /cancel-plan.
- Memory: /memory search <query>, get <id>, add <text>, update <id> <text>, delete <id>. Entries have a 300-character limit; updates supersede and deletion is soft. Dynamic tools are bound to the active owner.
- Status/help: /status, /help. /retry requeues the current owner’s last failed original request, including after restart.

See [scheduler_en.md](scheduler_en.md) for scheduled tasks. Rollout import, command macros and binary self-update were removed.
