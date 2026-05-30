# Decision: Promote contract-derivation outputs and establish build tree

**Kind:** scope_change
**Triggered by:** event 01KSRRWCWNCE9PDKTBYWWGE6WF (subagent_returned: contract-derivation handoff 01KSRR3G6XZ8RP5PEH2CSHWG2A)
**Date:** 2026-05-29

## What was decided

The contract-derivation subagent returned successfully with five output files for design v4 (issue #29, slice extends-pinning-signing-tiering). All five artefacts were promoted to canonical paths:

1. **Shared vocabulary** → `kupu/vocabulary/v1.md`
   Defines: ParsedDirective, MalformedDirective, Tier, TrustEntry, TrustFile, RuleProvenance,
   and three named errors. Covers all data shapes shared across the three leaf modules.

2. **directive-tokenisation contract v1** → `kupu/contracts/directive-tokenisation/v1.md`
   Pure computation module. Classifies a raw arai:extends line into ParsedDirective or
   MalformedDirective. No I/O. ACs: AC1, AC3, AC12a-h, AC14.

3. **fetch-verification contract v1** → `kupu/contracts/fetch-verification/v1.md`
   Owns pin comparison, ed25519 sidecar verification, and backward-compatible trust-file
   schema. ACs: AC2, AC3, AC4, AC5, AC6, AC7, AC8, AC13, AC14.

4. **tier-provenance contract v1** → `kupu/contracts/tier-provenance/v1.md`
   Carries tier and source_label through the inline step into rule provenance.
   Modifies parser.rs, store.rs, guardrails.rs. ACs: AC9, AC10, AC11, AC1 (tier half), AC14.

5. **resolve-composition contract v1** → `kupu/contracts/resolve-composition/v1.md`
   Wires the three leaves in sequence within resolve(). Owns cross-module integration
   tests. ACs: end-to-end backward compatibility, wiring order, per-directive isolation, AC14.

## Build tree established

`tree/current.yaml` written with four nodes (three leaves + one composition). Sequential
dispatch order decided (see below).

## Sequential dispatch rationale

The three leaf modules share overlapping source files — specifically src/extends.rs is
modified by all three, and tier-provenance additionally touches parser.rs, store.rs, and
guardrails.rs. Dispatching them to parallel worktrees would produce conflicting diffs on
src/extends.rs at merge time. The correct strategy is sequential dispatch where each leaf
is implemented, verified, and committed to the working tree before the next is dispatched.

Dispatch order:
1. directive-tokenisation — pure, foundational types (ParsedDirective, Tier). No sibling deps.
2. fetch-verification — consumes ParsedDirective, adds ed25519-dalek to Cargo.toml. Depends on #1.
3. tier-provenance — consumes Tier + admitted content, cross-cuts four files. Depends on #2.
4. resolve-composition — wires all three leaves, writes integration tests. Depends on #1-3.

## Affects

- vocabulary: none → v1 (new)
- contracts/directive-tokenisation: none → v1 (new)
- contracts/fetch-verification: none → v1 (new)
- contracts/tier-provenance: none → v1 (new)
- contracts/resolve-composition: none → v1 (new)
- tree/current.yaml: none → established (new)
