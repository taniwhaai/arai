# Decision: Promote brand-palette-styling contract v1 and dispatch leaf-implementation

**kind:** scope_change  
**triggered_by:** 01KSY1V5TCQQFYCQKJCEN8BEH6 (subagent_returned: contract-derivation 01KSYCONTRACTV5DERIV0000001)  
**recorded_at:** 2026-05-31T03:45:00Z

## What was decided

The contract-derivation subagent returned a clean contract for the `brand-palette-styling`
module (issue #83, design v5). The contract was promoted to its canonical location at
`kupu/contracts/brand-palette-styling/v1.md` and `project.yaml` updated to record
`contracts.brand-palette-styling.current_version: 1`.

A leaf-implementation subagent is being dispatched (handoff `01KSYLEAFIMPLBRAND000000001`)
against this contract. This is the only dispatch required for this build (tier: `single_module`).

## Why

The design tier is `single_module`. The contract derivation returned a contract and
not a re-raise, so promotion + immediate leaf dispatch is the correct path per the
orchestrator skill's building-phase rules.

## Contract summary

The contract defines:
- `src/style.rs` (new): palette constants (pounamu RGB 31,77,63 and ochre RGB 184,118,58),
  `should_colorize` gate with NO_COLOR > CLICOLOR_FORCE > IsTerminal precedence,
  and five semantic helpers: structural, passage, dim, warn, error.
- Modifications to `src/main.rs`, `src/audit.rs`, `src/stats.rs`, `src/guardrails.rs`
  to apply helpers at human-facing call sites.
- `src/hooks.rs` is a deliberate carve-out (AC8) — not touched.
- Zero new dependencies.
- Unit gate-matrix tests in `src/style.rs` and subprocess integration tests in `tests/`.
- Full gate (fmt + clippy + test) required.

## Affects

- `kupu/contracts/brand-palette-styling/v1.md` — created (canonical)
- `kupu/contracts/brand-palette-styling/meta.yaml` — created
- `project.yaml` — updated: contracts.brand-palette-styling.current_version: 1
- Leaf-implementation handoff `01KSYLEAFIMPLBRAND000000001` — dispatched
