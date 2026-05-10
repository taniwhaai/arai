---
version: 2
captured_at: 2026-05-10T04:50:00Z
source: user_amendment
parent_version: 1
phase: phase-2-issue-73-deprecation-shim
authoritative_spec: github.com/taniwhaai/arai/issues/73
---

# Brief v2 — Issue #73 deprecation shim

## Additive scope

Brief v1 covers the full 26-issue arai roadmap. This v2 amendment scopes the **next build cycle** to a single issue: GitHub #73, the deprecation shim that keeps existing arai installs working after the v0.2.15 path/env-var rename landed in PR #87.

v1 remains the master picture. v2 narrows the work in flight.

## Why now

PR #87 (merged) changed defaults: env var `ARAI_DB_DIR` → `ARAI_BASE_DIR`, default state path `~/.arai/` → `~/.taniwha/arai/`. Without a deprecation shim, anyone upgrading to v0.2.15 with `ARAI_DB_DIR` set or with their data in `~/.arai/` will silently lose access to their rules, audit log, and project DB. The migration command (#72) is a separate workstream and not yet built. The shim is the safety net buying time for #72 to land.

## Scope (single_module tier)

Modify `src/config.rs::Config::load()` to resolve the Arai base directory using the following order:

1. `ARAI_BASE_DIR` env var if set → use it (no warning).
2. Else `ARAI_DB_DIR` env var if set → use it + emit a deprecation warning instructing rename.
3. Else `~/.taniwha/arai/` exists → use it (no warning).
4. Else only `~/.arai/` exists → use it + emit a deprecation warning noting `arai migrate` is coming.
5. Else (neither exists) → use `~/.taniwha/arai/` (fresh-install default, no warning).

Warnings emit to stderr **only when stderr is a TTY** — arai is invoked as a Claude Code hook subprocess on every tool call; warnings on every hook invocation would be unacceptable noise.

## In scope

- New pure resolution function (testable without env-var or filesystem mutation, via injected closures).
- Tests covering all five branches plus the "both new and old default exist" precedence case.
- Caller-site change in `Config::load()` to use the new resolver and emit the optional warning.

## Out of scope (do NOT do)

- Migration logic (`arai migrate` command) that physically moves files — that's GitHub #72.
- Documentation/README sweep — that's GitHub #74.
- Any change outside `src/config.rs`.
- Changes to public API surface (struct fields, function signatures, env var names beyond reading the deprecated alias).
- Refactor or cleanup beyond what the shim itself requires.

## Acceptance criteria

- AC1: `ARAI_BASE_DIR` set → wins regardless of other state. No warning.
- AC2: `ARAI_BASE_DIR` unset, `ARAI_DB_DIR` set → uses ARAI_DB_DIR, single TTY-gated warning instructing rename.
- AC3: Both env vars unset, `~/.taniwha/arai/` exists → uses it, no warning.
- AC4: Both env vars unset, only `~/.arai/` exists → uses it, single TTY-gated warning noting deprecation.
- AC5: Both env vars unset, neither path exists → returns `~/.taniwha/arai/` (fresh-install default).
- AC6: Both `~/.taniwha/arai/` and `~/.arai/` exist → `~/.taniwha/arai/` wins.
- AC7: Resolution logic is testable without setting global env vars or creating real directories.
- AC8: `cargo test` passes (current main: 277 tests; expect 283+ after new tests).

## Tier rationale

`single_module`: change is contained inside one existing module (`src/config.rs`), no cross-module contracts change, no shared types are added, no composition phase needed. Per design-doc skill: "Skip [a multi-module design] for ... modifications that fit entirely within an existing module's contract." This work fits.

## Related

- Master picture: brief/v1.md
- GitHub epic: #61 (folder restructure)
- Sibling open issues: #72 (migration prompt), #74 (docs sweep)
- Lands on top of: PR #87 (merged) — moved the defaults
- Reverses risk introduced by PR #87 silent-break-on-upgrade