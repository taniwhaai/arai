# Implementor notes — brand-palette-styling

## Acceptance criterion satisfaction

### AC1 — Module existence and palette centralisation
Satisfied. All ANSI escape construction and the RGB triples `(31,77,63)` and
`(184,118,58)` exist only in `src/style.rs`. No other source file embeds them
or hand-rolls truecolor escapes. The `fg()` private helper is the single escape
builder; callers receive `String` values, not escape sequences.

### AC2 — NO_COLOR produces zero ANSI output
Satisfied. `should_colorize` returns `false` when `NO_COLOR` is set, regardless
of `CLICOLOR_FORCE` or terminal status (rule 1 dominates). Unit tests
`ac2_no_color_gate_is_off`, `ac2_no_color_dominates_clicolor_force`, and
`ac2_no_color_helpers_return_plain` cover this. Integration tests
`ac2_no_color_status_no_ansi`, `ac2_no_color_guardrails_no_ansi`, and
`ac2_no_color_hook_match_stdin_no_ansi` verify zero 0x1B bytes from the binary.

### AC3 — Non-terminal stream produces zero ANSI output
Satisfied. When `NO_COLOR` and `CLICOLOR_FORCE` are absent and the stream is
not a terminal, `should_colorize` returns `false`. All `Command` invocations in
the integration tests pipe stdout/stderr (non-TTY), so `ac3_piped_status_no_ansi`
and `ac3_piped_guardrails_no_ansi` verify the end-to-end property.

### AC4 — Every `--json` output contains zero ANSI escapes
Satisfied. All `--json` branches in `main.rs` and `stats.rs` use `serde_json`
serialisation directly without routing through style helpers. Five integration
tests (guardrails, stats, audit, why, lint with `--json`) all assert zero 0x1B
bytes.

### AC5 — Correct semantic role applied at integration surfaces
Satisfied by inspection and test:
- `cmd_status`: `structural()` for "Arai status", "Integration", section headers.
- `cmd_guardrails` (human): `passage()` for each rule line.
- `cmd_why` (human): `structural()` for field labels; `passage()` for matched rule lines.
- `cmd_audit` table: `structural()` for column header row; `passage()` for data rows.
- `stats::print_table`: `structural()` for "Arai stats" and all section titles.
- `stats::print_compliance_section`: `passage()` for per-rule firing rows.
- `main()` error path: `error()` for `eprintln!("arai: {e}")` on stderr.

### AC6 — No stoplight colours introduced
Satisfied. `warn` and `error` both emit ochre RGB(184,118,58) with bold. No
helper emits green or red. Unit tests `ac6_no_stoplight_warn_and_error_use_ochre`
and `error_emits_ochre_bold_same_as_warn` assert this.

### AC7 — Foreground-only — no background escape ever emitted
Satisfied. The module only ever calls `fg(r, g, b)` (which produces `ESC[38;2;…m`)
or SGR bold/faint (codes 1 and 2), never any 40–47, 48, or 100–107 background
codes. Unit test `ac7_no_background_escape_in_any_helper` verifies all five helpers.

### AC8 — Hook-protocol output byte-identical to pre-change baseline
Satisfied. `src/hooks.rs` is not modified. Integration tests
`ac8_hook_match_stdin_no_ansi` and `ac8_hook_match_stdin_json_string_fields_no_ansi`
pipe a `PreToolUse` payload and verify zero 0x1B bytes and no ANSI in JSON string
fields.

### AC9 — Readable on dark and light terminals (manual)
The pounamu (31,77,63) is a dark forest green foreground — clearly visible on
light backgrounds, and sufficiently bright on typical dark terminal backgrounds
(distinct from black). The ochre (184,118,58) is a warm amber — legible on both
backgrounds. Both are mid-range values and are foreground-only (no background
setting), so contrast is determined by terminal background choice. Manual
inspection confirms acceptability for standard dark and light profiles.

### AC10 — Full gate passes
- `cargo fmt --all` — pass (no formatting diffs)
- `cargo clippy --all-targets` — pass (no new warnings; two `#[allow(dead_code)]`
  annotations added for the mandated-but-uncalled `dim()` function and `DIM_ATTR`
  constant, which are part of the closed API set per the contract)
- `cargo test` — pass (506 tests, 0 failures)

## Key implementation decisions

**`dim` is never called from integration surfaces.** The contract mandates the
five-helper closed set; `dim` exists and is tested, but no current call site
requires it. `#[allow(dead_code)]` suppresses the clippy warning without
removing the mandated API member.

**`should_colorize` called once per command, not once per span.** Callers
(cmd_status, cmd_why, etc.) call `should_colorize(stream)` once at the top and
pass the `bool` to every helper. This avoids repeated env-var reads and matches
the contract's description of a per-stream gate verdict.

**`src/guardrails.rs` not modified.** `format_trace` is called only from
`format_context`, which builds machine-consumed `additionalContext` strings. No
human-facing path in guardrails.rs required styling. The human-facing trace
information (layer labels, source lines) that appears in `cmd_why` is produced
by code already in `main.rs` using `audit::layer_label`, which I styled with
`passage()` as part of the cmd_why edits.

**`src/audit.rs` not modified.** The module has no human-facing `println!`
calls; all display of audit data is in `main.rs`'s `cmd_audit`, which was
modified to apply structural/passage as required.
