---
version: 3
captured_at: 2026-05-10T05:45:00Z
source: user_amendment
parent_version: 2
phase: phase-3-issue-74-docs-sweep
authoritative_spec: github.com/taniwhaai/arai/issues/74
---

# Brief v3 — Issue #74 docs sweep

## Additive scope

Brief v1 covers the full 26-issue arai roadmap. Brief v2 scoped the previous build to GitHub #73 (the deprecation shim, now in PR #89 awaiting merge). This v3 amendment scopes the **next build cycle** to GitHub #74: sweeping user-facing documentation to reflect the path and env-var rename that landed in PR #87.

v1 remains the master picture. v2 documents the #73 build. v3 narrows this build's work to docs-only changes.

## Why now

PR #87 (merged) renamed Arai's defaults — env var `ARAI_DB_DIR` → `ARAI_BASE_DIR`, default state path `~/.arai/` → `~/.taniwha/arai/`. The README, install instructions, and changelog still reference the old paths in places. Until those are updated, new users following docs will mis-configure their setup.

## Scope (single_module tier — docs only)

Update three files at the repo root to reflect the v0.2.15 path/env-var rename:

1. `README.md` — five references to `~/.arai` paths need updating to `~/.taniwha/arai` equivalents.
2. `llms-install.md` — one reference to `~/.arai/`.
3. `CHANGELOG.md` — add a `[Unreleased]` entry documenting the rename + deprecation shim (PR #87 + PR #89 if/when merged). **Do NOT rewrite historical entries**: line 32's `~/.arai/db/<project>.db by default` is in the v0.2.14 release section and accurately describes that release's behaviour. Historical revision is forbidden.

## In scope

- Find every `~/.arai` (or bare `.arai`) reference in user-facing markdown and update to the new path, except inside CHANGELOG entries for already-released versions.
- Add a `[Unreleased]` CHANGELOG entry under `### Changed` (or similar) describing: env var rename (`ARAI_DB_DIR` → `ARAI_BASE_DIR`, old name still honoured with stderr warning), default path change (`~/.arai/` → `~/.taniwha/arai/`, old path still honoured with stderr warning), pointer to PR #87 + #89.
- Update the docker-volume-mount example at `README.md:639` if applicable.
- Verify no broken commands, no broken links, no broken instructions.

## Out of scope (do NOT do)

- Rewriting historical CHANGELOG entries. v0.2.x release notes describe what was true at the time and are immutable history.
- Updating any code file (`*.rs`, `*.toml`).
- Updating `.taniwha/` state, `.claude/` config, or any tooling files.
- Updating user-facing strings inside source files (`src/enrich.rs` already updated in PR #87; `src/config.rs` doc comments updated in PR #87).
- Adding any new documentation sections — only updating existing references.
- Changing tone, structure, or pedagogical approach of any doc — minimum-impact edits only.

## Acceptance criteria

- AC1: `grep -r '\.arai' --include='*.md' .` returns only intentional historical references (CHANGELOG entries for already-released versions) and `~/.taniwha/arai` matches (which contain `.arai` as a substring of `taniwha/arai`).
- AC2: README.md's audit-log path references show `~/.taniwha/arai/audit/` (lines 88 + 264).
- AC3: README.md's config-file path references show `~/.taniwha/arai/config.toml` (line 141).
- AC4: README.md's docker example uses paths consistent with the new default (line 639) — exact form left to implementor's judgement; the original mounted both source and target as `.arai`, so an analogous mount for the new path is the minimum bar.
- AC5: llms-install.md mentions `~/.taniwha/arai/` instead of `~/.arai/` (line 17).
- AC6: CHANGELOG.md has a new `[Unreleased]` entry describing the rename + deprecation shim, citing PR #87 (and #89 if/when known).
- AC7: CHANGELOG.md line 32 (in the v0.2.14 release section) is unchanged. Historical entries for prior releases are unmodified.
- AC8: `cargo test` still passes (no source files were touched, so this is a sanity check, not a regression).

## Tier rationale

`single_module`: this is a documentation update across three files, all at the repo root, no module boundary changes, no contracts, no shared types. Per design-doc skill: "Skip [a multi-module design] for ... modifications that fit entirely within an existing module's contract." Docs are not strictly a "module" but the change is contained, mechanical, and additive enough that the single_module ceremony fits.

The dogfood case for this brief is to confirm the Taniwha flow can handle a docs-only change without forcing structural commitments that don't apply (no resolver function, no data shape, no test surface beyond grep verification).

## Related

- Master picture: brief/v1.md
- Previous build: brief/v2.md (#73 deprecation shim)
- GitHub epic: #61 (folder restructure)
- Lands on top of: PR #87 (path rename, merged) and conceptually pairs with PR #89 (deprecation shim, awaiting merge)
- Sibling open issues: #72 (migration prompt) — independent