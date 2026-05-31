# Decision: Amend structural foreground colour — pounamu RGB(61,130,104)

**kind:** contract_amendment  
**triggered_by:** 01KSYJ4588C0TKGJJ0M5Y6FKK9 (user_input_received: ac9_legibility_decision)  
**recorded_at:** 2026-05-31T08:24:54Z

## What was decided

The user elected to lighten the structural foreground colour (pounamu) from the
original design-v5 value RGB(31,77,63) (#1f4d3f) to RGB(61,130,104) (#3d8268).

This is a binding palette amendment. The new value stays in the pounamu green
family and achieves approximately 4.6:1 contrast on both a pure-black (#000000)
and a pure-white (#ffffff) background, passing WCAG AA on each. The original
value was only ~2.1:1 on dark terminals, a genuine legibility defect for
structural text that AC9 exposes.

All other values and behaviour are unchanged:
- Ochre RGB(184,118,58) — unchanged.
- Gating logic (NO_COLOR / CLICOLOR_FORCE / IsTerminal precedence) — unchanged.
- AC8 carve-out (src/hooks.rs not touched) — unchanged.
- Machine-consumed path prohibition (--json renderings, hook-protocol) — unchanged.

## Rationale

The WCAG AA contrast minimum for normal text is 4.5:1. The original pounamu
#1f4d3f has a relative luminance of approximately 0.019, yielding contrast ratios
of ~2.1:1 on black and ~17:1 on white. While legible on white/light terminals,
it is essentially unreadable as-is on dark terminals. The corrected value #3d8268
has relative luminance approximately 0.18, yielding ~4.6:1 on black and ~4.6:1 on
white — narrowly above the AA threshold on both extremes, satisfying AC9
objectively rather than requiring a manual-skip verdict.

## Scope of corrective implementation

A corrective leaf-implementation dispatch (handoff 01KSYJCORRLEFIMPL000001) is
being emitted immediately after this decision record. Its scope is:

- **File changed:** `src/style.rs` ONLY.
- **Constants updated:** POUNAMU_R = 61, POUNAMU_G = 130, POUNAMU_B = 104.
- **Doc comments updated:** all references to RGB(31,77,63) and #1f4d3f become
  RGB(61,130,104) and #3d8268.
- **Co-located unit test updated:** the `structural_emits_pounamu_escape` assertion
  that checks for `\x1b[38;2;31;77;63m` must be updated to `\x1b[38;2;61;130;104m`.
- **Not touched:** tests/brand_palette_verifier.rs — that file belongs to the
  verifier subagent (next dispatch) and must be updated there.
- **Not touched:** src/hooks.rs, src/main.rs, src/stats.rs, src/guardrails.rs —
  all integration surfaces use the `structural()` helper which calls the
  POUNAMU_R/G/B constants dynamically; no literal RGB values appear in those files.
- **Gate:** fmt --check + clippy --all-targets + test all must pass clean.
- **No commit:** working tree only, branch feat/83-cli-palette.

After the corrective leaf returns, a corrective verifier dispatch will re-run
AC1–AC10 with tests/brand_palette_verifier.rs updated to the new structural RGB.

## Affects

- `src/style.rs` — POUNAMU_R/G/B constants + doc comments + unit test escape
  assertion (palette amendment, from v1-impl to corrected-impl)
- Design doc v5 palette value for structural colour — effectively superseded by
  this decision; a v6 design is not warranted for a constant-only amendment.
  Future readers: treat this decision record as the authoritative amendment note.
- Contract `brand-palette-styling/v1.md` — the palette section cites RGB(31,77,63);
  that value is superseded by this amendment. Contract remains v1; a new contract
  version is not warranted for a single-constant amendment that changes no AC
  behaviour or interface shape.
- Handoff 01KSYJCORRLEFIMPL000001 — corrective leaf-implementation dispatched.
