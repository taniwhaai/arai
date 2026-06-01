# Implementation Notes — gateway-outcome-glyphs

## Acceptance Criterion Satisfaction

### AC1 — Outcome-to-glyph mapping is total and fixed
`outcome_glyph` handles all four `Outcome` variants via a `match (outcome, unicode)` arm
with no fallthrough gap. Unit tests `ac1_unicode_glyph_mapping_colorize_false` and
`ac1_ascii_glyph_mapping_colorize_false` verify each outcome produces the exact expected
string and that no outcome returns an empty string.

### AC2 — Unicode decision precedence is fixed and TTY-independent
`should_use_unicode` reads `ARAI_ASCII` → `NO_UNICODE` → locale in priority order; no
terminal check is performed. Six unit tests (one per contract sub-case) cover ARAI_ASCII
override, NO_UNICODE override, no locale, LC_ALL UTF-8, LC_CTYPE UTF-8 (no LC_ALL), LANG
UTF-8 (no LC_ALL/LC_CTYPE), and LC_ALL non-UTF-8 blocking LANG. All serialised via
`GLYPH_ENV_MUTEX` to avoid races with the existing `with_env` serialisation pattern.

Integration tests `ac2_arai_ascii_audit_glyph_is_ascii` and `ac2_arai_ascii_why_glyph_is_ascii`
confirm that with `ARAI_ASCII=1`, no Unicode glyph codepoints appear in output and ASCII glyph
tokens do appear.

### AC3 — Glyph semantics (visual, dispatcher review)
Fixed character sequences: blocked `●·│✕`/`o.|x`, allowed `│●│`/`|o|`, warned `●·│`/`o.|`.

### AC4 — Human `arai audit` and `arai why` show per-outcome glyph
- `cmd_audit`: derives `Outcome` via `audit_entry_outcome` from `decision` + rules[] severity,
  calls `outcome_glyph(outcome, unicode, col)` to prefix each row.
- `cmd_why`: maps severity string to `Outcome`, prefixes each matched-rule line.
- `--json` branches are untouched.
- Integration tests `ac4_audit_human_output_contains_glyph` and `ac4_why_human_output_contains_glyph`
  seed a project, fire a matching hook, then assert glyph presence in human output.

### AC5 — `arai stats` warned glyph replaces generic warning icon
`print_compliance_section` in `src/stats.rs` now uses
`outcome_glyph(Outcome::Warn, unicode, col)` in place of the hardcoded `⚠` string.
`--json` path is untouched.

### AC6 — Hook Pre/Post output carries glyph but zero ANSI colour
`handle_stdin_impl` in `src/hooks.rs` now:
- Imports `style` (added to use list).
- Computes `hook_unicode = style::should_use_unicode()`.
- Prefixes `context` (additionalContext) with
  `outcome_glyph(ctx_outcome, hook_unicode, false)` — `colorize=false` is a literal,
  unconditional constant.
- Prefixes `deny_reason` with `outcome_glyph(Outcome::Block, hook_unicode, false)`.
- Colour helpers (`passage`, `structural`, etc.) are NOT called from hooks.rs.
Integration tests `ac6_hook_output_has_glyph_and_zero_ansi` and
`ac6_hook_output_arai_ascii_has_ascii_glyph_and_zero_ansi` verify glyph presence and
zero 0x1B bytes.

### AC7 — Every `--json` output is glyph-free
No `--json` branch routes through `outcome_glyph`. Integration tests
`ac7_audit_json_has_no_glyph`, `ac7_why_json_has_no_glyph`, and `ac7_stats_json_has_no_glyph`
recursively walk all JSON string fields and assert no glyph codepoints.

### AC8 — Ochre colour only on blocked cross when `colorize = true`
Unit test `ac8_ochre_only_on_unicode_blocked_cross_when_colorize_true` covers all
four corners: Block+unicode+colorize=true (ANSI present), Block+unicode+colorize=false
(bare), Block+ASCII (any colorize, no ANSI), and all non-blocked outcomes x all
unicode/colorize combinations.

### AC9 — Visual distinguishability (dispatcher review)
Glyph forms are fixed in code; visual review by the dispatcher as per contract.

### AC10 — Full gate passes with zero new dependency
- `cargo fmt --all -- --check`: exit 0
- `cargo clippy --all-targets`: 0 new warnings from changed files
- `cargo test`: all 555 tests pass (410 unit + 145 integration); 35 new tests added
- `Cargo.toml`/`Cargo.lock`: unchanged (no new dependencies)

## Hard Carve-outs Confirmed

1. **Hook path colorize=false**: `hooks.rs` calls `outcome_glyph(..., false)` with a literal
   `false`. `should_colorize` is never consulted on the hook path.
2. **`--json` paths glyph-free**: All `if json { ... return Ok(()); }` branches were left
   untouched.
3. **Audit-chain integrity markers**: The `✓`/`✗` in `arai audit --verify` at lines 847-860
   were not modified.

## Changed files
- `src/style.rs` — extended with `Outcome`, `should_use_unicode`, `outcome_glyph`, unit tests
- `src/main.rs` — `cmd_audit` + `cmd_why` human render prefixed; `audit_entry_outcome` helper added
- `src/stats.rs` — `print_compliance_section` warned flag uses `outcome_glyph`
- `src/hooks.rs` — style import added, `handle_stdin_impl` response-building prefixed
- `tests/gateway_outcome_glyphs.rs` — new subprocess integration tests (9 tests)
- `tests/brand_palette_verifier.rs` — updated `ac6_five_helper_set_closed_in_style_rs` count
  (6→8) and `ac1_hooks_rs_not_modified` to reflect PR #84 scope expansion
