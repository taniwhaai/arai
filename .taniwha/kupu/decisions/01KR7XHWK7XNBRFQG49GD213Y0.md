---
schema_version: 1
id: 01KR7XHWK7XNBRFQG49GD213Y0
decided_at:
  iso: 2026-05-10T03:05:36.615Z
  filename: 20260510T030536615Z
kind: scope_amendment
summary: Rename ARAI_DB_DIR env var to ARAI_BASE_DIR (honest naming — it's the whole state dir, not just the DB). Honor ARAI_DB_DIR for a deprecation window with stderr warning. Rust field arai_base_dir stays.
affects:
- '#71'
- '#73'
triggered_by: 01KR7W90Y7JV0N5XYHDPCBMVW9
---
# Decision: Rename ARAI_DB_DIR env var to ARAI_BASE_DIR (honest naming — it's the whole state dir, not just the DB). Honor ARAI_DB_DIR for a deprecation window with stderr warning. Rust field arai_base_dir stays.

## Context

## Options considered

## Resolution

## Rationale

## Consequences
