# Task — contract derivation for `legacy-path-migration`

You are the contract-derivation subagent. Derive the contract (manifest) for
the single module specified in `design_v2.md`: **`legacy-path-migration`**.

This is a single_module build for GitHub issue #72 (migration prompt for
moving `~/.arai/` to `~/.taniwha/arai/`). Only one new module is being added.

## Inputs (under the same directory as this file)

- `design_v2.md` — the structural design document. Authoritative source of
  module shape, inputs/outputs, side effects, behavioural guarantees,
  error semantics, and acceptance criteria. Derive against this.
- `project_context.yaml` — Rust 2021, cargo, single crate, single_package
  module layout (`src/{module}.rs`), tests alongside source as
  `#[cfg(test)] mod tests`. Convention: fallible library functions return
  `Result<T, String>`.
- `prior_contract_base_directory_resolution_v1.md` — STABLE contract from
  cycle #73. Read-only reference for the `ResolvedBaseDir` and
  `DeprecationNotice` shapes this module consumes. **You MUST NOT amend
  this contract.** Do not re-derive it. Do not propose changes to it.
- `decision_post_migration_behaviour.md` — user decision (2026-05-25):
  after a successful migration, `cmd_init` prints a confirmation hint and
  exits with code 0. It does NOT re-resolve `Config::load`. The
  `MigrationOutcome::prompted-accepted` variant is informational only at
  the entry-point level. Encode this in the caller-site change
  specification within the contract.

## Required outputs

Write the contract to:

  `<handoff>/outputs/contract.md`

If you cannot derive the contract from the inputs (genuine ambiguity, not
under-specification you can fill in with a defensible language-conventional
choice), write a re-raise YAML to:

  `<handoff>/outputs/re_raise.yaml`

per the re-raise protocol.

## Contract requirements (must address)

1. **Eight injected capabilities.** Design doc §Modules/legacy-path-migration/Inputs
   lists eight world-touching capabilities that are injected, not ambient:
   path-existence probe, directory-statistics, directory-move, marker-creation,
   one-line-input read, output-write, interactive-tty probe — plus the
   `ResolvedBaseDir` input value. The contract MUST specify the exact Rust
   types for each capability (function-pointer signatures, closure trait
   bounds, or a grouped struct of dyn-trait callables — your call, justify
   briefly). The contract MUST permit unit tests to substitute these
   capabilities with test doubles. The contract MUST NOT require any
   capability to access the real filesystem, real stdin/stdout, or real tty.

2. **EXDEV detection.** Design doc §Behavioural guarantees notes the move
   capability must attempt single-syscall same-filesystem rename first, and
   on failure-due-to-cross-filesystem-condition (and only that condition)
   fall back to copy-then-delete. The contract MUST name the exact Rust
   stdlib symbol used to detect the cross-device condition. The conventional
   symbol on Unix is `std::io::ErrorKind::CrossesDevices` (stabilised in
   1.85) or, for older toolchains, checking `errno == libc::EXDEV`. Pick
   one and justify briefly. Project context says Rust >= 1.75 edition 2021,
   detected_cargo_version 1.94.1 — so `CrossesDevices` (1.85+) is available.

3. **Caller-site change in `arai init` (`src/main.rs`).** Per decision
   `01KSF5DECISION72POSTMIGR001`: on `MigrationOutcome::prompted-accepted`,
   `cmd_init` prints a short confirmation hint (suggest text like
   `"Migration complete. Run 'arai init' again to finish initialisation."`)
   and exits with code 0. It does NOT call `Config::load` again. Specify
   the exact caller-site control flow per `MigrationOutcome` variant. Also
   specify whether `cmd_init` needs additive plumbing to thread the
   `ResolvedBaseDir` (or its notice) from `Config::load` into the migration
   call — design doc §Caller-site change in arai init names this as a
   contract-derivation concern.

4. **Decline marker path.** AC4: `<legacy_base_dir>/.migration_declined`,
   zero-content. The contract MUST specify this path verbatim (no other
   filename, no other parent directory).

5. **No regression of existing public API.** AC9 requires `cargo test`
   passes and 287+ existing tests preserved. The contract MUST specify
   that no public function in `Config`, `resolve_base_dir`, or any
   other existing module changes signature. The new module is additive
   only.

6. **No dependency on other modules in this cycle.** The module consumes
   `ResolvedBaseDir` as a value input. It does NOT call
   `base-directory-resolution`. State this explicitly in the Dependencies
   section.

7. **MigrationOutcome variants and MigrationSummary shape.** Per the design
   doc §Data shapes. Specify the Rust enum/struct shapes, including which
   variants carry payload and what the payload's Rust types are.

8. **Acceptance criteria.** Translate all of design doc §Test surface (AC1
   through AC9 plus the non-interactive, statistics-failure, determinism,
   and no-ambient-access criteria) into objectively verifiable acceptance
   criteria that a verifier holding only the manifest and project context
   can write tests against.

## Conventions

- Rust 2021 edition, snake_case, `Result<T, String>` for fallible functions
  unless a different shape is strictly necessary.
- Module file at `src/legacy_path_migration.rs` (per project_context
  module_layout `src/{module}.rs`). Tests live in `#[cfg(test)] mod tests`
  at the bottom of that file.
- Do NOT introduce new dependencies (no new crates in Cargo.toml).
- Be explicit about which symbols are `pub` (the module's public surface)
  vs internal.

## Boundaries

- Do NOT amend `base-directory-resolution`.
- Do NOT change any other file's public API.
- Do NOT propose changes to `Config::load`'s warning behaviour from cycle #73.
- Do NOT define behaviour for repo-local `.arai/` — only user-global.
- Do NOT add telemetry, audit-log emission, or analytics for migration events.
- Do NOT define a `arai migrate` subcommand — only the `arai init` caller site.

The brief, design, and the user decision are the only sources of authority.
Anything not derivable from them is either (a) a defensible language-conventional
choice you should make (e.g. exact function-pointer vs. boxed-closure syntax) or
(b) a genuine ambiguity you should surface as a re-raise.
