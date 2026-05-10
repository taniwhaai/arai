```yaml
verifier_report:
  contract:
    module: base-directory-resolution
    version: 1

  implementation:
    version: 1
    source_paths:
      - src/config.rs

  # ---------------------------------------------------------------------------
  # Structural constraint note
  # ---------------------------------------------------------------------------
  # arai is a binary-only crate (src/main.rs as root, no [lib] section in
  # Cargo.toml). External integration tests in tests/ cannot import internal
  # modules. The contract mandates tests using injected callables, which
  # requires direct access to resolve_base_dir. The only compilation-viable
  # location is inside src/config.rs as #[cfg(test)] mod tests.
  #
  # The implementor correctly placed tests there. The verifier wrote a
  # process-level test file at tests/verifier_base_directory_resolution.rs
  # (two smoke tests) and performed independent structural source inspection
  # for per-AC verification. The implementor's tests were reviewed
  # independently against the contract; findings are noted per-AC below.
  # ---------------------------------------------------------------------------

  acceptance_criteria:

    - id: AC1
      criterion: >
        When ARAI_BASE_DIR is set, the resolver returns that value as path with
        no notice, and does not call env-lookup(ARAI_DB_DIR) or path-exists at all.
      verifier_test: >
        Independent source inspection of resolve_base_dir (lines 93-98 of src/config.rs).
        Implementor test ac1_current_env_var_wins_unconditionally reviewed against contract.
      result: pass
      notes: |
        Branch 1 in the resolver calls env_lookup("ARAI_BASE_DIR") and returns
        immediately with ResolvedBaseDir { path: value, notice: None } when Some
        is returned. The ARAI_DB_DIR lookup and both path_exists calls are
        unreachable. The implementor's test iterates the full 8-combo grid
        (ARAI_DB_DIR x new_exists x old_exists), asserts path = "/explicit/path"
        and notice.is_none(), and confirms env_calls.len() == 1 (only ARAI_BASE_DIR)
        and path_calls.is_empty() for every combo. This matches the contract exactly.

    - id: AC2
      criterion: >
        When ARAI_BASE_DIR is absent and ARAI_DB_DIR is set, the resolver returns
        that value as path, notice = deprecated-env-var (non-empty message),
        and does not call path-exists at all.
      verifier_test: >
        Independent source inspection (lines 101-109). Implementor test
        ac2_deprecated_env_var_used_when_current_absent reviewed against contract.
      result: pass
      notes: |
        Branch 2 calls env_lookup("ARAI_DB_DIR"), returns ResolvedBaseDir with
        path = value and notice = Some(DeprecationNotice::DeprecatedEnvVar(msg))
        where msg is a non-empty string. path_exists is not called. The implementor's
        test iterates the 4-combo path_exists grid, confirms path = "/legacy/db",
        checks the notice is DeprecatedEnvVar with non-empty message, and asserts
        path_calls.is_empty(). Matches contract. Short-circuit verified.

    - id: AC3
      criterion: >
        When both env vars are absent and new-default exists (old-default irrelevant),
        the resolver returns new-default as path with no notice. Old-default must
        not be probed.
      verifier_test: >
        Independent source inspection (lines 119-125). Implementor test
        ac3_new_default_used_silently_when_it_exists reviewed.
      result: pass
      notes: |
        Branch 3: path_exists(&new_default) is true -> returns new_default, no notice.
        The old_default path is never queried. The implementor's test confirms
        result.path == NEW_DEFAULT, result.notice.is_none(), and that OLD_DEFAULT
        does not appear in path_calls. Matches contract.

    - id: AC4
      criterion: >
        When both env vars are absent, new-default does not exist, and old-default
        exists, the resolver returns old-default as path with notice = deprecated-default-path
        (non-empty message that contains "arai migrate").
      verifier_test: >
        Independent source inspection (lines 129-139). Implementor test
        ac4_old_default_used_with_notice_when_only_it_exists reviewed.
        AC4 mandatory wording ("arai migrate") confirmed by source inspection.
      result: pass
      notes: |
        Branch 4 message (line 130-133 of src/config.rs):
          "warning: ~/.arai is deprecated; the new default is ~/.taniwha/arai.
           A forthcoming `arai migrate` command will help move your data to
           the new location. (current path: {old_default})"
        This contains "arai migrate". The implementor's test asserts the
        DeprecatedDefaultPath variant, non-empty message, and msg.contains("arai migrate").
        Mandatory wording requirement satisfied.

    - id: AC5
      criterion: >
        When both env vars are absent and neither default exists, the resolver
        returns new-default as path with no notice.
      verifier_test: >
        Independent source inspection (lines 141-145 — branch 5 fallthrough).
        Implementor test ac5_fresh_install_fallback_to_new_default_with_no_notice reviewed.
      result: pass
      notes: |
        Branch 5 is the fallthrough after all four conditional checks fail. Returns
        ResolvedBaseDir { path: new_default, notice: None }. The implementor's
        test confirms path == NEW_DEFAULT and notice.is_none(). Matches contract.

    - id: AC6
      criterion: >
        When both env vars are absent and both defaults exist, the resolver returns
        new-default (not old-default) with no notice. Result must be identical to AC3.
      verifier_test: >
        Independent source inspection (lines 119-125 — branch 3 fires, branch 4 unreachable).
        Implementor test ac6_new_default_takes_precedence_over_old_when_both_exist reviewed.
      result: pass
      notes: |
        When path_exists(&new_default) is true, branch 3 returns immediately with
        new_default and no notice regardless of path_exists(&old_default). Branch 4
        (which would return old_default with notice) is unreachable. The implementor's
        test asserts result.path == NEW_DEFAULT, result.notice.is_none(), with an
        explicit comment "NOT AC4". Matches contract.

    - id: AC7
      criterion: >
        The resolver's implementation accesses no real process environment or filesystem
        directly. All world-touching capability is injected by the caller. Test bodies
        themselves must set no env vars and create no real directories.
      verifier_test: >
        Structural source inspection of resolve_base_dir (lines 83-146 of src/config.rs).
        Review of all implementor test bodies in config::tests (lines 315-635).
      result: pass
      notes: |
        The resolve_base_dir function body contains no std::env:: calls, no
        std::fs:: calls, no std::path::Path::new(...).exists(), no dirs:: calls.
        All env and filesystem access is performed through the env_lookup and
        path_exists closures supplied by the caller. The ambient-access calls
        (std::env::var, std::path::Path::new(p).exists()) appear only in
        Config::load() at lines 156-160, where they are passed as closures to
        resolve_base_dir — correctly separated from the resolver itself.
        The implementor's test bodies use only RefCell-recording closures;
        none set environment variables or create directories.

    - id: AC8
      criterion: >
        After introducing the resolver, cargo test passes with all previously
        passing tests (>=277) still passing, plus at least 6 new tests covering AC1-AC6.
      verifier_test: >
        cargo test run observed: 287 total tests, 0 failed.
        Breakdown: 261 unit (main), 4 hooks_safety, 1 mcp_check_action,
        19 parser_coverage, 2 verifier_base_directory_resolution.
        Without verifier tests: 285. Implementor added 8 new resolver tests
        (ac1 through ac6 = 6, additional_determinism, additional_notice_mutual_exclusivity).
      result: pass
      notes: |
        Total 285 tests without the verifier's own 2 tests. That is 285 >= 283
        (277 + 6 minimum). The 6 AC1-AC6 tests are all present and all pass.
        The 8 additional resolver tests (relative to the 277 prior baseline) exceed
        the 6-test minimum. All 277 previously passing tests continue to pass.

  overall: pass

  findings:
    - kind: other
      summary: Binary-only crate prevents closure-injection tests in tests/ directory
      details: |
        The contract mandates that all resolver tests use injected callables and
        prohibits tests from setting process environment variables or creating
        directories. This pattern requires direct function access, which is only
        possible within the crate (src/config.rs #[cfg(test)] mod tests) because
        arai has no [lib] target and no lib.rs. External integration tests in
        tests/ cannot import resolve_base_dir.

        The implementor correctly placed tests inside src/config.rs. The verifier
        wrote a process-level smoke test file (tests/verifier_base_directory_resolution.rs)
        which compiles and passes but cannot independently verify the resolver's
        pure-function behaviour using injected callables.

        This is not an implementation bug; it is a structural property of the
        codebase that the contract does not account for. For future contracts on
        this codebase, the "tests must use injected callables" requirement implies
        unit tests colocated with the source, not external integration tests.

        Recommendation for contract-derivation: note that arai is binary-only
        and that acceptance criteria requiring injected-callable tests must
        specify in-module unit tests, not tests/ integration tests.

    - kind: other
      summary: >
        Verifier's per-AC independent tests are source-inspection based, not
        executable injection-based — see structural constraint above
      details: |
        Because the verifier cannot compile closure-injection tests as external
        integration tests, per-AC pass/fail determinations are based on:
        (a) independent line-by-line reading of the resolver source,
        (b) independent reading of the implementor's tests against the contract,
        (c) full cargo test run confirming 0 failures.
        This is weaker than the verifier skill's ideal (independently compiled
        verifier tests), but it is the best achievable given the crate architecture.

  additional_findings:
    - id: notice-mutual-exclusivity
      result: pass
      notes: |
        The Rust type system enforces mutual exclusivity structurally: notice is
        Option<DeprecationNotice> where DeprecationNotice is an enum with exactly
        two variants. A value cannot be both simultaneously. Branches 1, 3, 5
        return notice: None. Branch 2 returns Some(DeprecatedEnvVar(_)). Branch 4
        returns Some(DeprecatedDefaultPath(_)). These branches are mutually
        exclusive by the short-circuit logic. The implementor's
        additional_notice_mutual_exclusivity test confirms both variants directly.

    - id: path-always-present-and-non-empty
      result: pass
      notes: |
        Every branch of resolve_base_dir returns a ResolvedBaseDir with a non-empty
        path field. Branch 1: env_lookup returns Some(value); the contract requires
        callers to return only non-empty strings as Some. Branch 2: same. Branches
        3/4: paths are constructed as format!("{home_trimmed}/.taniwha/arai") and
        format!("{home_trimmed}/.arai") — non-empty as long as home_trimmed is
        non-empty (the contract forbids invoking the resolver with an empty
        home-directory). Branch 5: same as branch 3 construction. No branch can
        produce an empty path field given the contract's input constraints.

    - id: determinism
      result: pass
      notes: |
        The resolver holds no shared mutable state. It is a pure function over its
        inputs. Given the same closures returning the same values, the same
        ResolvedBaseDir is produced. The implementor's additional_determinism test
        confirms two successive calls with identical inputs return == equal results.
        ResolvedBaseDir derives PartialEq, Eq, so structural equality is well-defined.

  structural_notes:
    ac7_inspection: |
      Confirmed: resolve_base_dir (lines 83-146, src/config.rs) contains no
      direct calls to std::env, std::fs, std::path::Path, or dirs. All
      environment and filesystem access is through the injected closures.
      The Config::load() caller site (lines 156-160) correctly supplies real
      std::env::var and std::path::Path::new(p).exists() as closures, keeping
      the resolver itself pure.

    ac4_wording_check: |
      Message text at src/config.rs lines 130-133:
        "warning: ~/.arai is deprecated; the new default is ~/.taniwha/arai.
         A forthcoming `arai migrate` command will help move your data to
         the new location. (current path: {old_default})"
      Contains "arai migrate". Mandatory wording requirement satisfied.

    tty_gate_inspection: |
      Config::load() lines 165-169:
        if let Some(notice) = &resolved.notice {
            if std::io::stderr().is_terminal() {
                eprintln!("{}", notice.message());
            }
        }
      The eprintln! is guarded by stderr().is_terminal(). std::io::IsTerminal
      is imported at line 2. TTY-gate is correctly at the caller site; the
      resolver does not inspect TTY state. Matches contract requirement.

  test_count_before: 277
  test_count_after: 287
  test_count_delta: +10
  test_count_note: |
    285 after implementation (8 new resolver tests + 2 pre-existing config tests
    that were already there), minus 277 prior = 8 new tests. Plus 2 from the
    verifier's own smoke test file = 287 total. All pass.

  notes_for_orchestrator: ~
```
