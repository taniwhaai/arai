---
schema_version: 1
kind: scope_change
triggered_by: 01KT11X69BBQ1WST13H30S7GV2
affects:
  - kind: contract
    id: copy-tone-audit
    from_version: null
    to_version: 1
---

# Decision: Promote copy-tone-audit contract v1 to canonical; dispatch leaf-implementation

## Context

Contract-derivation subagent (handoff 01KT11CONTRACTDERIVE00001) returned successfully.
Output: `kupu/orchestrator/handoff/01KT11CONTRACTDERIVE00001/outputs/contract-copy-tone-audit-v1.md`.

The contract covers issue #85 (copy-tone editorial retune), design v7, single_module tier.
It specifies AC1–AC10, six hard constraints (HC-1 through HC-6), five named edit surfaces,
committed VoiceSpec (6 rules) and SelfReferenceGlossary (5 terms) verbatim, before/after
exemplar table, and a verifier checklist.

## Decision

1. Promote the contract to canonical path `kupu/contracts/copy-tone-audit/v1.md`.
   Register in `project.yaml` and write `meta.yaml` for the contract family.

2. Dispatch a `leaf-implementation` subagent (handoff 01KT11YFKQZDZT37B6H0381XXY)
   against this contract. The subagent operates on branch `feat/85-copy-tone`, working-tree
   only (no commit), to:
   - Write `docs/voice.md` with VoiceSpec + SelfReferenceGlossary verbatim (AC1).
   - Retune user-facing strings at the five bounded edit surfaces (AC2–AC7).
   - Apply SelfReferenceGlossary consistently (AC6).
   - Update every test asserting a changed string in lockstep (AC9, HC-6).
   - Run the full gate: `cargo fmt --all` + `cargo clippy --all-targets` + `cargo test` (AC10).
   - Hard constraints: behaviour unchanged, JSON keys untouched, #83/#84 preserved, zero new dependency.

## Rationale

The tier is `single_module`. Per the orchestrator skill, this means: one leaf, one verifier,
build complete on pass. No composition phase. The contract is self-contained and does not
interact with any other module's contract. Dispatch is the correct next step.

The no-commit / correct-branch / working-tree-only constraint is explicit in the task to
preserve dispatcher control over the commit. The branch `feat/85-copy-tone` already exists
(created as part of the issue #85 build cycle).
