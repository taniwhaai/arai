---
schema_version: 1
id: 01KSRXEYF2X29A48ERMC94EWH6
decided_at:
  iso: 2026-05-29T03:46:47.394Z
  filename: 20260529T034647394Z
kind: scope_correction
summary: 'Composition integration-test harness redone: remove src/lib.rs + tempfile dev-dep (deviated from brief''s single-dependency rule); rewrite tests/extends_integration.rs as a subprocess test via CARGO_BIN_EXE_arai per repo convention (cf. #122 hooks_grok_exit_verifier). User-directed.'
affects:
- id: resolve-composition
  kind: composition
  version: 1
- action: remove
  id: src/lib.rs
  kind: file
- action: drop tempfile dev-dep
  id: Cargo.toml
  kind: file
- action: rewrite as subprocess test
  id: tests/extends_integration.rs
  kind: file
triggered_by: 01KSRWY13KFB2JPKAWNB9VNV7K
---
# Decision: Composition integration-test harness redone: remove src/lib.rs + tempfile dev-dep (deviated from brief's single-dependency rule); rewrite tests/extends_integration.rs as a subprocess test via CARGO_BIN_EXE_arai per repo convention (cf. #122 hooks_grok_exit_verifier). User-directed.

## Context

## Options considered

## Resolution

## Rationale

## Consequences
