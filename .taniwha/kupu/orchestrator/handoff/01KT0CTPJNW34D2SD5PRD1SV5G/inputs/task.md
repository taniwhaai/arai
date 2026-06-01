# Verifier Task — gateway-outcome-glyphs (contract v1)

## Your role

You are a **verifier** for the `gateway-outcome-glyphs` module on branch `feat/84-gateway-glyphs`.

The leaf implementation returned successfully (gate green: fmt + clippy 0 new + test 555/555, 35 new tests added). Your job is to **independently verify** that the implementation satisfies acceptance criteria AC1–AC10 in the contract, then write `verifier_report.yaml` to your output destination.

## Inputs provided

- `contract-gateway-outcome-glyphs-v1.md` — the full contract (AC1–AC10 + 3 carve-outs)
- `project_context_reduced.yaml` — language/toolchain/conventions
- `implementation_manifest.yaml` — which files were changed and what was changed

## Source files to inspect

All on branch `feat/84-gateway-glyphs` (working tree, no commit):

- `src/style.rs` — the two new functions (`should_use_unicode`, `outcome_glyph`) + co-located unit tests
- `src/main.rs` — `arai audit` + `arai why` human render call-sites
- `src/stats.rs` — warned-glyph replacement for `⚠`
- `src/hooks.rs` — Pre/Post glyph prefix, `colorize = false` literal (carve-out 1)
- `tests/gateway_outcome_glyphs.rs` — new subprocess integration tests
- `tests/brand_palette_verifier.rs` — modified (style pub-fn count 6→8, hooks assertion updated)
- `Cargo.toml` — verify unchanged vs `origin/main`

## What to verify — AC by AC

### AC1 — Outcome-to-glyph mapping is total and fixed

Confirm the code mapping in `src/style.rs`. Every Outcome value (Block, Warn, Inform, Allow) is mapped in both unicode=true and unicode=false modes. Block→`●·│✕`/`o.|x`; Warn→`●·│`/`o.|`; Inform→`●·│`/`o.|` (same as Warn); Allow→`│●│`/`|o|`. No outcome is unmapped or returns empty.

Also confirm via the unit tests in `src/style.rs` that the mapping is tested for all four outcomes in both modes.

### AC2 — Unicode decision precedence is fixed and TTY-independent

Confirm `should_use_unicode()` logic in `src/style.rs`:
- `ARAI_ASCII` non-empty → false (overrides everything)
- `NO_UNICODE` non-empty → false (overrides locale)
- No override + no locale vars → false (conservative default)
- `LC_ALL` contains `utf-8` or `utf8` (case-insensitive) → true
- `LC_ALL` absent, `LC_CTYPE` contains utf-8/utf8 → true
- Both absent, `LANG` contains utf-8/utf8 → true
- No terminal/TTY check is performed

Additionally, when `should_use_unicode()` returns false: ALL bytes from `outcome_glyph` with any Outcome and `colorize=false` must be `<= 0x7F`. You may verify this via subprocess with `ARAI_ASCII=1` or by code inspection.

### AC3 — Glyph semantics match the gateway mark (manual)

Eyeball the glyph table in `src/style.rs`:
- blocked Unicode `●·│✕`: dot left, cross right of gateway
- allowed Unicode `│●│`: dot inside, between two uprights
- warned Unicode `●·│`: dot adjacent, pre-passage
- ASCII forms map same spatial layout

Note your visual impression in the report. No automated assertion needed.

### AC4 — Human `arai audit` and `arai why` show the per-outcome glyph

Write or run subprocess tests (using `env!("CARGO_BIN_EXE_arai")` + `ARAI_BASE_DIR` isolation):
- Seed an audit log with at least one Block and one Allow entry
- Run `arai audit` (no `--json`): confirm glyph characters appear in output
- Run `arai why` against a command that matches a blocking rule: confirm blocked glyph appears
- Run both with `--json`: confirm NO glyph codepoints (`●`, `│`, `·`, `✕`, `o.|x`, `|o|`, `o.|`) appear in any field values (see AC7)

The leaf's `tests/gateway_outcome_glyphs.rs` already covers some of this. You may run those tests AND author additional tests or assertions if you judge coverage insufficient.

### AC5 — `arai stats` warned glyph replaces generic `⚠`

Inspect `src/stats.rs` to confirm `⚠` was replaced with `outcome_glyph(Outcome::Warn, ...)`. Confirm the `--json` path is untouched (no glyph in JSON output).

You may also run a subprocess test: seed audit data, run `arai stats` without `--json`, check for warned glyph; run with `--json`, check no glyph in field values.

### AC6 — Hook Pre/Post output: glyph present, ZERO ANSI colour

This is the most critical carve-out check. Run subprocess:

