# Implementation notes — legacy-path-migration

## Language-conventional choices

**`Rc<RefCell<T>>` in tests**: The contract mandates `Box<dyn Fn(...)>` capabilities
without `'static` bound, which means closures can capture short-lived locals. Tests use
`Rc<RefCell<T>>` (not `Arc<Mutex<T>>`) to share mutable counters across move-captured
closures. This is idiomatic Rust for single-threaded test code and avoids unnecessary
synchronisation.

**`#[allow(clippy::type_complexity)]` on `MigrationCapabilities`**: The contract
mandates `Box<dyn Fn(&str) -> Result<MigrationSummaryStats, String>>` verbatim for
`dir_stats` and similarly for `move_dir` and `create_marker`. Clippy flags these as
"very complex types". Since the types are contract-specified and cannot be changed
without violating the manifest, the allow-attribute is the appropriate suppression at
the struct declaration site.

**`copy_dir_recursive` as a private `fn` in `init.rs`**: The contract specifies the
cross-device fallback lives in the live `move_dir` closure constructed at the `cmd_init`
call site. Extracting it as a named private function in `init.rs` (the call-site file)
avoids code duplication between the live closure and readability without moving it to
the module or exposing it publicly.

**`write_output` appends without newline**: The contract says "whether it appends a
trailing newline is the implementor's choice". The live capability uses `print!` (no
trailing newline) because the prompt text already ends with a space after `[y/N] `,
giving the user's cursor a natural landing position on the same line as the question.

## Contract clause resolution

**"All eight capabilities"** vs seven struct fields: The contract's prose header says
"All eight capabilities" but the `MigrationCapabilities` struct definition lists seven
fields. Counting the fields explicitly: `path_exists`, `dir_stats`, `move_dir`,
`create_marker`, `read_line`, `write_output`, `is_interactive` = 7. The task.md also
confirms "All 7 capabilities in MigrationCapabilities are Box<dyn Fn(...)>". Resolved
as a prose typo; implemented with 7 fields exactly matching the struct definition.

**`cmd_init` vs `init::run`**: The contract references `cmd_init` as the entry point.
Task.md explicitly instructs: translate this to `init::run` in `src/init.rs`. Applied
mechanically.

**Migration call placement**: The contract says "AFTER Config::load returns and BEFORE
any subsequent init work". In `init::run`, this is immediately after `Config::load()?`
returns (line 5 of the original function), before the `println!("  Scanning for
instruction files...")` that begins normal init work.

**`PromptedAccepted` early return**: The contract says "exit with code 0; do NOT
continue normal arai init flow". Since `init::run` returns `Result<(), String>` and
`main` exits 0 on `Ok(())`, returning `Ok(())` early achieves both goals. No
`std::process::exit` is needed; the natural function return satisfies the requirement.

**`PromptedAcceptFailed` handling**: The contract says "exit with non-zero code". 
Returning `Err(format!("migration failed: {desc}"))` causes `main` to print to stderr
and exit 1, satisfying the requirement without calling `process::exit` directly.

**`ResolvedBaseDir` reconstruction in `init::run`**: `Config` does not store the
full `ResolvedBaseDir` — only `arai_base_dir` (the path) and the new
`deprecation_notice` field. At the call site in `init::run`, a `ResolvedBaseDir` is
reconstructed from these two fields. This is correct because `arai_base_dir` is
exactly `resolved.path` coerced to `PathBuf` and back to `String`. No information
is lost.

## Tests added beyond the AC list

- `decline_marker_failure_returns_marker_failed`: covers `PromptedDeclineMarkerFailed`
  outcome when `create_marker` returns `Err`.
- `read_line_failure_treated_as_decline`: verifies the AC2 rationale that a failed
  `read_line` is treated as empty input (default-decline), with `create_marker`
  invoked.
- `confirmation_line_contains_run_arai_init_again`: verifies the confirmation phrase
  requirement from the contract ("Run 'arai init' again").
- `move_dir_invoked_with_correct_args`: verifies `move_dir` receives `(resolved.path,
  dest_path)` exactly.

## Warnings and surprises

`cargo build` succeeded with zero warnings. `cargo test` passes all 302 tests (276
unit + 4 hooks_safety + 1 mcp_check_action + 19 parser_coverage + 2
verifier_base_directory_resolution). The 4 pre-existing clippy warnings in
`verifier_base_directory_resolution.rs` and `store.rs` are untouched; no new
clippy warnings were introduced in files I authored or modified.

The only borrow-checker surprise was returning a tuple containing a `borrow()` result
from a helper function: the `Ref<'_, T>` borrow kept the `Rc<RefCell<T>>` borrowed
past the function's drop point. Resolved by materialising the values into owned copies
(`let x = *counter.borrow(); x`) before constructing the tuple return.
