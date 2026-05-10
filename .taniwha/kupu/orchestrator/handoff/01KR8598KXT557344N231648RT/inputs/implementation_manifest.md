# Implementation notes — base-directory-resolution

## Files changed

- `src/config.rs` — added pure resolver `resolve_base_dir`, the public
  data shapes `ResolvedBaseDir` and `DeprecationNotice` (with
  `DeprecatedEnvVar(String)` and `DeprecatedDefaultPath(String)`
  variants), and rewired `Config::load()` to call the resolver with
  injected dependencies and emit any notice to stderr behind an
  `IsTerminal` gate.

No other files were touched. The resolver was kept inside `config.rs`
rather than split into a new `src/base_dir.rs` because:

- `src/main.rs` is the only crate-root file (no `lib.rs`), and the
  authorization rules forbid modifying `main.rs`. Splitting the
  resolver out would require adding `mod base_dir;` to `main.rs`.
- Co-locating the resolver with its sole caller (`Config::load`) keeps
  the caller-site change reviewable in one diff.

## Per-AC test coverage

Tests live in `src/config.rs` `#[cfg(test)] mod tests`, immediately
after the two pre-existing tests. All eight new tests use injected
closures wrapped in `RefCell` so they can also assert short-circuit
behaviour by inspecting which deps were called.

| AC | Test fn | What it asserts |
|----|---------|-----------------|
| AC1 | `ac1_current_env_var_wins_unconditionally` | Iterates the full grid of `ARAI_DB_DIR` × `path_exists(new)` × `path_exists(old)` (8 combos). For every combo: `path == "/explicit/path"`, `notice.is_none()`, env-lookup called exactly once with `"ARAI_BASE_DIR"`, path-exists never called. Verifies the short-circuit guarantee in addition to the value. |
| AC2 | `ac2_deprecated_env_var_used_when_current_absent` | Iterates the 4-combo `path_exists` grid. For every combo: `path == "/legacy/db"`, notice variant is `DeprecatedEnvVar` with non-empty message, path-exists never called (branch 2 short-circuits 3-5). |
| AC3 | `ac3_new_default_used_silently_when_it_exists` | `new_default` returned, no notice; old-default not probed (branch 3 short-circuits branch 4). |
| AC4 | `ac4_old_default_used_with_notice_when_only_it_exists` | `old_default` returned, notice variant is `DeprecatedDefaultPath`, message is non-empty AND `msg.contains("arai migrate")` (per AC4's mandatory wording requirement). |
| AC5 | `ac5_fresh_install_fallback_to_new_default_with_no_notice` | Both env vars unset, both defaults absent → `new_default` returned, no notice. |
| AC6 | `ac6_new_default_takes_precedence_over_old_when_both_exist` | Disambiguator vs. AC4. Both defaults exist → result identical to AC3 (`new_default`, no notice), explicitly NOT AC4. |
| AC7 | (structural — verified by test bodies) | Every test constructs its own closures. Test bodies set no env vars, create no directories, do not touch `$HOME`. The `RefCell` call-recording also proves the resolver does not bypass injected deps (calls show up in the recorder). |
| AC8 | (whole-suite) | `cargo test` shows 261 lib tests + 4 + 1 + 19 = 285 tests passing, ≥ the 277-test minimum stated in the AC. |
| Notice mutual exclusivity | `additional_notice_mutual_exclusivity` | Confirms branch 2 yields only `DeprecatedEnvVar`, branch 4 yields only `DeprecatedDefaultPath`; branches 1/3/5 yield `None` (covered transitively by AC1/AC3/AC5/AC6). |
| Determinism | `additional_determinism` | Two successive calls with the same closures return `==`-equal `ResolvedBaseDir`s. |

The "path always non-empty" additional clause is covered transitively
by the AC1-AC6 tests, each of which asserts a concrete non-empty path
value for its branch.

## Verification command

```bash
cargo test
```

(No features needed; the resolver lives in the default build. The
project's default cargo features include `code-graph`, but the
resolver does not depend on any feature flags.)

To run only the new resolver tests:

```bash
cargo test --bin arai config::tests::ac
```

To run the whole config module (includes the two pre-existing tests):

```bash
cargo test --bin arai config::
```

## Constraints the verifier should know

- **`std::io::IsTerminal`** is brought into scope at the top of
  `config.rs` (`use std::io::IsTerminal;`) and used at the caller-site
  in `Config::load()` to gate the `eprintln!`. Stable since Rust 1.70;
  fine on the project's Rust 1.94+.
- **The resolver is generic over `Fn` closures**, not `dyn Fn` trait
  objects. This keeps it allocation-free and lets callers pass
  capturing closures without boxing. Test helpers `env_with` /
  `exists_with` use lifetime-annotated `impl Fn(...) + 'a` returns to
  borrow the call-recorder `RefCell` cleanly.
- **Empty env-var values are treated as unset.** The contract says
  env-lookup returns either "non-empty text string" or
  "unset/empty"; `Config::load()` enforces this with
  `.ok().filter(|v| !v.is_empty())`. The resolver itself does not
  re-validate — it trusts the closure's `Some` to mean non-empty per
  contract.
- **Path strings are constructed with `format!`**, not `PathBuf`, so
  the resolver never touches `std::path` internals or the filesystem.
  A trailing slash on the home-directory string is tolerated via
  `trim_end_matches('/')`.
- **No new files** were created. No files outside `src/config.rs`
  were modified.
- The full `cargo test` run (285 tests) passes locally on this branch.