```
echo '{"tool_name":"Bash","tool_input":{"command":"git push --force origin main"}}' | ARAI_BASE_DIR=<tmp> CARGO_BIN_EXE_arai=... <binary> guardrails --match-stdin
```

Confirm:
1. Output contains glyph characters
2. Output contains **zero ANSI escape bytes** (no byte with value `0x1B`)
3. Under `CLICOLOR_FORCE=1`, output STILL contains zero ANSI escape bytes (this is the key invariant the dispatcher already verified manually — your automated test must confirm it)
4. Under `ARAI_ASCII=1`, all glyph bytes are `<= 0x7F`

**The #83 verifier's original invariant must still hold**: ZERO ANSI escape bytes in hook (`guardrails --match-stdin`) output and in ALL `--json` output, even under `CLICOLOR_FORCE=1`. This is the invariant `tests/brand_palette_verifier.rs` guards. Confirm the updated file still asserts it correctly.

### AC7 — Every `--json` output is glyph-free

Run subprocess tests for each `--json`-capable command:
- `arai audit --json`
- `arai why --json`
- `arai stats --json`

Confirm no field value contains any of: `●` (U+25CF), `·` (U+00B7), `│` (U+2502), `✕` (U+2715), or the ASCII sequences `o.|x`, `|o|`, `o.|` as glyph tokens.

Run under `CLICOLOR_FORCE=1` as well.

### AC8 — Ochre colour appears ONLY on blocked cross when colorize=true

Inspect or test `outcome_glyph` in `src/style.rs`:
- `(Block, unicode=true, colorize=true)` → string contains ANSI bytes wrapping `✕`
- `(Block, unicode=true, colorize=false)` → bare `●·│✕`, no ANSI bytes
- Any non-Block outcome, any params → no ANSI bytes
- `(Block, unicode=false, any colorize)` → no ANSI bytes (ASCII forms never coloured)

Also confirm: `src/hooks.rs` passes `colorize = false` as a **literal constant** (not a variable) — code inspection sufficient.

Additionally, confirm the **ochre cross appears ONLY in human TTY output, never in hook or NO_COLOR contexts**:
- Under `NO_COLOR=1`: the ochre glyph (if `arai audit` or `arai why` is the command) must not contain ANSI bytes
- Under `CLICOLOR_FORCE=1` in a non-hook context: ANSI bytes may appear on the blocked cross in human output (this is expected)

### AC9 — Outcomes are distinguishable (manual)

Eyeball the four glyph forms. Note your impression: are they visually distinct? Does the gateway metaphor read naturally? Does weight match severity? State your verdict in the report.

### AC10 — Full gate passes with zero new dependency

Run the full gate yourself, independently of the leaf's reported results:
- `cargo fmt --all -- --check` — must exit 0
- `cargo clippy --all-targets` — must exit 0, no new warnings vs the pre-existing baseline
- `cargo test` — all tests pass

Verify `Cargo.toml` is unchanged vs `origin/main`:
```bash
git diff origin/main -- Cargo.toml
```
Must produce no output (no diff).

## What you MUST NOT do

- Do not modify any production source file (`src/*.rs`)
- Do not modify `Cargo.toml` or introduce any new dependency
- You MAY add a new test file (e.g. `tests/gateway_glyphs_verifier.rs`) or add to an existing test file — but only test code, no production logic
- Do not commit anything

## Output

Write `verifier_report.yaml` to:
`/home/matt/r/arai/.taniwha/kupu/orchestrator/handoff/01KT0CTPJNW34D2SD5PRD1SV5G/outputs/verifier_report.yaml`

Schema:
```yaml
overall: pass | partial | fail
gate:
  fmt: pass | fail
  clippy: pass | fail
  test: pass | fail
  cargo_toml_unchanged: pass | fail
per_ac:
  ac1: pass | fail | partial
  ac2: pass | fail | partial
  ac3: pass | fail | partial   # manual eyeball
  ac4: pass | fail | partial
  ac5: pass | fail | partial
  ac6: pass | fail | partial   # most critical — zero ANSI in hook under CLICOLOR_FORCE
  ac7: pass | fail | partial
  ac8: pass | fail | partial
  ac9: pass | fail | partial   # manual eyeball
  ac10: pass | fail | partial
evidence:
  ac1: <brief description of what you checked>
  ac2: <brief description>
  ac3: <visual impression>
  ac4: <what you ran and what you saw>
  ac5: <what you checked>
  ac6: <subprocess commands run, output observed, ANSI byte check result>
  ac7: <json commands run, glyph search result>
  ac8: <what you inspected or tested>
  ac9: <visual impression>
  ac10: <gate command output summary>
findings:
  - <any AC that did not fully pass: describe exactly what failed and what the implementation must fix>
```

If `overall: fail` or `overall: partial`, list specific findings the leaf must address before the build can complete.

If `overall: pass`, leave `findings` as an empty list.
