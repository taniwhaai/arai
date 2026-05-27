# Repo-layer enforcement — scoping doc

**Status:** scope, not spec. Resolves [#82](https://github.com/taniwhaai/arai/issues/82). Parent epic: [#63](https://github.com/taniwhaai/arai/issues/63).

A pre-commit (and/or server-side) hook that runs Arai's rule engine on git diffs. Universal across AI tools because it sits at the repo layer, not the tool layer.

---

## Why this matters

Today's enforcement is per-tool. Each AI tool ships its own surface (Claude Code hooks, Grok TUI hooks, Cursor MCP context, Copilot ingest-only) and the strength of enforcement tracks what the tool exposes. The matrix has gaps:

| Tool                | Pre-tool-call block | Pre-commit block (this work) |
|---------------------|---------------------|-------------------------------|
| Claude Code         | ✅ hook              | ✅                             |
| Grok TUI            | ✅ hook              | ✅                             |
| Cursor              | ⚠️ context only      | ✅                             |
| Copilot             | ⚠️ context only      | ✅                             |
| Windsurf            | ⚠️ context only      | ✅                             |
| _Any future tool_   | ?                    | ✅                             |

The repo layer catches the diff regardless of which tool produced it. Slower feedback than blocking mid-tool-call, but it's the only layer that's truly universal — and the only layer that's robust to tools that don't expose hooks at all.

The motivating case is unchanged from #82: hand-writing an alembic migration file. Whether the AI did it, the developer did it, or it slipped in via a paste from a different agent's chat, the diff hits `migrations/versions/*.py` and Arai blocks it.

---

## Rule-engine reuse path

The good news: most of the matcher is already shaped for this. `guardrails::match_guardrails` takes terms + tool name + hook event and returns matched rules. Diffs map cleanly onto the existing `tool_input` shape used by `Edit` and `Write`:

| Diff event   | Maps to                                            |
|--------------|----------------------------------------------------|
| Added file   | `Write { file_path, content }`                     |
| Modified file| `Edit { file_path, new_string, old_string }`       |
| Renamed file | `Edit { file_path = new_path }` (+ rename hint)    |
| Deleted file | _new shape needed — see schema gaps below_         |

A new module `src/repo_check.rs` (working name) would:

1. Read a git diff (from stdin or from `git diff --cached` / `git diff <base>..<head>`).
2. Parse it into per-file change descriptors.
3. For each descriptor, synthesize the equivalent `tool_input` and call `hooks::match_hook` with `tool_name = "Write"` or `"Edit"`.
4. Collect matches across all files, emit a single deny/warn/allow verdict for the commit.

Reusing `match_hook` (not just `match_guardrails`) keeps the existing severity routing, prerequisite-tracking, and audit-write paths working without duplication. Severity → exit code:

- Any `block` match → exit 1, fail the hook
- Only `warn` / `inform` matches → exit 0, print advisory to stderr
- No matches → exit 0, silent

---

## Local vs server-side vs CI

Three deployment surfaces, listed in order of feedback latency:

### 1. Local `pre-commit` hook _(start here)_

Runs on the developer's machine before `git commit` completes. Fastest feedback. Bypassable with `--no-verify` (a feature, not a bug — escape hatch for emergencies).

**Distribution:**
- **Option A — the [`pre-commit`](https://pre-commit.com) framework.** Ship a `repo` entry users add to their `.pre-commit-config.yaml`. Familiar, widely deployed, handles install + invocation. Best for adoption.
- **Option B — `arai init --pre-commit`.** Drop a script straight into `.git/hooks/pre-commit`. No third-party dependency. Worse for users already on the `pre-commit` framework.
- **Recommendation:** ship A, document B as a fallback for teams that don't use the framework.

### 2. GitHub Action on PR _(the unbypassable layer)_

Runs on every PR push. Not strictly "pre-receive" but functionally equivalent when wired as a required check on the protected branch. Cannot be bypassed by `--no-verify`. Slow feedback (post-push), but catches anything that slipped past the local hook.

**Distribution:**
- Ship a reusable workflow (`.github/workflows/arai-check-diff.yml` or a [composite action](https://docs.github.com/en/actions/sharing-automations/creating-actions/creating-a-composite-action)) so users can drop it in with two lines.
- Document the required-check setup.

### 3. Server-side `pre-receive` hook _(GHE / self-hosted only)_

True pre-receive — runs on the git server, rejects the push before it's accepted. Cannot be bypassed at all. Only available on GitHub Enterprise Server (or self-hosted git). **Defer until there's user demand** — most teams are on github.com and #2 covers them.

### Phasing

| Phase | Surface | Audience |
|---|---|---|
| 1 (v1)   | Local pre-commit (framework + manual)            | Solo developers + small teams |
| 2 (v1.1) | GitHub Action / reusable workflow                | Teams with PR review + required checks |
| 3 (v2)   | Server-side pre-receive                          | GHE / self-hosted |

Phases 1 and 2 are independent and can land in either order; both share the `arai check-diff` core.

---

## Schema gaps for diff-based matching

The canonical rules schema (`docs/rules-file-spec.md` Part 2) already covers most of what's needed:

- `when.tool = ["Write", "Edit"]` — already routes correctly with the diff→tool synthesis above.
- `when.path = ["**/migrations/versions/*.py"]` — already path-glob matched.
- `when.content_pattern = "AKIA[0-9A-Z]{16}"` — already matches against the file's new content.

What's **missing** for diff-aware matching:

| Gap | What it lets you express | Workaround today |
|---|---|---|
| `when.diff_added` (regex) | "Block if this regex appears in **added** lines specifically" — e.g. catching a new `console.log` without flagging existing ones | Use `when.content_pattern` and accept false positives on pre-existing matches |
| `when.diff_removed` (regex) | "Warn if this string is being **removed**" — e.g. removing a test assertion, deleting a TODO comment marker | None |
| `when.commit_size` / `when.files_changed` | "Block PRs touching > 50 files" — catches accidental mass changes | None |
| `when.author` / `when.branch` | Server-side: "block force-pushes from the bot account" | None |
| `then.action = "comment"` (CI mode) | "Don't block, post an inline PR comment instead" | None — CI mode would default to `block` exits |

**v1 ships without these.** The path + content + tool fields cover the alembic / secret-key / managed-file cases that motivate the work. Diff-aware fields land in phase 3 once there's user demand and concrete cases that need them.

---

## Implementation breakdown (proposed tickets)

In rough dependency order:

1. **`arai check-diff` subcommand** (core)
   - Reads diff from stdin or `--from-rev`/`--to-rev` flags.
   - Synthesizes `tool_input` per file; calls `hooks::match_hook` per synthesized call.
   - Aggregates verdicts; emits per-rule output (JSON via `--json`).
   - Severity → exit code as above.
   - Tests: parser fixtures for added / modified / renamed / deleted; round-trip a `git diff` from a real fixture repo.

2. **`pre-commit` framework integration**
   - Sample `.pre-commit-config.yaml` block in README + the project's own `.pre-commit-config.yaml`.
   - Document `arai check-diff` invocation contract (stdin shape, exit codes, env vars).

3. **`arai init --pre-commit` opt-in**
   - Writes `.git/hooks/pre-commit` script.
   - Refuses if the file already exists unless `--force`.
   - Tested against a tempdir fixture.

4. **Reusable GitHub Action**
   - Composite action under `.github/actions/arai-check-diff/`.
   - Documents required-check setup.
   - End-to-end test via `act` or a self-test workflow.

5. **Schema additions for diff-aware matching** (defer until demand)
   - `when.diff_added` / `when.diff_removed` per the gaps table.
   - Spec update to `docs/rules-file-spec.md`.
   - Parser + matcher changes.

6. **Server-side `pre-receive`** (defer indefinitely; pull when GHE users ask)
   - Bash wrapper that pipes the push diff into `arai check-diff`.
   - Install docs.

---

## Out of scope

- **Replacing the per-tool hooks.** This work _adds_ a layer; the existing PreToolUse hooks stay. Faster feedback at the tool layer is worth keeping for tools that expose it.
- **Mid-commit auto-fix.** `arai check-diff` reports verdicts; it doesn't rewrite the diff. Auto-fix is a different feature with different failure modes.
- **Diff-aware rule authoring UX.** The canonical TOML schema is what teams hand-edit; phase 3 schema additions inherit the same authoring path. No new authoring surface.

## Open questions

- **Default exit-code behaviour on CI.** Should phase 2 default to `block` exits (blocks the PR via required check) or `warn` exits + inline comments (advisory)? Probably configurable per repo via a workflow input. Worth getting maintainer / early-adopter input before phase 2 ships.
- **Diff size limits.** Big monorepo PRs can generate huge diffs. Per-file streaming is straightforward; a wall-clock budget for the whole check may be necessary to avoid CI timeouts.
- **Caching.** Subsequent pushes to the same PR may re-check the same files. Worth investigating whether to cache rule-evaluation results by `(rule_id, file_hash)` keys.
