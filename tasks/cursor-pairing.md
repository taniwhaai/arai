# Cursor pairing — better than advisory

Plan for shipping four layered improvements to Arai's Cursor (and other
non-Claude MCP-client) story.  Self-contained brief — pick this up cold,
execute, push.

## Context

- Repo: `github.com/taniwhaai/arai` — clone at `C:/Users/tim/Documents/GitHub/arai`.
- Workflow: push directly to `main` (per `feedback_arai_pr_flow.md`).  No
  PR, no feature branch.
- Build/test: WSL with cargo at `/home/tim/.cargo/bin/cargo`.  Use
  `wsl -- bash -lc "cd /mnt/c/Users/tim/Documents/GitHub/arai && cargo test"`.
- Test count baseline (pre-this-work): **277** across all binaries.
- Bench harness lives at `bench/hot_path.sh`, `bench/skip_path.sh`,
  `bench/nomatch_path.sh`.
- Spread across the day — user explicitly does not want all four
  pushed in a single commit.  Plan: **four commits, four pushes**, one
  per item below.  Verify tests green between each push.

## Why this

Real-time blocking via PreToolUse hook is Claude-Code-only and that's a
client-side limitation, not an Arai gap.  The four items below close the
distance between "advisory" and "best-effort enforcement" on Cursor /
Cline / Aider / any future MCP-capable editor:

1. Native rule generation — Cursor's own loader injects them
2. MCP resources — Cursor reads them into context proactively
3. MCP prompts — slash-commands the user can invoke
4. Tool description tweak + outcome recording — soft self-gate

## Commit 1 — Generate `.cursor/rules/*.mdc` from classified rules

**Goal.** When the user runs `arai sync-cursor-rules`, write Cursor-native
MDC files derived from the classified rule set.  Cursor's own loader then
injects them with proper scope and timing — no MCP roundtrip, no model
cooperation needed.

**Cursor MDC frontmatter** (per cursor.com/docs):

```mdc
---
description: <short, one-line, used by "Agent Requested" mode>
globs: <comma-separated path patterns; triggers "Auto Attached" mode>
alwaysApply: <bool>
---

<markdown rule body>
```

Three Cursor application modes:
- **Always Apply** (`alwaysApply: true`) — every prompt
- **Auto Attached** (`globs: "..."`) — when files matching are referenced
- **Agent Requested** (`description: "..."` only) — model decides

**Mapping Arai → Cursor.**

| Arai shape | Cursor mode | File |
|---|---|---|
| Severity = Block, any tool scope | Always Apply | `arai-block.mdc` |
| Severity = Warn/Inform, intent.tools narrow (e.g. `["Bash"]` for python) | Auto Attached, globs derived from tools | `arai-tool-{tool}.mdc` |
| Severity = Warn/Inform, intent.tools = `["*"]` | Agent Requested | `arai-general.mdc` |

Tool → glob mapping (initial set, extend as needed):

| Tool / subject | Globs |
|---|---|
| python / pip / poetry / pytest | `**/*.py` |
| node / npm / yarn / pnpm / jest | `**/*.{js,ts,jsx,tsx}` |
| cargo / rust | `**/*.rs` |
| go | `**/*.go` |
| docker | `**/Dockerfile,**/docker-compose.*` |
| terraform | `**/*.tf` |
| sql / alembic | `**/migrations/**,**/*.sql` |

Fallback when no clear glob: write to `arai-general.mdc` (Agent Requested).

**File ownership.**  Every generated file starts with:

```
<!-- arai-generated: do not edit; regenerate with `arai sync-cursor-rules` -->
```

Regeneration deletes ONLY files containing that marker — leaves
hand-written `.cursor/rules/*.mdc` untouched.  Use a manifest at
`{arai_base}/cursor-sync-manifest.json` listing the files we wrote so
cleanup is reliable even if the marker is missing.

**CLI surface.**

```
arai sync-cursor-rules            # generate / regenerate
arai sync-cursor-rules --dry-run  # preview, no writes
arai sync-cursor-rules --clean    # remove arai-owned files only
```

Don't auto-trigger on `arai scan` yet — let users opt in until the
mapping shape stabilises.

**Files to touch.**
- `src/main.rs` — new `Commands::SyncCursorRules { dry_run, clean }` variant + dispatch
- `src/cursor_sync.rs` — new module: `generate(cfg, db, opts) -> Result<SyncReport>`
- `src/store.rs` — possibly a helper to bucket guardrails by tool scope
- `tests/cursor_sync.rs` — integration test: seed project with mixed rules, run sync, parse generated frontmatter, verify Cursor-shape

