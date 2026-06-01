# Design v6 approved — proceed to contract derivation

**Decision ID:** 01KT0B56QHN9JWXWZB7077PSM5
**Kind:** scope_change
**Recorded at:** 2026-06-01T01:00:52Z
**Triggered by:** user_input_received event 01KT0B3J1WXN6V7TX9D558WBX0

## Summary

The user approved design v6 (single_module, gateway-outcome-glyphs) without
modification. No notes provided. No requested changes.

## What was decided

Proceed from pre-derivation to contract derivation for the single module
"gateway-outcome-glyphs". Contract-derivation subagent dispatched with handoff
id 01KT0B51P5GZW77R4DJE63C9D9.

## Affects

- design v6 (status: approved, basis for contract derivation)

## Constraints carried into contract derivation

**Tier:** single_module — one module contract only, no shared vocabulary, no
composition layer.

**Module placement:** Extend existing src/style.rs. Not a new src/glyph.rs.

**Binding glyph set (user-approved, verbatim):**
- blocked  Unicode `●·│✕`   ASCII `o.|x`
- allowed  Unicode `│●│`    ASCII `|o|`
- warned   Unicode `●·│`    ASCII `o.|`  (Warn and Inform both map here)

**should_use_unicode() decision logic (TTY-independent):**
- ARAI_ASCII present → false (ASCII)
- NO_UNICODE present → false (ASCII)
- None of LC_ALL / LC_CTYPE / LANG contains utf-8/utf8 → false (ASCII)
- Otherwise → true (Unicode)

**outcome_glyph(outcome, unicode, colorize):**
- Pure function, fixed glyph table, no environment reads internally.

**Ochre cross:** ONLY when outcome=blocked AND unicode=true AND colorize=true.
Reuses #83's ochre/error helper verbatim — zero new colour code.

**Named carve-outs:**
1. Hook path (src/hooks.rs): always passes colorize=false — glyph chars present,
   zero ANSI escape bytes.
2. --json output: zero Unicode glyph codepoints (● U+25CF, · U+00B7, │ U+2502,
   ✕ U+2715) in any field value.
3. Audit-verify ✓/✗ integrity markers: untouched (different semantic).

**Zero new dependency** (std::env for locale/override; no new crates).

**Acceptance criteria:** AC1–AC10 from design v6 / brief v8.

**Full gate:** cargo fmt --all -- --check + cargo clippy --all-targets + cargo test.

**Subprocess integration test required** using env!("CARGO_BIN_EXE_arai") +
ARAI_BASE_DIR isolation pattern.
