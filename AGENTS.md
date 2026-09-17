# AGENTS.md — Arai

This file contains rules that AI coding agents **must** follow when working in this repository.

These rules exist because mistakes here are expensive (corrupting Taniwha build state, breaking compartmentalization invariants, etc.).

## Core Discipline (Non-Negotiable)

- **Always enter plan mode** for any non-trivial task (3+ steps, architectural decisions, or anything that would benefit from user review before coding). Use the `enter_plan_mode` tool.
- **Never edit source files while in plan mode** except for the plan file itself.
- **Exit plan mode** with `exit_plan_mode` and present the plan for explicit user approval before making implementation changes.
- **Respect the current plan file** as the single source of truth during any planning or implementation phase. Do not improvise outside it.

## Taniwha / Subagent Rules

- When using the Taniwha compartmentalized build system (`.claude/skills/`), **always follow the documented process**:
  - Use the orchestrator/dispatcher pattern for complex work.
  - Dispatch the correct role subagents (design-doc, contract-derivation, leaf-implementation, composition, verifier).
  - Never bypass phases (especially verification).
- **Never manually mutate** files under `.taniwha/kupu/` except through the approved mechanisms (Kupu MCP tools when available, or the exact bash fallback scripts in `.claude/skills/_shared/scripts/util/`).
- Always prefer Kupu MCP tools when available. When they are not, use the canonical bash scripts (`new_ulid.sh`, `now.sh`, `event_path.sh`).

## Work Style

- **Plan first.** Write plans to `tasks/todo.md` (or the active session plan file) with checkable items before deep implementation.
- **Verify before claiming done.** Run tests, benchmarks, and manual verification. Do not mark tasks complete until they actually work.
- **Minimal impact.** Only touch what is necessary. Challenge yourself: "Is there a more elegant way with less surface area?"
- After any correction from the user, update `tasks/lessons.md` with the pattern and rules to prevent recurrence.

## Tool Usage

- When working on this repo, prefer using the tools and subagents defined in `.claude/skills/`.
- For complex multi-step work, use `spawn_subagent` with appropriate types (`explore`, `plan`, `general-purpose`, etc.) rather than trying to do everything in one context.

## Git / Branching

- Contribution flow is PR-to-main. Do not push feature work straight to `main`.
- Grok Build and Codex hook support have **shipped** (Grok in 1.1.x, Codex in 1.1.2). Do not treat `feat/grok-tui-support` as the live integration branch.
- Keep host-integration changes focused. Large new features should be discussed before expanding scope.

## Using Arai (Dogfooding)

`arai init` registers native hooks for Claude Code (`.claude/settings.json`), Grok Build (`.grok/hooks/arai.json`), Codex (`.codex/hooks.json`), and Cursor (`.cursor/hooks.json`).
- Run `arai init` in this repo so these rules are enforced on the host you are using.
- Codex: after init, enable the project hooks through `/hooks`. Writing the file does not grant host trust.
- Treat violations of the rules in this file as high-severity (many are `never` / `must` style).
- Use `arai why`, `arai status`, and `arai audit` to inspect and improve compliance.

## Changes That Affect Guardrails

- Any change that touches hook handling, tool name normalization, response formats, matching logic, or discovery **must** include or update tests (especially in `tests/hooks_safety.rs` and `tests/codex_hooks.rs`).
- When adding new instruction file support (new basenames or directories), update both `discovery.rs` and `hooks.rs::is_instruction_file`.

## Host integrations (Claude Code, Grok Build, Codex, Cursor)

- All three native PreToolUse paths must remain functional. A change that fixes one host must not regress the others.
- When modifying host detection or response emission, preserve and test Claude (`hookSpecificOutput.permissionDecision`), Grok (`decision` + exit 2), and Codex shapes.
- Prefer each host's native registration over relying on another host's compatibility layer.
- Do not bypass Arai hooks when they are active (do not set `ARAI_DISABLED=1` or `ARAI_DENY_MODE=off` without explicit justification and logging).
- When using `arai why` or `arai audit` during development, treat the output as authoritative for why a rule fired.
- After landing host-integration changes, run `arai init` on the affected host and verify the hooks are active (`arai status`, host `/hooks` UI, a harmless blocked-rule probe).
- Keep `normalize_tool_name` current with live host tool names. A missing alias is a silent fail-open. Grok Build: `run_terminal_command` → `Bash`. Codex: canonical `Bash` plus per-file `apply_patch`. Cursor: `Shell` → `Bash`, `Delete` → `Edit` (see `src/cursor.rs`).
- Do not let work on one host reduce quality or test coverage of the others.
- When under high context pressure, treat the rules in this file as more important, not less.
- When Arai fires a rule, treat it as feedback and update AGENTS.md, lessons, or the integration as needed.

These rules are here to protect the integrity of the project. Violating them has real downstream cost.