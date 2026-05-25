# TODO: Grok TUI / supergrok Integration for Arai

**Status**: Planning phase — issue + starter PR to begin the work. This file tracks the immediate meta-task and the subsequent implementation plan.

**Date**: 2026-05-26 (approx, from context)
**Owner**: Grok (in TUI session) + maintainers
**Related**: Arai's existing Claude Code hook + discovery integration; prior Cursor pairing work in tasks/cursor-pairing.md

## Immediate Meta-Task (this session)
- [x] Explore .claude/skills/ (Taniwha orchestration, Kupu phases, state layout, re-raise protocol)
- [x] Research Grok TUI docs (hooks.md, skills.md, mcp-servers.md, project-rules.md, subagents, plan-mode)
- [x] Analyze Arai hook handler (hooks.rs), init injection (init.rs), discovery, guardrails tool-name handling, output formats
- [x] Confirm Kupu MCP unavailable in this session (search_tool empty; prior "connection failed")
- [ ] Write this plan to tasks/todo.md
- [ ] Create GitHub issue for "Support Grok TUI / supergrok as first-class hook target"
- [ ] Create feature branch + initial commit (plan + any immediate compatible tweaks) + open PR
- [ ] Use Kupu bash fallbacks (new_ulid.sh, now.sh, event_path.sh) to record a decision/event for starting this work under .taniwha/kupu/
- [ ] User verification of plan before full implementation

## Why This Work
Arai's core value is zero-noise, domain-specific guardrail enforcement on tool calls and prompts via the standard hook stdin/stdout protocol (PreToolUse deny, UserPromptSubmit summaries, FileChanged rescan, etc.).

