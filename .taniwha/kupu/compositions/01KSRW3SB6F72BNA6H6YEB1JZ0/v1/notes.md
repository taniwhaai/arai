# Resolve-Composition Notes

## Summary

The resolve-composition node wires three completed child modules (directive-tokenisation, fetch-verification, tier-provenance) through the `resolve()` function in `src/extends.rs`. The wiring is **already complete and correct** — all three child modules were properly integrated during their respective leaf-implementation phases. This composition node's role is to verify the wiring is faithful to the composition contract and to author the cross-module integration test.

## Wiring Overview

The composition implements the exact five-step sequence specified in the composition contract:

### Step 1: Tokenise
`extract_directives()` calls `classify_directive()` on each `arai:extends` directive line in the instruction file. Each directive line is parsed into either a `ParsedDirective` (success) or `MalformedDirective` (failure). Malformed directives are logged to stderr and skipped; parsing continues with the next directive.

**Source:** `src/extends.rs` lines 941-952, 1045

### Step 2: Resolve Trust Entry & Obtain Content
`fetch()` checks that the URL is trusted via `is_trusted()`, then looks up the corresponding `TrustEntry` via `find_trust_entry()` before attempting to fetch the content. The content is fetched from the network (or cache fallback) using the existing fetch/cache path.

**Source:** `src/extends.rs` lines 725-733

### Step 3: Verify
`fetch()` calls `verify_content()` on both the fresh-remote path (line 758) and the stale-cache path (line 742). The verification checks:
- **Pin comparison:** If the directive carries a `@<pin>` token, the sha256 hash of the obtained content must match the pin (case-insensitive). Mismatch → reject + warn.
- **Signature verification:** If the TrustEntry carries a `pubkey`, the sidecar (`<url>.sig`) is fetched and the signature is verified using ed25519. Missing or invalid signature → reject + warn.
- **Combined checks:** Both must pass when both are configured. Neither → admit (no new checks configured for this URL).

The backward-compatibility invariant is maintained: when both pin and pubkey are absent, `verify_content` returns `Ok(true)` immediately (line 492), producing byte-identical behavior to the pre-slice code.

**Source:** `src/extends.rs` lines 479-530, 742, 758

### Step 4: Tag Provenance & Inline
`resolve()` emits a tier-annotated block-start comment before inlining the admitted upstream content:

```html
<!-- arai:extends-block url="<url>" tier="<tier>" -->
```

The tier value comes from `ParsedDirective.tier`. When tier is absent (bare directive), it defaults to "peer" (lines 1057-1062). No new code branch is entered when tier is absent, satisfying AC1 (backward-compat).

**Source:** `src/extends.rs` lines 1044-1079

### Step 5: Downstream Pipeline
The inlined + provenance-tagged content passes to the existing parser/store/guardrails pipeline. The parser's `extract_rules_from_resolved()` function (parser.rs, lines 182-245) reads the block-start markers and applies per-block provenance (tier + source_label) to each extracted rule triple.

**Source:** `src/parser.rs` lines 182-245

## Error Semantics

All error paths follow the composition contract:

- **MalformedDirective from tokenisation:** `resolve()` logs a stderr warning (line 1072-1074) and skips the directive. Processing continues with the next directive. No error is propagated to the caller.

- **Reject from fetch-verification:** `fetch()` logs a stderr warning (line 744, 761) and returns an Err. `resolve()` treats this as a failed fetch (line 1072-1074), skips the directive, and continues. No error is propagated to the caller.

- **Trust-file read error:** The `read_trust()` function (line 333-338) silently defaults to an empty trust file if the file cannot be read or deserialized. This is the existing behavior and is unchanged. The absence of a trust file is not an error condition.

Per-directive failures are isolated: a directive that fails to verify does not affect subsequent directives. If the directive file contains ten directives and three fail verification, the output contains seven admitted upstream blocks plus the local content.

## Backward-Compatibility Invariants (Hard)

The composition contract requires two mechanisms guarantee byte-identical behavior for a bare `arai:extends <url>` directive with a legacy list-of-strings `trusted_extends.toml`:

### Tokeniser Half (AC1)
A directive with no trailing tokens produces a `ParsedDirective` with `pin=None` and `tier=None`. The code never enters the verify-content branch when both are absent. The signature check and pin comparison are guarded by the presence of the pin/pubkey values.

**Test:** `test_ac1_bare_directive_legacy_trust_file` verifies this end-to-end.

**Source:** `src/extends.rs` lines 184-236

### Trust-File Half (AC7-AC8)
The `TrustFile` dual-form deserialiser maps legacy list-of-strings entries to `TrustEntry` with `pubkey=None`. When pubkey is absent, `verify_content` skips the signature check entirely (line 500). When pin is also absent, `verify_content` returns `Ok(true)` immediately without entering any new branch (line 492).

**Test:** `test_ac1_bare_directive_legacy_trust_file` verifies this end-to-end.

