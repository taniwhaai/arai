# Task — leaf-implementation (module: gateway-outcome-glyphs)

Implement **gateway-outcome-glyphs** against contract v1
(`inputs/contract-gateway-outcome-glyphs-v1.md`). `inputs/project_context_reduced.yaml`
has conventions. EXTEND the existing `src/style.rs` (do NOT create a new module).

## What to build (in src/style.rs)
- `should_use_unicode() -> bool`: ASCII when `ARAI_ASCII` or `NO_UNICODE` env set;
  else Unicode iff locale (LC_ALL > LC_CTYPE > LANG, first set wins) contains
  "utf-8"/"utf8" (case-insensitive); else ASCII. TTY-INDEPENDENT.
- `outcome_glyph(outcome, unicode, colorize) -> String`: map Block→blocked,
  Warn|Inform→warned, Allow→allowed to the binding table:
  blocked Unicode `●·│✕` / ASCII `o.|x`; allowed `│●│` / `|o|`; warned `●·│` / `o.|`.
  The `✕` (Unicode) / `x` (ASCII) is ochre — reuse the existing style ochre helper
  (passage/error) — ONLY when `colorize` is true; otherwise the bare glyph.
  Reuse the existing `should_colorize` gate where a call site needs the colorize flag.

## Apply at call sites
- src/main.rs — `arai audit` per-firing human render + `arai why` matched-rule human
  render: prefix the row with the outcome glyph (colorize from should_colorize).
- src/stats.rs — replace the generic `⚠` with the warned glyph.
- src/hooks.rs — Pre/Post live surface (deny reason / the human-readable
  additionalContext line): prefix with the glyph, **colorize = false ALWAYS** (glyph
  characters only, NO ANSI colour — preserves #83's hook carve-out).

## HARD carve-outs
- `--json` paths carry NO glyph codepoints (●, │, ·, ✕) and no ASCII glyph tokens.
- The audit-chain `✓`/`✗` integrity markers (src/main.rs ~847-850) stay UNCHANGED.
- NO ANSI colour ever added to src/hooks.rs output.
- ZERO new dependency (do not touch Cargo.toml deps).

## Tests
- src/style.rs `#[cfg(test)]`: outcome→glyph mapping (unicode + ascii); ochre on the
  cross only when colorize; should_use_unicode precedence (ARAI_ASCII/NO_UNICODE
  override; locale UTF-8 vs not). Guard env-var tests against races (follow the
  existing serialise pattern in style.rs).
- A subprocess integration test under tests/ (env!("CARGO_BIN_EXE_arai"),
  ARAI_BASE_DIR temp isolation, NO new dep): glyph present in human audit/why;
  `ARAI_ASCII=1` ⇒ glyph-region bytes all ≤ 0x7F; hook `guardrails --match-stdin`
  output contains the glyph but ZERO `\x1b`; every `--json` output has no glyph codepoints.
- Existing suite must still pass.

## Full gate (AC10 — MANDATORY, in order, fix all): `cargo fmt --all` →
`cargo clippy --all-targets` (no new warnings) → `cargo test`.

## DO NOT COMMIT, do NOT switch branches (branch feat/84-gateway-glyphs; working
tree only — dispatcher commits at end).

## Output: `implementation_manifest.yaml` to outputs/ (module, contract_version:1,
files_modified/created, gate_results + test counts, prose summary, committed:false).
Emit outputs/re_raise.yaml ONLY for a genuine contract gap. Final message: short
confirmation incl. gate results.
