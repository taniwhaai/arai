# Update: First-class Grok TUI Support (Core Implementation)

This is a substantial update to the starter plan in PR #123.

## Summary

Arai now has working first-class support for the Grok Build TUI (supergrok), in addition to existing Claude Code support.

### Changes

- **Tool name normalization** (`guardrails::normalize_tool_name`)
  - Maps Grok tool names (`run_terminal_cmd`, `search_replace`, `read_file`, etc.) to Arai's internal canonical names.
  - Single place to maintain provider-specific aliases going forward.

- **Host detection + dual response format**
  - Detects Grok via `GROK_HOOK_EVENT` / `GROK_SESSION_ID` environment variables (following the existing pattern for `ARAI_*` flags).
  - Emits correct response shapes:
    - Grok: `{"decision": "allow|deny", "reason": "...", "additionalContext": "..."}`
    - Claude: original `hookSpecificOutput` + `permissionDecision` (bit-for-bit unchanged for existing users).

- **Native registration**
  - `arai init` now writes (or merges into) `.grok/hooks/arai.json` in addition to the existing `.claude/settings.json` path.
  - `arai deinit` cleans up both locations.
  - Grok's strong compatibility layer (it loads `.claude/settings.json`) means users get value even before switching to native hooks.

- **AGENTS.md family discovery**
  - Full support for `AGENTS.md`, `Agents.md`, `AGENT.md`, `agents.md` (project-level and global `~/.grok/`).
  - `is_instruction_file()` updated so FileChanged/InstructionsLoaded events trigger rescans.

- **Tests & Verification**
  - New smoke test exercising Grok-shaped payloads + environment.
  - Full `cargo test` green.
  - All hot-path, skip-path, and nomatch-path benchmarks pass with no regression (hot path median remains ~2.1 ms).

- **UX & Docs**
  - `arai status` now has a clean "Integration" section showing Claude + Grok TUI support.
  - Init success message improved.
  - README.md and CLAUDE.md updated with accurate language about Grok hook support (both now first-class for blocking).
  - Created real `AGENTS.md` at project root with high-value, enforceable rules for AI agents working on this codebase (Taniwha discipline, mandatory plan mode, protecting `.taniwha/` state, subagent patterns, etc.). This is real dogfooding of the Grok TUI integration — the file is written so that once Arai is installed, an AI agent will actually be guarded by these rules.

## Design Principles Followed

- **Minimal impact** — Claude Code path is untouched.
- **Zero regression** — All existing Claude behavior preserved.
- **Hot path safety** — Fail-closed behavior, normalization is cheap.
- **Testability** — `match_hook` remains pure; new paths exercised in integration tests.

## How to Test (as a Grok TUI user)

1. `arai init`
2. Check that `.grok/hooks/arai.json` was created (or hooks appear in the Grok `/hooks` modal via the compatibility layer).
3. Add a simple prohibitive rule to your `AGENTS.md` (e.g. "never force push to main").
4. Trigger a matching tool call — it should be blocked with a clear reason.

## Next (out of scope for this PR)

- More sophisticated parser improvements for agent-frequent rule patterns.
- Further dogfooding of the integration on this repo (we've already started with a strong `AGENTS.md` containing real, enforceable rules for AI agents).
- Optional MCP/skill surface for arai commands.

## Current Status

Core implementation + tests + benchmarks + meaningful dogfooding via `AGENTS.md` are complete on the branch. Ready for review and merge of the first-class Grok TUI support.

Refs: #122

🤖 This work was done with the explicit goal of making Arai actually usable by AI coding agents (including the implementor) in the Grok TUI.