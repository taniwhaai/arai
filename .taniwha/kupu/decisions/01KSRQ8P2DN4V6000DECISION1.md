---
schema_version: 1
decision_id: 01KSRQ8P2DN4V6000DECISION1
kind: scope_change
triggered_by: "01KSRQ6M23VGS0J0KXPYYJKZZ5"
recorded_at: "2026-05-29T02:00:00Z"
affects:
  - kind: design_doc
    id: design
    from_version: 3
    to_version: null   # v4 will be produced by the design-doc subagent
---

# Decision: Dispatch design-doc subagent for brief v6 (issue #29)

## Context

Brief v6 was written at `.taniwha/kupu/brief/v6.md` and a `build_started` event
(01KSRQ6M23VGS0J0KXPYYJKZZ5) was recorded by the dispatcher. The prior slice
(brief v5 / issue #122 exit codes) is complete and merged (PR #136).

The current design (v3) is scoped to `hooks-grok-exit` / brief v5. It cannot serve
brief v6 — the new scope spans `src/extends.rs` (primary), plus the rule pipeline
(`src/parser.rs`, `src/store.rs`, `src/guardrails.rs` for tiering),
`src/main.rs` (trust --pubkey flag), and `Cargo.toml` (ed25519-dalek). Design v4
must be produced from scratch for brief v6.

## Decision

Dispatch a design-doc subagent with:
- Input: brief v6 + project_context (redacted to language/conventions)
- Prior design v3 as `prior_design_version` (to inform the subagent of existing
  patterns and conventions in the codebase — not to extend v3, but as style context)
- Task: produce design v4 for brief v6 (issue #29), declaring tier and module
  decomposition, defining module boundaries, carrying AC1–AC14, and stating
  out-of-scope items

## Tier note

The brief explicitly notes this is likely `small_multi_module` or `full_decomposition`.
Three separable concerns are named:
1. **directive-grammar parsing** — `src/extends.rs` directive tokeniser (pin + tier tokens)
2. **fetch-time verification** — pin check + ed25519 sig verification within `src/extends.rs`
3. **tiering** — cross-cutting concern through `src/parser.rs`, `src/store.rs`,
   `src/guardrails.rs` for tier propagation and rule-ranking

The design-doc subagent must decide the tier. The orchestrator does not force a tier;
this is explicitly the design phase's job.

## Rationale

No design doc exists for brief v6. Pre-design is the correct current build phase.
No open re-raises. No stale work from the prior slice that would block this dispatch.
The single correct next action is to produce the design.