- Grok TUI (supergrok) has a **hooks system that is deliberately Claude-compatible** in event names, JSON shape on stdin (hook_event_name / tool_name snake_case in practice, per test samples), matcher syntax, and command-hook execution.
- Grok explicitly loads `~/.claude/settings.json` and `.claude/` for compatibility, so `arai init` "just works" today for many users.
- However, full native support is needed for:
  - Project-scoped `.grok/hooks/*.json` (trust model, .grok/ dir)
  - Correct response format for native Grok hooks (`{"decision":"deny","reason":"..."}` + exit 2 vs. Claude's `hookSpecificOutput.permissionDecision`)
  - Tool name normalization (Grok internals: `run_terminal_cmd`, `search_replace`, `read_file` vs. Arai's internal "Bash"/"Edit"/"Read")
  - Discovery of Grok's project-rules files: `AGENTS.md`, `Agents.md`, `AGENT.md` (in addition to CLAUDE.md)
  - Environment detection (GROK_* vs CLAUDE_* vars) for host-specific behavior, audit fields, etc.
  - Future: possible MCP exposure of arai_* tools, or a Grok skill for "arai: why", "arai status"
  - Parity with Claude for `arai init`, `arai deinit`, `arai guardrails --match-stdin`
  - Leverage Grok's subagents/skills/plan-mode in Arai's own development (Taniwha already models this)

Zero breaking changes for existing Claude users. Detection + graceful dual support.

## High-Level Plan (for the feature PRs)
1. **Hook payload normalization & dual-format output** (hooks.rs)
   - Add `normalize_tool_name()`: map Grok names → Arai internal (run_terminal_cmd→Bash, search_replace→Edit, read_file→Read, Write→Edit, etc.). Keep Claude names as identity.
   - Update `extract_terms`, `should_skip_tool` call sites, and `match_hook` to normalize early.
   - In `handle_stdin` (and the fail-closed error path): detect host via `GROK_HOOK_EVENT` / `GROK_SESSION_ID` env (or absence of CLAUDE_*), emit the correct stdout JSON:
     - Grok: `{"decision": "allow"}` or `{"decision": "deny", "reason": "..."}`
     - Claude (current): the hookSpecificOutput shape (keep exact for compat)
   - Update `known_hook_event` if needed; add tests for both payload shapes.
   - Preserve the existing 22-32ms hot-path performance.

2. **Init / deinit / hook registration for Grok locations** (init.rs, config.rs)
   - Add `grok_hooks_dir()` / `grok_settings_compat_path()` etc. in Config.
   - `arai init`: after (or alongside) Claude injection, write or merge a `arai.json` into:
     - Project: `<root>/.grok/hooks/arai.json` (preferred for new projects)
     - Fall back / also support global `~/.grok/hooks/arai.json` (with user opt-in)
   - Use the same hook JSON structure (it is compatible).
   - `arai deinit`: remove Arai entries from both .claude/settings.json AND any .grok/hooks/*.json files that contain them.
   - Make injection idempotent and trust-aware (document that project .grok/hooks requires `/hooks-trust` or equivalent in Grok).
   - Update `arai status` and `arai init` output to mention both hosts.

3. **Discovery & instruction file support** (discovery.rs, hooks.rs::is_instruction_file, parser tests)
   - Add AGENTS.md, Agents.md, AGENT.md, AGENTS.md (and perhaps .grok/AGENTS.md) to project + global discovery (parallel to CLAUDE.md).
   - Extend `is_instruction_file()` to trigger rescan on those basenames and under .grok/ equivalents of rules/ dirs if they appear.
   - Update parser_coverage corpus + integration test.
   - Consider .grok/project-rules or similar if Grok evolves a directory form.

4. **Tests, scenarios, docs, CLI**
   - Add Grok-specific test payloads in `tests/hooks_safety.rs` and prompt_collector tests.
   - New scenario or `arai test` case exercising Grok-shaped input + expected output format.
   - Update CLAUDE.md, README.md, `arai --help` / man pages with Grok support notes.
   - `arai why` already uses the pure `match_hook` — it will benefit automatically once normalization is in.
   - Consider a `--host grok|claude|auto` flag for `guardrails --match-stdin` (advanced / testing).
   - Audit entries: add optional `host: "grok" | "claude"` field (additive, older entries null).

5. **Optional / follow-up (not for initial PR)**
   - MCP server exposing `arai_list_guardrails`, `arai_why`, `arai_add_rule` (similar to existing arai MCP server in mcp.rs).
   - A bundled Grok skill (`~/.grok/skills/arai/` or project) for common arai commands.
   - Telemetry differentiation.
   - Full Taniwha-tracked implementation of the above using design-doc → contracts → leaf + verifier (leveraging the existing .taniwha/ and skills in this repo).

## Acceptance Criteria (for the feature to be "done")
- `arai init` on a fresh checkout registers working hooks that Grok TUI loads (visible in /hooks or Ctrl+L) and that fire on PreToolUse for Bash-equivalent.
- A Block-severity rule actually denies a matching `run_terminal_cmd` under Grok (end-to-end).
- UserPromptSubmit summaries and FileChanged rescans work.
- No regression on Claude Code (full test matrix + manual).
- `cargo test` green; hot-path benches not regressed >5%.
- Docs updated; `arai status` reports "Grok + Claude compatible".
- Issue #XXX closed by the PR.

## Risks / Open Questions
- Exact payload field casing under real Grok hook invocation (samples use snake_case; user-guide example used camelCase — normalization must be defensive).
- Whether Grok's compatibility layer for .claude/settings.json passes the hook output through a translator or expects native format when the hook JSON came from that source.
- Performance: one extra string map on hot path is fine.
- Security: same as existing (hooks run with user perms; project hooks require explicit trust in Grok too).

## Next Steps After Starter PR
- User reviews plan + approves (or provides adjustments).
- Full implementation via Taniwha if desired (dispatch design-doc for this feature, derive contracts, etc.), or direct leaf PRs.
- Capture lessons in tasks/lessons.md after any corrections.

## Kupu Logging (for this meta-task)
Will use bash fallbacks (since no kupu.* MCP tools in session) to append a `work_started` event + decision record under the existing .taniwha/kupu/ tree, following state-layout.md + kupu-phases.md conventions. This records "Grok TUI integration planning initiated via direct session + gh issue/PR".

## Implementation Progress (as of late May 2026)

Core Grok TUI support implemented and verified on `feat/grok-tui-support`:

- Tool name normalization (`normalize_tool_name`) + centralized `CANONICAL_TOOLS`.
- Host detection (`GROK_*` vs `CLAUDE_*` env vars) + clean dual response format (Grok flat `{"decision"}` vs Claude `hookSpecificOutput`).
- `arai init` / `deinit` now handle both `.claude/settings.json` and `.grok/hooks/arai.json`.
- AGENTS.md family discovery (project + global `~/.grok/`) + `is_instruction_file` updates.
- New Grok smoke test in `tests/hooks_safety.rs`.
- `arai status` and init output now mention Grok TUI support.
- README.md and CLAUDE.md lightly updated.
- Full `cargo test` + all three hot-path benchmarks (`hot_path.sh`, `skip_path.sh`, `nomatch_path.sh`) pass with no regression (hot path median ~2.1 ms).

All changes follow minimal impact + zero regression for existing Claude users.

Next: Light polish on `arai status` messaging if needed, then focused PR update to #123.

---
*This plan follows the repo's CLAUDE.md workflow: plan first in tasks/todo.md, verify before deep changes, use Kupu (fallback) for mechanical state, minimal impact, no laziness on root cause (dual host support + normalization).*
