---
schema_version: 1
id: 01KSR9ZSS5D7GCQK9RXM47EBEP
decided_at:
  iso: 2026-05-29T00:00:00.000Z
  filename: 20260529T000000000Z
kind: scope_change
summary: "Contract returned for hooks-grok-exit; promoting to canonical location and dispatching leaf-implementation"
affects:
  - kind: contract
    id: hooks-grok-exit
    from_version: null
    to_version: 1
triggered_by: 01KSR9TKCMY192GVAYM9MW8FXE
---

# Decision: advance from pre-derivation to building — hooks-grok-exit

## Triggered by

Event `01KSR9TKCMY192GVAYM9MW8FXE` (`subagent_returned`, contract-derivation handoff
`01KSR9GV1PEB6G5F9Q2JFFEQ5Q`).

## What happened

The contract-derivation subagent returned successfully with no re-raise. The output
`contract_manifest.md` is well-formed:

- Module: `hooks-grok-exit`
- AC1–AC7 carried verbatim from design v3
- Language-neutral schema; Rust/cargo conventions noted in implementation notes
- All changes confirmed confined to `src/hooks.rs`
- No shared vocabulary needed (single_module tier)
- No new crate dependencies

## Decision

Promote the contract to its canonical location
(`.taniwha/kupu/contracts/hooks-grok-exit/v1.md`) and dispatch the leaf-implementation
subagent (handoff `01KSR9W013AK08AWPHRQ4KN5PZ`, model `claude-sonnet-4-6`).

The leaf-implementation subagent will edit the existing `src/hooks.rs` (a 1561-line file)
in place. It must not create a greenfield file.

## Why this decision

The single_module tier dictates: one contract derivation, one leaf implementation, one
verifier, build complete. There is no ambiguity in the contract and no re-raise to route.
The only correct next step is implementation dispatch.

## Next

After implementation returns, dispatch the verifier against contract v1.
On verifier pass, mark the build complete.
