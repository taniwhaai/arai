# Decision: Brief v9 / Design v7 kickoff

**Decision ID:** 01KT0RDESIGNKICKOFFV9001
**Kind:** scope_change
**Triggered by:** 01KT0RAWPNVJ4S49WHAHBZ7FXT (build_started for brief v9)
**Recorded at:** 2026-06-01T04:55:00Z
**Orchestrator invocation:** 41

## Summary

Brief v9 (GitHub issue #85, copy-tone audit) kicked off. The prior slice
(brief v8 / #84 gateway glyphs) is complete and merged as PR #141
(build_completed event 01KT0E0HT0EAWVG680PZCWTN4M, commit 31f0fe3).

Design-doc agent dispatched for design v7 (handoff 01KT0RDESIGNV9DESIGN00001).

## Affects

- design_doc: from v6 to v7 (v7 pending subagent output)

## Scope of brief v9

Editorial retune only — NOT new logic. The brief asks for:
- User-facing strings across hooks.rs, init.rs, error messages across src/,
  command-output prose in main.rs + stats.rs, and a light README intro pass
  to be moved from a friendly-devtool register to restrained declarative.
- A committed voice spec + self-reference glossary (docs/voice.md) as the
  auditable backbone so future maintainers can hold the register.

## Hard constraints carried into design

- Behaviour UNCHANGED: wording only, no logic change.
- JSON protocol keys/values/structure: UNCHANGED.
- #83 colour behaviour and #84 glyph behaviour: both PRESERVED.
- Tests in lockstep: any changed human string has its asserting test updated
  in the same change; cargo test must remain green.
- ZERO new dependency.
- Full gate: cargo fmt --all + cargo clippy --all-targets + cargo test (both
  leaf implementer and verifier must run the full gate).

## Design agent instructions

Handoff 01KT0RDESIGNV9DESIGN00001. Design must:
1. Declare structural tier from first principles (single_module expected).
2. Commit voice spec with concrete register rules making AC2/AC5/AC6 checkable.
3. Commit self-reference glossary (Arai / rule+guardrails / the model / you).
4. Inventory user-facing surfaces to retune per file.
5. Carry AC1–AC10 verbatim from brief v9.
6. State out-of-scope explicitly.
7. Produce AC mapping table (each AC → which surface + what verifier checks).
8. Note test-lockstep risk (many tests assert exact human strings).

## Next orchestrator step

After subagent returns design_v7.md, surface it to the user for design
approval. On approval, proceed to leaf-implementation (if single_module)
or contract-derivation (if multi-module).
