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

- Feature work for the Grok TUI integration lives on `feat/grok-tui-support`.
- Keep the main integration changes focused. Large new features (deeper parser improvements, full dogfooding guardrails, etc.) should be discussed before expanding scope.

## Using Arai (Dogfooding)

Once the Grok TUI integration is merged and installed:
- Run `arai init` in this repo to start enforcing these rules via native Grok hooks.
- Treat violations of the rules in this file as high-severity (many are `never` / `must` style).
- Use `arai why`, `arai status`, and `arai audit` to inspect and improve compliance.

## Changes That Affect Guardrails

- Any change that touches hook handling, tool name normalization, response formats, matching logic, or discovery **must** include or update tests (especially in `tests/hooks_safety.rs`).
- When adding new instruction file support (new basenames or directories), update both `discovery.rs` and `hooks.rs::is_instruction_file`.

## Grok TUI Integration

- The Grok TUI support (normalization, dual response formats, native `.grok/hooks` registration) must remain fully functional and low-regression for Claude Code.
- When modifying host detection or response emission logic, ensure both Grok and Claude shapes are preserved and tested.
- The `AGENTS.md` file itself should be treated as a first-class instruction file for Grok users of this repo.
- Do not bypass Arai hooks when they are active in this environment (e.g., do not set `ARAI_DISABLED=1` or `ARAI_DENY_MODE=off` without explicit justification and logging).
- When the Grok TUI integration is active, prefer using native `.grok/hooks/` registration over relying solely on the Claude compatibility layer for this project.
- When using `arai why` or `arai audit` during development, treat the output as authoritative for understanding why a rule fired.
- After landing changes to the Grok TUI integration, run `arai init` locally in the Grok TUI and verify that the new hooks are active and the rules in this file are being respected.
- Use the Grok TUI + Arai combination to enforce the rules in this file on yourself during development of this project.

These rules are here to protect the integrity of the project. Violating them has real downstream cost.