# Decision: Design v5 approved — advance to contract derivation

**Decision ID:** 01KSYCONTRACTDERIV000DEC001
**Kind:** scope_change
**Recorded:** 2026-05-31T03:32:36Z
**Triggered by:** user-input-01KSY1CZP9GMNB9Z54JP11KKQY (design_doc_approval response)
**Orchestrator invocation:** 29

## What was decided

The user approved design v5 (single_module, brand-palette-styling, issue #83)
without modification. Response was "Approve" with no notes or change requests.

## Affects

- `kupu/design/v5.md` — promoted to canonical design, version 5, parent brief v7
- Build phase advances from design-pending-approval to pre-derivation

## What this unlocks

A contract-derivation subagent (handoff 01KSYCONTRACTV5DERIV0000001) is
dispatched to produce the single module contract for brand-palette-styling.

## Contract scope locked by this decision

The contract-derivation subagent is bound by the following (from design v5):

- **Module:** brand-palette-styling (`src/style.rs` — new file)
- **Tier:** single_module (no composition, no vocabulary file)
- **Palette:** pounamu RGB(31,77,63), ochre RGB(184,118,58) — foreground-only, exact, fixed
- **Gate precedence (fixed):** NO_COLOR present → off; CLICOLOR_FORCE present → on;
  target stream is terminal → on; else → off
- **Constraints:** hand-rolled ANSI truecolor, ZERO new Cargo dependencies,
  foreground-only (no background escapes), no stoplight semantics,
  plain output byte-identical to input when colour is off
- **Data shapes:** ColorVerdict, StyledSpan, closed semantic role set
  (structural/passage/dim/warn/error — no green, no red)
- **AC8 carve-out:** src/hooks.rs deliberately untouched; hook-protocol JSON
  byte-identical to today; no string routed through style reaches agent output
- **Integration surfaces:** src/main.rs, src/audit.rs, src/stats.rs,
  src/guardrails.rs (mechanical substitution only, --json branches never styled)
- **Subprocess test:** CARGO_BIN_EXE_arai pattern, ARAI_BASE_DIR isolation,
  no new test dependency, zero-ANSI assertions for --json, hook output, pipe, NO_COLOR
- **Full gate [AC10]:** cargo fmt --all -- --check + cargo clippy --all-targets + cargo test

## Build path from here (single_module)

1. contract-derivation returns → place at kupu/contracts/brand-palette-styling/v1.md
2. dispatch leaf-implementation (inputs: contract v1 + project_context)
3. dispatch verifier (inputs: contract v1 + project_context + implementation_manifest)
4. verifier passes → mark build complete
