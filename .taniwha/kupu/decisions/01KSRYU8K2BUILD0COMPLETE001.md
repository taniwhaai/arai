# Decision 01KSRYU8K2BUILD0COMPLETE001

kind: scope_change
triggered_by: 01KSRYTX4V76JRET2EBKQVA2JB
recorded_at: 2026-05-29T04:11:36.000Z
orchestrator_invocation: 26

## Summary

The independent verifier (handoff 01KSRY2BWJB0MDH0FJ5WDTCP2S) returned
overall: PASS for the full issue-#29 slice. All 14 acceptance criteria pass.
The build is complete. The working tree on branch feat/29-extends-policy is
ready to commit and merge.

## Verification gate results

- cargo fmt --all -- --check: PASS (no diff)
- cargo clippy --all-targets: PASS (5 pre-existing warnings, 0 new)
- cargo test: 486 passed / 0 failed (7 suites including 23 new verifier tests)
- src/lib.rs: does NOT exist (single-binary crate correctly maintained)
- Cargo.toml vs origin/main: single new dependency (ed25519-dalek = "2.1.1");
  tempfile removed from dev-dependencies
- tests/verifier_extends_slice.rs added by verifier: 23 subprocess-level tests
  covering all AC families

## Per-AC verdict (all PASS)

- AC1: Bare directive (URL only) backward-compat — PASS
- AC2: Matching pin admits — PASS
- AC3: Pin mismatch rejects + MalformedDirective for invalid hex — PASS
- AC4: Pin check on stale-cache fallback path — PASS
- AC5: Valid ed25519 signature admits (code-path + unit level) — PASS
- AC6: Missing/invalid sidecar rejects — PASS
- AC7: No pubkey means no sidecar fetch — PASS
- AC8: Legacy trust file (list-of-strings) parses correctly — PASS
- AC9: strict tier: upstream rule not shadowed by local — PASS
- AC10: advisory tier: upstream deprioritised, still present — PASS
- AC11: override tier: triple-equality implicit drop (A/B/C sub-cases) — PASS
- AC12a-h: Tokeniser reject/accept matrix — PASS (all 8 sub-cases)
- AC13: trust --add --pubkey records key; listing shows "(key configured)" — PASS
- AC14: Full local gate clean — PASS

## Affected artefacts

- build: issue-#29 slice (feat/29-extends-policy) — status: complete
- working tree: all changes accumulated, NOT yet committed (correct)

## Next action

Surface to user for commit + PR approval. The dispatcher will stage the
complete file set and open a PR to main referencing issue #29 upon user
approval.

## Commit file set (for dispatcher)

Stage these files explicitly (no git add -A):
  Cargo.toml
  Cargo.lock
  src/extends.rs
  src/parser.rs
  src/store.rs
  src/guardrails.rs
  src/main.rs
  src/canonicalize.rs
  src/enrich.rs
  src/hooks.rs
  src/scenarios.rs
  tests/extends_integration.rs
  tests/verifier_extends_slice.rs
  .taniwha/kupu/orchestrator/current_state.yaml
  .taniwha/kupu/orchestrator/next_action.yaml
  .taniwha/kupu/decisions/01KSRYU8K2BUILD0COMPLETE001.md
  .taniwha/kupu/events/2026/05/29/20260529T041136000Z-01KSRYU8K2BUILD0COMPLETE001.yaml
  .taniwha/kupu/events/index.yaml
  .taniwha/kupu/orchestrator/handoff/01KSRY2BWJB0MDH0FJ5WDTCP2S/outputs/verifier_report.yaml
  tests/verifier_extends_slice.rs (already listed above)

Exclude: .taniwha/kupu/state/
