# Task — verify `legacy-path-migration` against contract v1

You are the verifier subagent. Verify that the implementation produced by the
leaf-implementation subagent satisfies every acceptance criterion in the
contract.

You are independent of the implementor — write your OWN tests against the
contract's acceptance criteria (do NOT just observe that the implementor's
tests pass). Then run them and report pass/fail per AC.

## Inputs (under `inputs/`)

- `task.md` — this file.
- `contract.md` — the manifest. THIS is what you verify against. The
  acceptance criteria section is authoritative. Ignore any synthesised
  AC summary you may have seen elsewhere.
- `project_context.yaml` — Rust/cargo conventions.
- `implementation_manifest.yaml` — list of files the implementor wrote.
- `implementor_notes.md` — implementation notes (for context only, not
  authoritative; the contract is authoritative).

## Source under test (repo-root paths)

- `/home/matt/r/arai/src/legacy_path_migration.rs` — the module
- `/home/matt/r/arai/src/config.rs` — additive: `pub deprecation_notice` field
- `/home/matt/r/arai/src/init.rs` — additive: call site
- `/home/matt/r/arai/src/main.rs` — additive: mod declaration

Read each one. Verify against the contract's acceptance criteria, NOT against
a paraphrase or summary.

## How to verify

1. Read the contract's `## Acceptance criteria` section (the section that
   enumerates AC1–AC9 plus the additional non-interactive, statistics-failure,
   determinism, and no-ambient-access criteria).
2. For each AC, decide what test would prove it. Write the test as a Rust
   integration test in `tests/verifier_legacy_path_migration.rs` (new file).
3. Run the tests: `cargo test --test verifier_legacy_path_migration`.
4. Also run the full suite: `cargo test` — confirm ≥ 302 tests pass.
5. Inspect the source code where a test can't fully prove the AC (e.g.
   "no ambient access" — confirm structurally that no `std::env::var`,
   `std::fs`, `println!`, `eprintln!`, `std::io::stdin`, `std::io::stdout`,
   or `IsTerminal` call appears in module's pure function bodies — only in
   the live capability constructors at the `src/init.rs` call site).

## Output

Write to `outputs/`:

- `verifier_report.yaml` — REQUIRED. Shape:

  ```yaml
  schema_version: 1
  overall: pass | fail | partial
  contract_version: 1
  module: legacy-path-migration
  per_ac:
    - id: AC1
      verdict: pass | fail | partial
      evidence: |
        Short citation of test name and/or source-code location demonstrating
        the verdict (e.g. "test name `<x>` passes; src/legacy_path_migration.rs
        line N–M shows the trigger predicate").
    # ...one entry per AC, including the additional criteria...
  cargo_test_summary:
    full_suite_count: <int>
    full_suite_pass: <bool>
    verifier_tests_added: <int>
    verifier_tests_pass: <bool>
  notes: |
    Any observations not captured in per_ac.
  ```

- `tests/verifier_legacy_path_migration.rs` is also expected — written to its
  repo-root path so the verifier's tests are reproducible. If the file already
  exists from a prior verifier run, overwrite it.

- If you find a failing AC, do NOT modify the implementation. Report it in
  the report and let the orchestrator decide remediation.

## Boundaries

- Do NOT modify the module's source (only `tests/verifier_legacy_path_migration.rs`).
- Do NOT amend the contract.
- Do NOT skip ACs you find inconvenient to test — if an AC is hard to test
  structurally, write a structural-check test (e.g. text-search the source
  for forbidden symbols) and mark verdict appropriately.