**Tests.**
- Each application mode produces correct frontmatter shape (alwaysApply
  vs globs vs description-only)
- Re-running sync is idempotent (same input → same output, no churn)
- `--clean` removes only arai-owned files; hand-written files survive
- `--dry-run` writes nothing
- Tool → glob mapping covers the table above; unmapped tool falls
  through to general

**Open questions for the day-of agent.**
- Should we regenerate on every `arai scan` automatically, or stay
  explicit?  Default: stay explicit for v1, revisit after dogfooding.
- One file per rule vs one per (severity × tool)?  Default: one per
  bucket, fewer files.  Bucket key = `(severity, tool_scope_glob)`.
- What about disabled rules (commit bd32a5d)?  They MUST NOT appear in
  generated files — `load_guardrails` already filters them out, so
  using that as the source is correct.

**Estimated diff.** ~250 net lines + ~120-line test file.

## Commit 2 — MCP resources

**Goal.** Expose Arai's data via MCP `resources/*` so Cursor reads it
into context at session start, no `tools/call` required.

**Resources to add.**

| URI | What |
|---|---|
| `arai://rules/active` | JSON: full active rule set with severity / tools / source |
| `arai://rules/recent-decisions` | JSON: last 50 firings + bypass entries from the audit log |
| `arai://compliance/last-7-days` | JSON: per-rule honored/ignored ratio over 7d |

**MCP wire-up.**  Three handler additions in `src/mcp.rs`:

1. `handle_resources_list()` returns the URI catalog with metadata
2. `handle_resources_read(uri)` returns the JSON blob for a URI
3. Add `"resources": {}` to `capabilities` in `handle_initialize`
4. Wire `resources/list` and `resources/read` into the dispatch in `run()`

The data builders mostly exist already:
- `arai://rules/active` → `db.load_guardrails()` → reformat
- `arai://rules/recent-decisions` → `audit::query()` (used by `cmd_audit`)
- `arai://compliance/last-7-days` → reuse the per-rule rollup from `cmd_stats --by-rule`

**Tests.**
- Extend `tests/mcp_check_action.rs` (or new `tests/mcp_resources.rs`) — drive an MCP session that calls `resources/list` then `resources/read` for each URI; assert valid JSON and expected schema.
- Unit test in `mcp.rs::tests` that the URI list contains all three.

**Estimated diff.** ~150 net lines + ~80-line test.

## Commit 3 — MCP prompts (slash-commands)

**Goal.** Expose user-invokable slash-commands via MCP `prompts/*`.

**Prompts to add.**

| Name | Args | Behaviour |
|---|---|---|
| `arai-check` | `tool`, `tool_input` | Same as `arai_check_action`, but invoked by user typing `/arai-check` in Cursor |
| `arai-status` | none | Returns the same summary `arai status` produces, formatted for chat |
| `arai-doctor` | none | Reports detected client capability and warns if hooks aren't available |

**`arai-doctor` substance.**  MCP doesn't carry "is this Cursor or
Claude" reliably (clientInfo is informational and easily spoofed), so
the prompt outputs:

> Arai is running. Detected client: <clientInfo.name or "unknown">.
> Real-time blocking is available **only** when this server is paired
> with Claude Code, which uses PreToolUse hooks. Other clients
> (Cursor, Cline, Aider, …) get advisory enforcement: Arai surfaces
> rules and records compliance verdicts but cannot block tool calls.
> If you're seeing this in Claude Code and the hook isn't firing,
> run `arai status` and check `.claude/settings.json`.

**MCP wire-up.**  Two handlers in `src/mcp.rs`:

1. `handle_prompts_list()` returns the prompt catalog
2. `handle_prompts_get(name, args)` returns the rendered prompt
3. Add `"prompts": {}` to `capabilities`
4. Wire `prompts/list` and `prompts/get` into dispatch

**Tests.**
- `tests/mcp_prompts.rs` integration: drive an MCP session, call
  `prompts/list`, verify all three names present; call `prompts/get`
  for each, verify rendered text shape

**Estimated diff.** ~120 net lines + ~80-line test.

## Commit 4 — `arai_check_action` description + `arai_record_outcome` tool

