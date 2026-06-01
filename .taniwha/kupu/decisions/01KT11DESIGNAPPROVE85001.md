# Design v7 approved — dispatching contract-derivation for copy-tone-audit

**Decision ID:** 01KT11DESIGNAPPROVE85001
**Kind:** scope_change
**Recorded:** 2026-06-01T07:30:31Z
**Orchestrator invocation:** 43
**Triggered by:** user_input_received event 01KT11CZSF25SMP4P1D98ABF8Y

## Summary

User approved design v7 (clean Approve, no notes). Contract derivation for
module copy-tone-audit is now dispatched (handoff 01KT11CONTRACTDERIVE00001).

## What was approved

Design v7 for issue #85 — restrained-declarative voice retune of Arai's
user-facing strings. Key properties confirmed by the approval:

- **Tier:** single_module. One module (copy-tone-audit), no composition layer.
- **Module count:** 1. No shared types. No vocabulary extension needed.
- **Deliverable:** Retune user-facing string *content* at five bounded edit
  surfaces (src/hooks.rs, src/init.rs, user-visible errors across src/,
  src/main.rs + src/stats.rs, README intro/tagline) plus author docs/voice.md.
- **VoiceSpec:** 6 rules (declarative, specific, adequate-at-deny, no
  anthropomorphism, sentence case, preserve #83/#84).
- **SelfReferenceGlossary:** 4 concepts (Arai / rule(s)+guardrails / the model /
  you), one term per concept, held consistently.
- **ACs:** AC1-AC10 all mapped to specific verifier checks.
- **Hard constraints:** Behaviour unchanged, JSON protocol preserved, #83
  colour + #84 glyph preserved, zero new dependency, lockstep tests mandatory,
  full gate required.
- **Open questions:** None.

## What was decided

Proceed directly to contract derivation for module copy-tone-audit. No re-design
needed. No notes to carry forward.

**Contract derivation dispatch:**
- Handoff ID: 01KT11CONTRACTDERIVE00001
- Role: contract-derivation
- Model: claude-sonnet-4-6
- Inputs: design/v7.md + project_context.yaml
- Expected output: contract-copy-tone-audit-v1.md

The contract task brief specifies verbatim carrythrough of: VoiceSpec (6 rules),
SelfReferenceGlossary (4 concepts), before/after exemplars (illustrative register
ground truth), AC1-AC10 as given/when/then pass-fail descriptions, bounded
edit-surface inventory, 6 hard constraints as a numbered list, and explicit verifier
obligations as a checklist (voice consistency review + full gate + lockstep grep).

## Affected artefacts

| Kind | ID | Version transition |
|------|----|--------------------|
| design_doc | kupu/design/v7.md | current (approved) |
| contracts | copy-tone-audit | none yet → to be derived as v1 |
