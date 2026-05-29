# Decision: Dispatch resolve-composition after all three leaves verified

**Kind:** composition_repair
**Triggered by:** event 01KSRW2E80MX9KTZXAGDKSVX3Z (tier-provenance subagent_returned)
**Recorded:** 2026-05-29

## Summary

All three leaf modules of the extends-pinning-signing-tiering slice (GitHub issue #29) have
been implemented and gate-passed in the working tree on branch feat/29-extends-policy. The
composition node (resolve-composition) is now unblocked and is being dispatched.

## Leaf gate results

| Leaf | Handoff | fmt | clippy | test count | new tests |
|------|---------|-----|--------|------------|-----------|
| directive-tokenisation | 01KSRRYVFH6CAERYQS5TK5Q1N9 | pass | 0 new | 415 total | 23 |
| fetch-verification | 01KSRSR46N9W8F66SVH651VRWF | pass | 0 new | 379 total | 16 |
| tier-provenance | 01KSRTT28PTQ2T2B3CRQKP3308 | pass | 0 new (baseline 9) | 386 total | 7 |

## Composition deliverables

1. Verify/complete the wiring of directive-tokenisation → fetch-verification → tier-provenance
   through the 4-step pipeline in resolve() in src/extends.rs.
2. Author the 9-scenario cross-module integration test in tests/ (cache-seeded, no real network).
3. Full gate: cargo fmt --all --check + cargo clippy --all-targets + cargo test.

## Constraints

- No commit. No branch switch. Working tree only on feat/29-extends-policy.
- No re-implementation of leaf logic.
- All 9 test scenarios required (per the composition contract's AC list).

## Composition handoff

Handoff ID: 01KSRW3SB6F72BNA6H6YEB1JZ0
Outputs at: .taniwha/kupu/orchestrator/handoff/01KSRW3SB6F72BNA6H6YEB1JZ0/outputs/