**Goal.** Soft self-gate via tool description, plus a way for non-Claude
clients to feed the compliance log.

**4a. Tool description tweak.**  In `handle_tools_list`, append to
`arai_check_action`'s description:

> **Call this BEFORE any Bash, Edit, Write, NotebookEdit, or other
> file-or-state-modifying tool call** that may match an active
> guardrail. The probe is read-only and does not write to the audit
> log; it gives you the matched rules and severity so you can decide
> whether to proceed, refine the action, or refuse.

This is a one-line change, free, and measurable: track the ratio of
`arai_check_action` calls to `arai_record_outcome` calls in the audit
log to see how often the model self-gates.

**4b. New `arai_record_outcome` tool.**

```jsonc
{
  "name": "arai_record_outcome",
  "description": "After taking an action that matched one or more guardrails (per arai_check_action or via a recent firing), record whether you honored the rule. This builds the compliance log for non-Claude clients where Arai cannot correlate PreToolUse with PostToolUse via hooks. Call once per matched rule per action.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "triple_id": { "type": "integer", "description": "id from arai_check_action's matched array" },
      "honored": { "type": "boolean", "description": "true if you respected the rule, false if you proceeded against it" },
      "session_id": { "type": "string", "description": "Claude Code or client session id; empty string if unknown" },
      "note": { "type": "string", "description": "Optional one-line rationale" }
    },
    "required": ["triple_id", "honored"]
  }
}
```

**Audit log shape.**  Writes a single JSONL entry with
`decision: "self_reported_outcome"`, the triple_id, honored bool, and
optional note.  `cmd_stats --by-rule` should pick these up the same way
it picks up Compliance entries — extend the rollup if needed.

**Files to touch.**
- `src/mcp.rs` — description tweak + new tool entry + dispatcher case +
  `tool_record_outcome` function
- `src/audit.rs` — possibly a new `record_self_reported_outcome` helper
  if direct write is awkward
- `src/stats.rs` — extend the per-rule compliance rollup to count
  self_reported outcomes alongside Compliance verdicts
- `tests/mcp_check_action.rs` — extend with a record-outcome roundtrip

**Estimated diff.** ~120 net lines + minor test extension.

## Test plan summary

- Run `cargo test` after each commit; expect 277 tests to grow by 8-15
  per commit (~310 total at the end of the day)
- Run `bench/hot_path.sh` once at the end to confirm the changes
  haven't regressed the hot path (none of the four touch the hook hot
  path; this is a sanity check only)
- Hand-test on a real Cursor install if possible: open a project with
  rules, run `arai sync-cursor-rules`, reload Cursor, confirm the
  generated rules appear in the model's context (Cursor surfaces
  active rules in its sidebar)

## CHANGELOG plan

One entry per commit under `## [Unreleased]`, e.g.:

```markdown
### Cursor

- Generate `.cursor/rules/*.mdc` from classified rules via `arai sync-cursor-rules`
  for native Cursor enforcement (no MCP roundtrip required)

### Mcp

- Resources surface (`resources/list` + `resources/read`) for active
  rules, recent decisions, and 7-day compliance — Cursor and other
  MCP clients now read context proactively at session start
- Slash-commands via `prompts/list` (`/arai-check`, `/arai-status`,
  `/arai-doctor`)
- `arai_record_outcome` tool for self-reported compliance on
  non-Claude clients
- Sharpen `arai_check_action` description so models self-gate before
  state-changing tool calls
```

## Workflow note for tomorrow

The user explicitly asked for spaced commits — don't bundle.  Push
each commit independently with its own CHANGELOG entry.  After commit
1 lands, run a quick `arai sync-cursor-rules` against the arai repo
itself (eat-the-dogfood test) and screenshot the generated `.cursor/`
tree — useful as a release-notes artefact.

After commit 4: announce on the README's "How it works" section that
Cursor users now have native rule injection + MCP resources + soft
self-gate, with a link to a new `docs/cursor-pairing.md` walkthrough.
That doc isn't on the critical path — write it after the four code
commits land.

## Out of scope for this push

- Auto-emit `.cursor/rules/*.mdc` on `arai scan` — wait for v1 dogfooding
- Cursor "Background Agents" / experimental APIs — too immature
- Cline / Aider parallels — same MCP shape, but their rule-injection
  surfaces differ; cover after Cursor lands
- Full lib.rs split for criterion benches — separate concern
