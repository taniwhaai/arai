# Decision: Corrective leaf dispatch — AC6 glossary-consistency nits

**kind:** re_raise_resolution  
**triggered_by:** 01KT13PVGPVKR5QKXXQ9YAPD98 (verifier returned partial, invocation 46)  
**affects:**
- kind: implementation, module: copy-tone-audit, from_version: 1 (corrective in-place, same handoff)

---

## What happened

The verifier for the issue-#85 copy-tone-audit slice returned `overall: partial`. The GATE is GREEN (fmt clean, clippy 0 new, 550+ tests pass, Cargo.toml unchanged) and ALL hard constraints are satisfied. The sole reason for `partial` rather than `pass` is three register/glossary-consistency nits under AC6 that the leaf implementation missed:

1. **`src/init.rs:24`** — step header reads `"Extracting guardrails..."` — uses "guardrails" as a unit term where the SelfReferenceGlossary specifies "rule(s)" as the unit. Should read `"Extracting rules..."`.

2. **`src/main.rs:1213, 1337, 1559`** — three state messages read `"No guardrail database found.  Run \`arai init\` first."` — uses "guardrail" as unit term. Should use "rule" (e.g. `"No rule database found.  Run \`arai init\` first."`).

3. **`src/enrich.rs` (~10 strings) and `src/scenarios.rs:89`** — user-visible error paths still use `"Failed to <verb> <thing>: {e}"` form. VoiceSpec rule 2 requires `"Could not <verb> <named thing>: {e}"`. These were outside the original contract's bounded edit surfaces but are user-reaching strings that are inconsistent with the glossary commitment now that all other user-facing paths are retuned.

## Decision

Dispatch a corrective leaf-implementation (handoff `01KT13QCORRLEAF00000001`) scoped ONLY to those three literal-string-change categories across four files:
- `src/init.rs` — one string
- `src/main.rs` — three strings
- `src/enrich.rs` — ~10 strings (user-visible error messages only)
- `src/scenarios.rs` — one string

Constraints on the corrective leaf:
- String literals only — NO logic, JSON keys/enums, colour, glyph, or other changes
- Do NOT touch `docs/voice.md` or other files
- Do NOT add any dependency
- Run FULL gate (cargo fmt --all + clippy --all-targets + cargo test)
- Update any test asserting a changed string in lockstep (expected: none, but must grep)
- Branch `feat/85-copy-tone`, working-tree only (no commit)

After the corrective leaf returns, re-dispatch the verifier (handoff `01KT13QVERIFYRE00000001`) to confirm AC1–AC10 all pass (especially AC6) and re-run the full gate.

## Why a corrective leaf rather than surface to user

The three nits are:
- Editorial only (no behaviour change, no logic change)
- Fully approved by prior user decisions: the SelfReferenceGlossary is committed and the "Could not" form for user-visible errors is already applied everywhere else
- ≤2 dispatch loop (corrective leaf + re-verify) — under the hard rule's exception threshold for small recovery loops
- No new user decision is needed; the approved spec already covers these sites

This is mechanical completion of the approved spec, not a scope change.
