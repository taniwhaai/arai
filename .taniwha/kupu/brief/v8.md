# Brief — Issue #84: gateway-derived outcome glyphs (replace generic warning/blocked icons)

## Source
GitHub issue #84 (parent #64, "epic: UI retune against new brand"). Follow-on to
#83 (the pounamu/ochre palette, merged in PR #140).

## Goal
Where Arai renders generic warning/blocked iconography (⚠️ 🛑 ❌), use glyphs derived
from the Arai "gateway" mark — a vertical line/threshold the ochre dot passes
through. A user looking at a Pre/Post tool-call event in the CLI should tell at a
glance what happened (blocked / allowed / warned), and the visual language should
read as recognisably Arai-native, not generic devtool. Glyphs MUST have ASCII-safe
fallbacks for terminals that won't render Unicode cleanly.

## Chosen glyph set (user-approved — "bar + dot", closest to the line+ochre-dot mark)
| outcome | Unicode | ASCII |
|---------|---------|-------|
| blocked (dot outside the gateway + ochre cross) | `●·│✕` | `o.|x` |
| allowed (dot passing through, centered)         | `│●│`  | `|o|` |
| warned / informed (dot adjacent, pre-passage)   | `●·│`  | `o.|` |

The `✕` is ochre (reuse the #83 palette) ONLY in human TTY output; wherever the
colour gate is off (piped, NO_COLOR, and the hook path) it renders as the bare glyph.

## Context / findings
- Existing iconography is sparse: `✓`/`✗` in audit-chain verify (src/main.rs ~847-850),
  a generic `⚠` in src/stats.rs (~537). No heavy 🛑/❌/⚠️. So this mostly INTRODUCES a
  coherent glyph vocabulary.
- Outcome severity already modelled: intent::Severity = Block / Warn / Inform (+ Allow).
  The glyph is a pure function of severity/outcome.
- #83 shipped src/style.rs with should_colorize + ochre helpers (passage/error) —
  reuse for the ochre cross. ZERO new dependency for this slice too.
- STANDING CARVE-OUT (from #83): the hook-protocol path must NEVER get ANSI colour.
  Glyphs are plain characters (JSON-safe), so they MAY appear in hook output — but
  the ochre cross there must be the bare glyph, uncoloured.

## Design
Add glyph logic to src/style.rs (one presentation module — palette + gating + glyphs;
a sibling src/glyph.rs is acceptable if the design prefers, but reuse should_colorize/
ochre regardless):
- `should_use_unicode() -> bool`: true unless `ARAI_ASCII` (or `NO_UNICODE`) env set,
  AND the locale looks UTF-8 (LC_ALL / LC_CTYPE / LANG contains "utf-8"/"utf8",
  case-insensitive); else ASCII. Independent of TTY (glyphs are safe when piped).
- `outcome_glyph(outcome, unicode, colorize) -> String`: Block→blocked, Warn|Inform→
  warned, Allow→allowed per the table; Unicode or ASCII per `unicode`; ochre on the
  `✕` only when `colorize` (never in the hook path).

Apply across the outcome surfaces:
- src/main.rs — `arai audit` per-firing render + `arai why` matched-rule render:
  prefix rows with the outcome glyph.
- src/stats.rs — replace the generic `⚠` with the warned glyph.
- src/hooks.rs — the live Pre/Post surface (deny reason / human-readable
  additionalContext line): prefix with the glyph (chars only — NO ANSI colour;
  ASCII-fallback-aware).

## Acceptance criteria
- AC1: an `outcome_glyph` function maps Block / Warn|Inform / Allow to the gateway
  glyph set above (Unicode + ASCII).
- AC2: Unicode by default; ASCII fallback when ARAI_ASCII set OR locale non-UTF-8;
  ARAI_ASCII=1 glyph output contains only bytes <= 0x7F.
- AC3: glyph semantics match spec — blocked=dot-outside+cross, allowed=dot-centered,
  warned/informed=dot-adjacent.
- AC4: `arai audit` and `arai why` human (non-json) output show the per-outcome glyph.
- AC5: the generic `⚠` in stats.rs is replaced by the warned glyph.
- AC6: the live hook Pre/Post surface carries the glyph (ASCII-fallback-aware) with
  NO ANSI colour added to hook output (the cross is the bare glyph there).
- AC7: every `--json` output is unchanged (no glyphs in json field values).
- AC8: ochre colour on the `✕` appears only in human TTY output and obeys the #83
  gate (absent under NO_COLOR / non-TTY / hook path).
- AC9: outcomes distinguishable at a glance and read as Arai-native (manual — the
  dispatcher will eyeball the rendered set + ASCII fallback, as it did the WCAG check on #83).
- AC10: full gate — cargo fmt --all -- --check + cargo clippy --all-targets (no new
  warnings) + cargo test; ZERO new dependency.

## Scope / files
- src/style.rs (glyph fns; reuse palette/gate) [or sibling src/glyph.rs].
- src/main.rs (audit + why render), src/stats.rs (⚠ replacement), src/hooks.rs
  (Pre/Post glyph — chars only, no colour).
- Tests: style/glyph unit tests (outcome→glyph mapping, unicode vs ascii, ochre only
  when colorize) + subprocess integration test (env CARGO_BIN_EXE_arai, ARAI_BASE_DIR
  isolation, NO new dep) asserting: glyph present in human audit/why; ARAI_ASCII=1 →
  ascii-only; hook output has the glyph but ZERO ANSI; --json has no glyphs.

## Out of scope / boundaries
- `--json` outputs (structured severity, not glyphs) — unchanged.
- The audit-chain `✓`/`✗` integrity markers (different semantic — verification, not a
  gateway passage) — left as-is.
- NO ANSI colour ever added to src/hooks.rs output (glyph characters only).
- No new dependency. No theming/custom glyph config. Glyph designs for the brand
  marks themselves (#65-67) are separate issues.

## Process requirements (standing lessons)
- Leaf + verifier MUST run the FULL gate (cargo fmt --all + clippy --all-targets +
  cargo test), not just test.
- Integration tests use the subprocess pattern (env!("CARGO_BIN_EXE_arai"),
  ARAI_BASE_DIR temp isolation) — NO new dependency.