**Source:** `src/extends.rs` lines 284-319 (dual-form deserialiser)

## Behavioral Guarantees

### Ordering
Step 1 (tokenise) always precedes Step 3 (verify). Step 3 always precedes Step 4 (provenance/inline). The resolve() loop calls these in strict sequence per directive.

**Tests:** `test_wiring_order_tokenise_before_verify`, `test_wiring_order_verify_before_provenance`

### Atomicity per Directive
Each directive is either fully admitted (all steps 1-4 succeed) or fully skipped (any failure causes skip). No partial inline.

**Tests:** `test_per_directive_failure_isolation`, `test_ac3_pin_mismatch_rejects_content`

### Idempotency
Given the same instruction file and trust file, `resolve()` produces the same output. No state accumulates across calls.

**Verification:** The function is pure at the call level (no shared mutable state) and deterministic (tokenization is order-independent per AC12f).

### Concurrency
`resolve()` is called during discovery/scan. The verification path holds no shared mutable state and is safe under concurrent invocation. Store writes (from the downstream parser/store layer) follow the existing store concurrency model.

**Verification:** No shared mutable state in extends.rs; existing store model is unchanged.

## Integration Test Scenarios

The new cross-module integration test `tests/extends_integration.rs` covers:

1. **AC1 — Bare directive backward-compat:** Legacy trust file + no trailing tokens → byte-identical output.
2. **AC2 — Pin matching:** Pin present and matching → content admitted and inlined.
3. **AC3 — Pin mismatch:** Pin present and mismatching → content rejected, local content preserved.
4. **AC6 — Missing sidecar:** Pubkey configured but sidecar missing → content rejected.
5. **AC9 — Strict tier:** Upstream rule with strict tier not shadowed by local rule with same subject.
6. **AC10 — Advisory tier:** Upstream rule with advisory tier present but deprioritised.
7. **AC11a — Override with match:** Upstream SPO triple matches local SPO → upstream dropped.
8. **AC11b — Override without match:** Upstream SPO triple does not match any local SPO → upstream retained.
9. **Per-directive isolation:** Two directives where first fails, second succeeds → output includes second upstream, not first.
10. **Wiring order 1:** Malformed directive (tokenisation fails) does not trigger a fetch.
11. **Wiring order 2:** Pin mismatch (verification fails) does not emit provenance marker.

All tests seed the on-disk cache directly, avoiding real network calls.

**Test file:** `tests/extends_integration.rs` (11 tests, all pass)

## Architecture Notes

### New File: src/lib.rs
A minimal library entrypoint was created to expose the `extends` module publicly for integration testing. This allows the test suite to call `resolve()`, `classify_directive()`, `fetch()`, `trust_add()`, etc. directly without invoking the binary.

The lib.rs is test-only and not published or part of the production interface. The main binary remains unchanged.

### New Dependency: tempfile = "3"
Added to `[dev-dependencies]` in Cargo.toml to support temporary directory management in the integration test. Used by `TempDir::new()` for test isolation.

### Doctest Fix: src/extends.rs resolve()
The doctest example in the `resolve()` documentation contained HTML comments (`<!-- ... -->`) which were being interpreted as Rust code. Fixed by marking the example with `ignore` directive so `cargo test --doc` skips it.

## Gate Results

All three gates pass:

- **cargo fmt --all --check:** No formatting issues.
- **cargo clippy --all-targets:** 9 pre-existing warnings (unchanged). No new warnings introduced.
- **cargo test:** All 516 tests pass (386 unit + library + 130 integration, including 11 new extends_integration tests).

## Verification Checklist

✓ Wiring is deterministic from the contracts  
✓ No new side effects introduced  
✓ No hidden adapters (faithful contract-to-contract routing)  
✓ Every parent guarantee has a derivation  
✓ Every child error reaches a destination  
✓ Composition is mechanical, not creative  
✓ All contract acceptance criteria satisfied  
✓ Backward-compatibility invariants verified end-to-end  
✓ Cross-module integration test complete  
✓ Full gate passes (fmt/clippy/test)  

## Known Limitations / Out of Scope

The integration tests use public APIs and cache-seeding, not cryptographic key generation. Signature verification with actual keys is tested separately in the fetch-verification unit tests (ac5, ac6a, ac6b). The integration test focuses on the composition wiring, not on cryptographic correctness (which is owned by the ed25519-dalek crate and the fetch-verification leaf).

## References

- Composition contract: `.taniwha/kupu/orchestrator/handoff/01KSRW3SB6F72BNA6H6YEB1JZ0/inputs/composition_contract.md`
- Child contracts:
  - `inputs/contract_directive_tokenisation.md`
  - `inputs/contract_fetch_verification.md`
  - `inputs/contract_tier_provenance.md`
- Shared vocabulary: `inputs/vocabulary.md`
- Project context: `inputs/project_context.yaml`
- Integration test: `tests/extends_integration.rs`
