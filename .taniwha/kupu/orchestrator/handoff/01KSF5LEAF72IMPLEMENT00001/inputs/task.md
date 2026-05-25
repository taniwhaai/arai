# Task — implement `legacy-path-migration` against contract v1

You are the leaf-implementation subagent for the Arai project (Rust CLI at
`/home/matt/r/arai`).

## Inputs (under `inputs/`)

- `contract.md` — the manifest for module `legacy-path-migration`. THIS is the
  source of truth for module shape, types, behaviour, and acceptance criteria.
  Implement faithfully against it.
- `prior_contract_base_directory_resolution.md` — STABLE contract from cycle
  #73. Read-only reference for the `ResolvedBaseDir` and `DeprecationNotice`
  shapes you will consume. MUST NOT amend or change behaviour of this module.
- `project_context.yaml` — Rust 2021, cargo, single crate, single_package
  module layout (`src/{module}.rs`), tests live in `#[cfg(test)] mod tests`
  at the bottom of each source file. Convention: `Result<T, String>` for
  fallible functions. No new crate deps.

## Where to write code (repo-root paths)

- **NEW FILE:** `src/legacy_path_migration.rs` — the module implementation
  plus its tests. This is the primary deliverable.
- **ADDITIVE EDIT:** `src/config.rs` — expose the `DeprecationNotice` produced
  by `Config::load`. Currently the notice is consumed locally for the stderr
  warning and dropped; it must be persisted into the `Config` struct so the
  init entry point can read it. **Add** a `pub deprecation_notice:
  Option<DeprecationNotice>` field to `Config`. Populate it from the
  `resolved.notice` Option that `Config::load` already computes. Do NOT
  rename, move, or change any existing field; do NOT change the behaviour of
  the stderr warning emission (it still fires once on every `Config::load`).
- **ADDITIVE EDIT:** `src/lib.rs` (or `src/main.rs` — check which declares the
  module list) — declare `pub mod legacy_path_migration;` next to the other
  `pub mod` declarations.
- **ADDITIVE EDIT:** `src/init.rs` `pub fn run()` — THIS is the init entry
  point, not `cmd_init`. (The contract calls it `cmd_init` because that is
  the canonical name in the brief and design; in this repo the actual function
  is `init::run` in `src/init.rs`, dispatched from `Commands::Init => init::run()`
  in `src/main.rs`.) The migration call must be made AFTER `Config::load`
  returns and BEFORE any subsequent init work (discovery, parsing, etc.).
  The control-flow table from the contract applies as-written, just at this
  call site instead of in `cmd_init`.

## Boundaries

- Do NOT touch any other source file.
- Do NOT add new crate dependencies (no Cargo.toml changes). Use only stdlib.
  For TTY detection: `std::io::IsTerminal` is stable since Rust 1.70 and is
  already used in `src/config.rs` — use the same approach.
- Do NOT change any existing public function signature.
- Do NOT remove or skip any existing test.
- Do NOT amend `base-directory-resolution` or `resolve_base_dir`.

## Acceptance — verify before reporting done

1. `cargo build` succeeds with no warnings introduced.
2. `cargo test` exits 0. The new tests pass. **Existing tests must still pass
   unchanged.** The project had 287+ tests before; the new total must be
   ≥ 287 + new_test_count. Report the actual count.
3. `cargo clippy --all-targets` produces no new warnings (existing warnings
   are out-of-scope, but you must not add new ones).
4. All contract acceptance criteria (AC1–AC9 plus the additional non-interactive,
   statistics-failure, determinism, and no-ambient-access criteria) are
   covered by unit tests in `src/legacy_path_migration.rs`.
5. All capability invocations in tests use closures over captured state. No
   test body may touch the real filesystem, real stdin, real stdout, or real
   tty.

## Required output

Write your output to `outputs/`:

- `manifest.yaml` — list of all new/changed files with their repo-relative
  paths, a one-sentence description per file, and the `cargo test` count
  observed.
- `notes.md` — implementation notes covering: any defensible language-
  conventional choices you made, any contract clauses you found ambiguous
  (and how you resolved them), any tests added beyond the AC list, and any
  warnings or surprises during cargo test.
- If you genuinely cannot implement the contract as specified, write
  `re_raise.yaml` per the re-raise protocol and produce no source changes.

The source files themselves go to their repo-root paths (NOT into `outputs/`).
The orchestrator/dispatcher will treat those as the production deliverables.
