# Host-integration status

**Grok Build (native hooks) and Codex (native hooks) have shipped.**
This file was a May 2026 planning note for first-class Grok TUI /
supergrok support. Do not treat the unchecked items below the fold as
current work.

Shipped:

- Dual-format hook output (Claude `permissionDecision` / Grok `decision` + exit 2)
- Native `.grok/hooks/arai.json` registration and `arai deinit` cleanup
- `AGENTS.md` / `Agents.md` discovery (project + `~/.grok/`)
- Tool-name aliases including live Grok `run_terminal_command` → `Bash`
- Codex `.codex/hooks.json` (PreToolUse / PostToolUse / UserPromptSubmit) in **v1.1.2**
- Per-file Codex `apply_patch` matching

Current product docs: [README](../README.md), [docs/codex.md](../docs/codex.md),
[docs/upstream/grok-hooks-reverification-1.0.0.md](../docs/upstream/grok-hooks-reverification-1.0.0.md).
Contribution flow: PR-to-main.

The original planning checklist is retained in git history (`tasks/todo.md`
on `feat/grok-tui-support` and early main commits). New host work belongs
in a GitHub issue, not this file.
