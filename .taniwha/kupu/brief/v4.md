---
version: 4
captured_at: 2026-05-11T03:00:00Z
source: user_amendment
parent_version: 3
phase: phase-4-issue-72-migration-prompt
authoritative_spec: github.com/taniwhaai/arai/issues/72
---

# Brief v4 — Issue #72 migration prompt

## Additive scope

Brief v1 covers the master 26-issue arai roadmap. Briefs v2 (#73 deprecation shim) and v3 (#74 docs sweep) covered earlier build cycles, both merged. This v4 amendment scopes the **next build cycle** to GitHub #72: a first-run migration prompt offering to move `~/.arai/` to `~/.taniwha/arai/`.

## Why now (and dependence on prior cycles)

PR #87 (merged) renamed defaults. PR #89 (#73, merged) added the deprecation shim that **detects the legacy path and signals it via the `DeprecationNotice::DeprecatedDefaultPath(String)` variant** in `ResolvedBaseDir`. Today, when an existing user runs arai with data in `~/.arai/`, they get a stderr warning. They get NO offer to actually migrate.

**This cycle adds the offer.** The shim's `DeprecatedDefaultPath` variant becomes the trigger.

## Required reading from prior cycles (for the orchestrator and downstream subagents)

This brief deliberately depends on artifacts from earlier build cycles. Read these before designing:

- **`.taniwha/kupu/brief/v2.md`** — defines the deprecation shim's resolution order and the two notice variants. The trigger for THIS cycle's migration prompt is `DeprecatedDefaultPath`, NOT `DeprecatedEnvVar` (no path migration needed when only the env var is deprecated).
- **`.taniwha/kupu/contracts/base-directory-resolution/v1.md`** — the contract for `resolve_base_dir`. The `ResolvedBaseDir` struct, `DeprecationNotice` enum, and the five-branch resolution order are all defined there. This cycle MUST NOT redefine those — they are stable, in main, and consumed verbatim.
- **`.taniwha/kupu/design/v1.md`** — design for the deprecation shim. Section "Caller-site change in the existing configuration-loading module" is directly relevant: the migration prompt logic plugs into the same caller site (`Config::load`) where the warning is currently emitted.
- Decisions worth checking: any decision affecting the path layout (e.g. the scope-amendment decisions from #71's cycle establishing `.taniwha/arai` as the target).

If any of those prior artifacts contradict this brief, **surface a re-raise — do not silently override**. Brief v4 is layered on top of v2, not replacing it.

## Scope (single_module tier — same module as #73)

Modify `src/config.rs` (and possibly `src/main.rs` to thread state into `arai init`) to add a migration-offer flow gated on the existing `DeprecatedDefaultPath` notice.

### Behaviour required

1. **Detection**: re-use the existing `resolve_base_dir` resolution. When the result has `notice = Some(DeprecatedDefaultPath(_))` AND we're running an interactive command (specifically `arai init`, NOT every hook subprocess), the migration flow may run.
2. **Prompt UX**: print a short summary describing what would be moved (source path, destination path, file count, total size). Ask `y/N` (default No — never surprise users).
3. **On accept (y)**: move `~/.arai/` to `~/.taniwha/arai/` atomically (rename, falling back to copy-then-delete if rename fails across filesystems). Print confirmation.
4. **On decline (N or default)**: write a marker file at `~/.arai/.migration_declined` (or equivalent) so the prompt does not re-fire on the next `arai init`. The deprecation warning from the shim continues, but no further offer.
5. **Subsequent prompts**: if the marker file exists, the migration flow short-circuits silently. If the user later removes the marker manually, the prompt fires again.

### Open design questions for the design-doc / contract-derivation phases

- **Trigger location**: `arai init` is the obvious candidate. Should other interactive commands also offer? (The brief's preferred answer: `arai init` only — keeps the surface minimal. Other commands continue to emit the deprecation warning via the shim.)
- **Decline-state location**: marker file at `~/.arai/.migration_declined`? A line in `~/.taniwha/arai/config.toml`? An environment variable `ARAI_MIGRATION_DECLINED=1`? The brief leans toward marker file in the legacy directory because it survives the legacy path's existence and disappears naturally if the user later deletes `~/.arai/` themselves.
- **Atomicity**: `std::fs::rename` for same-filesystem; `fs_extra::dir::move_dir` or copy-then-delete for cross-filesystem. The brief leans toward "try rename first, fall back gracefully" — but the implementor should verify Rust stdlib semantics around cross-filesystem renames.
- **What about repo-local `.arai/`?**: the original brief mentions `<repo>/.arai/` but the resolver only looks at user-global. Brief v4 scopes to user-global only. Repo-local migration, if needed, can be its own future ticket.

These are real design questions — the design-doc phase IS justified for this cycle (unlike #74). Don't surface to the user asking to skip; run the canonical flow.

## In scope

- New `migrate_arai_dir(...)` (or equivalent name) function in `src/config.rs` or a new helper module.
- Plumbing in `src/main.rs::cmd_init` (or wherever `arai init` lives) to call the migration check after `Config::load`.
- Marker-file handling for the decline state.
- Tests covering: prompt fires when `DeprecatedDefaultPath` present + `arai init`; prompt does NOT fire when notice is `DeprecatedEnvVar` or absent; prompt does NOT re-fire when marker file exists; accept-flow moves the directory; decline-flow writes the marker; non-`arai init` commands never prompt.

## Out of scope (do NOT do)

- ANY changes to `resolve_base_dir`, `ResolvedBaseDir`, or `DeprecationNotice` (those are stable contracts from cycle #73).
- ANY change to other commands' behaviour (only `arai init` gets the prompt).
- Repo-local `.arai/` migration (different ticket if needed).
- Removing the deprecation warning itself — the shim's stderr warning continues regardless of migration choice.
- Documentation sweep (separate concern).
- Changes to `~/.taniwha/arai/`'s internal layout.
- A "reset migration decline" command (manually deleting the marker file is sufficient for v1).

## Acceptance criteria

- AC1: On `arai init` with `~/.arai/` present and `~/.taniwha/arai/` absent (i.e., resolver returns `DeprecatedDefaultPath` notice), a migration prompt is printed describing source, destination, file count, total size.
- AC2: Default (empty input or 'N') is decline; only explicit 'y' or 'Y' triggers migration.
- AC3: On accept, `~/.arai/` is moved to `~/.taniwha/arai/` (rename or copy-then-delete) and confirmation is printed.
- AC4: On decline, a marker file is written at `~/.arai/.migration_declined` (or equivalent path documented in tests).
- AC5: When the marker file exists, the prompt does NOT re-fire on subsequent `arai init` invocations (silent short-circuit).
- AC6: On `arai init` with `~/.taniwha/arai/` already present (resolver returns no notice or returns `DeprecatedEnvVar`), no prompt is shown.
- AC7: Non-`arai init` commands (e.g. `arai status`, `arai why`) never invoke the migration flow regardless of notice state.
- AC8: All migration-prompt logic is testable without real filesystem mutation — prompt I/O, file-system operations, and marker-file checks are injectable so tests can drive every branch with closures.
- AC9: `cargo test` passes with all current tests still passing (currently 287) plus new tests covering AC1–AC7.

## Tier rationale

`single_module`: changes are contained to `src/config.rs` and `src/main.rs::cmd_init`. The new `migrate_arai_dir` function depends only on `ResolvedBaseDir` and the standard library. No new shared types cross module boundaries. Per the design-doc skill: "modifications that fit entirely within an existing module's contract" — the existing module is the configuration/init flow.

The full canonical flow (design-doc → contract-derivation → leaf-implementation → verifier) IS justified for this cycle because of the open design questions above. **Do not surface_to_user requesting to skip phases for #72.**

## Related

- Master picture: brief/v1.md
- Prior cycles: brief/v2.md (#73 shim — DEPENDENCY), brief/v3.md (#74 docs sweep)
- Stable contract: contracts/base-directory-resolution/v1.md (DEPENDENCY)
- GitHub epic: #61 (folder restructure)
- Lands on: PR #89 (merged, provides the trigger), PR #87 (merged, established the new path)
- Future: a "force re-prompt" or "reset decline" command if users ask for it; repo-local `.arai/` migration if usage emerges.