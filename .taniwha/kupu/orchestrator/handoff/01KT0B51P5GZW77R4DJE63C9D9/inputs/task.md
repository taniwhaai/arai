# Task — contract-derivation (module: gateway-outcome-glyphs)

Derive ONE standalone contract for **gateway-outcome-glyphs** (extend src/style.rs)
from approved design v6 (`inputs/design_v6.md`). Single_module ⇒ one contract, no
vocabulary, no composition. Output: `contract-gateway-outcome-glyphs-v1.md` in this
handoff's outputs/.

Carry AC1–AC10 as verifiable given/when/then pass-fail descriptions.

Binding constraints to encode:
- Glyph table (exact): blocked Unicode `●·│✕` / ASCII `o.|x`; allowed `│●│` / `|o|`;
  warned (Warn AND Inform) `●·│` / `o.|`.
- `should_use_unicode() -> bool`: ASCII when `ARAI_ASCII` or `NO_UNICODE` env is set;
  else Unicode iff locale (LC_ALL > LC_CTYPE > LANG, first set wins) contains
  "utf-8"/"utf8" (case-insensitive); else ASCII. TTY-INDEPENDENT.
- `outcome_glyph(outcome, unicode, colorize) -> String`: Block→blocked, Warn|Inform→
  warned, Allow→allowed; Unicode or ASCII per `unicode`; the `✕` is ochre (reuse #83
  style ochre/error helper) ONLY when `colorize` is true; otherwise the bare glyph.
- Reuse the existing src/style.rs `should_colorize` gate + ochre helper. ZERO new dependency.

Hard carve-outs (contract boundaries):
- **Hook path always passes colorize=false** → glyph characters appear in
  src/hooks.rs Pre/Post output (deny reason / human additionalContext line) but NO
  ANSI colour (preserves #83's hook carve-out). ASCII-fallback-aware.
- **--json glyph-free**: no glyph codepoints (●, │, ·, ✕) and no ASCII glyph tokens
  in any --json field value.
- The audit-chain `✓`/`✗` integrity markers are UNCHANGED (different semantic).

Integration surfaces (name them; contract is for the glyph functions): src/main.rs
(arai audit + arai why human render), src/stats.rs (replace the generic ⚠ with the
warned glyph), src/hooks.rs (Pre/Post glyph, colorize=false).

Testing (encode as ACs the verifier checks):
- Unit: outcome→glyph mapping (both unicode + ascii); ochre present on ✕ only when
  colorize; should_use_unicode precedence (ARAI_ASCII/NO_UNICODE override; locale).
- Subprocess integration (env!("CARGO_BIN_EXE_arai"), ARAI_BASE_DIR isolation, NO new
  dep): glyph present in human audit/why; `ARAI_ASCII=1` ⇒ output bytes all ≤ 0x7F in
  glyph region; hook `guardrails --match-stdin` output contains the glyph but ZERO
  ANSI (\x1b); every --json output has no glyph codepoints.
- AC10 full gate: cargo fmt --all -- --check + cargo clippy --all-targets (no new
  warnings) + cargo test — state as a verifier requirement.

Language-neutral (no Rust code; neutral notation). Emit `re_raise.yaml` instead ONLY
for a genuine design gap (there should be none). Final message: short confirmation
of the file written.
