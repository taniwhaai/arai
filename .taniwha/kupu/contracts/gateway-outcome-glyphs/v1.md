# Manifest: gateway-outcome-glyphs

## Responsibility

Decide once per process whether glyphs render as Unicode or ASCII (by reading override environment variables and the locale, independent of terminal status), and map a tool-call outcome to its gateway glyph string — colouring the blocked cross in ochre only when the caller passes `colorize = true` — both functions added to the existing `src/style.rs`.

## Not responsible for

Deciding which outcome a given rendered line represents (each call-site already knows its severity/outcome); deciding whether the destination stream may receive colour (that is the pre-existing `should_colorize` gate, reused by call-sites — not re-implemented here); emitting any colour on the hook path or any glyph into `--json` output; touching the audit-chain `✓`/`✗` integrity markers (different semantic — left as-is by design).

## Inputs

- **outcome** (`Outcome`, required for `outcome_glyph`): the closed four-value set — Block, Warn, Inform, Allow. Every member of this set must be handled; no outcome is unmapped. The mapping is: Block → blocked glyph; Warn → warned glyph; Inform → warned glyph; Allow → allowed glyph.
- **unicode** (boolean, required for `outcome_glyph`): selects which column of the glyph table to draw from — `true` selects the Unicode forms, `false` the ASCII forms. Callers derive this from `should_use_unicode` for all paths except the hook path, where they derive it the same way (only `colorize`, not `unicode`, is hard-coded on the hook path).
- **colorize** (boolean, required for `outcome_glyph`): `true` permits the blocked glyph's cross character to be wrapped in the ochre/error treatment from the pre-existing style helper; `false` emits bare glyph characters only. On the hook path this argument is always `false` — the call-site hard-codes it, unconditionally and without consulting any colour gate.
- **process environment** (inbound, read by `should_use_unicode`):
  - `ARAI_ASCII` — if present and non-empty, forces ASCII regardless of locale.
  - `NO_UNICODE` — if present and non-empty, forces ASCII regardless of locale.
  - `LC_ALL`, `LC_CTYPE`, `LANG` — locale strings read in that priority order (first one that is set and non-empty wins); if the winning value contains the substring `utf-8` or `utf8` (case-insensitive), Unicode is selected; otherwise ASCII.

## Outputs

- **`should_use_unicode` return value** (boolean): `true` means use Unicode glyph forms; `false` means use ASCII glyph forms. The same environment at the same moment always produces the same value (pure read of env state).
- **`outcome_glyph` return value** (text string): the gateway glyph for the given outcome, drawn from the binding glyph table below, optionally with the blocked cross wrapped in the ochre treatment. The exact character sequences are fixed:

  | outcome  | Unicode form | ASCII form | semantics                                              |
  |----------|-------------|------------|--------------------------------------------------------|
  | blocked  | `●·│✕`      | `o.\|x`    | dot outside the gateway, cross to the right            |
  | allowed  | `│●│`       | `\|o\|`    | dot centered, passing through                          |
  | warned   | `●·│`       | `o.\|`     | dot adjacent, pre-passage (Warn and Inform both map here) |

  The `✕` in the Unicode blocked form is the only character that may carry colour. It is wrapped in the ochre/error treatment from the pre-existing style helper if and only if `colorize = true`, `unicode = true`, and the outcome is blocked. In every other case the returned string is bare glyph characters with no colour bytes, no background, no other escape sequences.

  ASCII forms are 7-bit clean: every byte in the returned string is at or below 0x7F. The ASCII blocked form (`o.|x`) carries no ochre treatment in any circumstance.

## Side effects

None. Both functions read the process environment and return values; they write nothing. Reading environment variables is an observation, not a side effect. The caller owns the actual output.

## Error semantics

- **Both functions are total:** a value is returned for every valid input. No error condition is signalled.
- **Missing or unparseable locale** (`should_use_unicode`): treated as "not clearly UTF-8" — returns `false` (ASCII). This is the safe, conservative direction and is never an error.
- **Missing or unreadable environment variables** (`should_use_unicode`): an absent `ARAI_ASCII` or `NO_UNICODE` means the override is not set (not an error); absent or empty locale variables contribute nothing to the UTF-8 check. The function returns `false` (ASCII) when no variable provides a positive UTF-8 signal.

## Behavioural guarantees

- **Idempotency:** both functions are pure with respect to their observable outputs. `should_use_unicode` returns the same value for the same environment state. `outcome_glyph` returns the same string for the same `(outcome, unicode, colorize)` triple given the same behaviour of the ochre helper. Calling either function repeatedly under constant conditions has no observable effect.
- **Ordering:** no ordering requirement — the functions are stateless and have no sequencing dependency on each other or on any other function.
- **Atomicity:** both functions are single-valued computations; there is no partial-output failure mode. The function either returns a complete string or the process does not proceed past the call.
- **Concurrency:** both functions are safe under concurrent invocation. They hold no shared mutable state. `should_use_unicode` reads the process environment (which is effectively immutable after startup); `outcome_glyph` is a pure function of its arguments.
- **Resource bounds:** both functions allocate a small, constant amount of memory per call (bounded by the length of the glyph strings and any colour-escape bytes around the cross). They make no external calls and perform no I/O.
- **TTY-independence of the Unicode decision:** `should_use_unicode` does not consult terminal status. The Unicode vs ASCII decision is based solely on the override environment variables and locale. Glyphs are plain characters, safe when piped; the decision must not change between a terminal and a pipe.
- **Ochre treatment is strictly bounded:** the only byte sequence this module ever adds beyond the literal glyph characters is a single ochre foreground escape (and its reset) around the blocked cross, and only under the three-way condition `colorize = true AND unicode = true AND outcome = Block`. No other glyph, no other outcome, no other condition ever causes colour bytes to appear in the output.
- **Glyph table is fixed:** the character forms in the table above are the complete, user-approved set. No other glyph forms exist, no outcome is unmapped, and there is no per-user override of the characters themselves.
- **Ochre helper is reused, not re-implemented:** the ochre/error treatment comes from the pre-existing helper in `src/style.rs` (introduced in PR #83). This module adds no new colour code of its own and no new palette entry.
- **ASCII forms are 7-bit clean:** when `unicode = false`, every byte in the return value of `outcome_glyph` is `<= 0x7F`.

## Dependencies

- **Pre-existing `src/style.rs` ochre/error helper (from PR #83):** used internally by `outcome_glyph` to wrap the blocked cross when `colorize = true`. Same-module call — no cross-module import.
- **Pre-existing `src/style.rs` `should_colorize` gate (from PR #83):** used by call-sites to derive the `colorize` argument they pass to `outcome_glyph`. Not called directly by the two new functions; call-site responsibility.
- **Process environment access:** standard facility already available in the codebase — no new dependency is introduced.

## Referenced data shapes

All data shapes are defined inline in this manifest (single-module tier; no vocabulary file). The shapes are:

- **Outcome** — the closed four-value set: Block, Warn, Inform, Allow.
- **OutcomeGlyph** — the binding glyph table (Unicode and ASCII columns, plus ochre-cross rule), reproduced in full in the Outputs section.
- **UnicodeDecision** — the boolean output of `should_use_unicode`, with the fixed-precedence decision rule described in Inputs.
- **GlyphArgs** — the three-value argument to `outcome_glyph`: Outcome (the value), unicode boolean (table column selector), colorize boolean (ochre-cross gate).

## Acceptance criteria

### AC1 — Outcome-to-glyph mapping is total and fixed

**Given** the `outcome_glyph` function exists in `src/style.rs`.
**When** called with each of the four Outcome values (Block, Warn, Inform, Allow), with `unicode = true`, and with `colorize = false`.
**Then** Block returns a string containing `●·│✕`; Warn returns a string containing `●·│`; Inform returns a string containing `●·│` (same as Warn); Allow returns a string containing `│●│`. No outcome returns an empty string or an unmapped value.

**When** called with the same four Outcome values with `unicode = false` and `colorize = false`.
**Then** Block returns a string containing `o.|x`; Warn returns `o.|`; Inform returns `o.|`; Allow returns `|o|`. Every byte in every returned string is `<= 0x7F`.

---

### AC2 — Unicode decision precedence is fixed and TTY-independent

**Given** the `should_use_unicode` function in `src/style.rs`.

**When** `ARAI_ASCII` is set to any non-empty value in the environment.
**Then** the function returns `false` regardless of locale variables and regardless of terminal status.

**When** `ARAI_ASCII` is absent and `NO_UNICODE` is set to any non-empty value.
**Then** the function returns `false` regardless of locale and terminal status.

**When** neither override is set and none of `LC_ALL`, `LC_CTYPE`, `LANG` is set or all are empty.
**Then** the function returns `false`.

**When** neither override is set and `LC_ALL` is set to a value containing `utf-8` or `utf8` (case-insensitive) regardless of the value of `LC_CTYPE` or `LANG`.
**Then** the function returns `true`.

**When** neither override is set and `LC_ALL` is absent and `LC_CTYPE` contains `utf-8` or `utf8`.
**Then** the function returns `true`.

**When** neither override is set and both `LC_ALL` and `LC_CTYPE` are absent and `LANG` contains `utf-8` or `utf8`.
**Then** the function returns `true`.

**When** `ARAI_ASCII` is set and a locale variable is also set to a UTF-8 value.
**Then** the function returns `false` (override wins over locale in all cases).

**In all cases:** terminal/TTY status is not a factor. The function returns the same value whether the output stream is a terminal or a pipe.

**Additionally (7-bit-clean assertion):** when `should_use_unicode()` returns `false`, every string returned by `outcome_glyph` for any Outcome value with `colorize = false` contains only bytes `<= 0x7F`.

---

### AC3 — Glyph semantics match the gateway mark

**Given** the glyph table in the Outputs section.
**When** each glyph form is examined visually.
**Then:** the blocked Unicode form (`●·│✕`) shows the dot to the left of the gateway and a cross to the right; the allowed Unicode form (`│●│`) shows the dot inside the gateway, centered between two uprights; the warned Unicode form (`●·│`) shows the dot adjacent to the gateway, pre-passage. The ASCII forms map the same spatial layout in 7-bit characters. *(Verification is a manual eyeball by the dispatcher; the character sequences themselves are fixed in code.)*

---

### AC4 — Human `arai audit` and `arai why` output shows the per-outcome glyph

**Given** a subprocess test using the build-provided binary path, an isolated state directory via `ARAI_BASE_DIR`, and a seeded audit log containing entries with at least one Block and one Allow outcome.

**When** `arai audit` is run without `--json`.
**Then** the output contains the glyph characters corresponding to the outcomes in the log (e.g., `●·│✕` or `o.|x` for blocked; `│●│` or `|o|` for allowed, depending on locale).

**When** `arai why` is run without `--json` against a command that matches a blocking rule.
**Then** the output contains the blocked glyph.

**When** the same commands are run with `--json`.
**Then** the `--json` output contains none of the glyph characters `●`, `│`, `·`, `✕`, `o.|x`, `|o|`, `o.|` in any field value (see AC7).

---

### AC5 — `arai stats` warned glyph replaces the generic warning icon

**Given** the stats rendering in `src/stats.rs` and at least one warned/informed event in the audit log.

**When** `arai stats` is run without `--json`.
**Then** the output contains the warned glyph (`●·│` if Unicode, `o.|` if ASCII) where the generic `⚠` previously appeared. The generic `⚠` is no longer present in the replaced position.

**When** `arai stats` is run with `--json`.
**Then** the `--json` output contains no glyph characters in its field values (see AC7).

---

### AC6 — Hook Pre/Post output carries the glyph but zero ANSI colour

**Given** a subprocess test piping JSON to `guardrails --match-stdin` in a configuration that triggers a blocking rule.

**When** the hook handler emits its deny reason and `additionalContext` line.
**Then** the output contains the blocked glyph characters (`●·│✕` if Unicode, `o.|x` if ASCII, per locale of the test environment).
**And** the output contains zero ANSI escape bytes (no byte sequence starting with `0x1B`).

**When** `ARAI_ASCII=1` is set.
**Then** the glyph in the hook output consists only of bytes `<= 0x7F`.

---

### AC7 — Every `--json` output is glyph-free

**Given** a subprocess test running each `--json`-capable command (`arai audit --json`, `arai why --json`, `arai stats --json`, and any other command that supports `--json`).

**When** the command produces `--json` output.
**Then** no field value in the JSON output contains any of the Unicode glyph codepoints `●` (U+25CF), `·` (U+00B7), `│` (U+2502), `✕` (U+2715), nor any of the ASCII glyph character sequences `o.|x`, `|o|`, `o.|` appearing as glyph tokens.

---

### AC8 — Ochre colour appears on the blocked cross only when `colorize = true`

**Given** the `outcome_glyph` function.

**When** called with `outcome = Block`, `unicode = true`, `colorize = true`.
**Then** the returned string contains ANSI colour bytes wrapping the `✕` character (specifically the ochre foreground sequence from the pre-existing style helper), and the reset sequence follows.

**When** called with `outcome = Block`, `unicode = true`, `colorize = false`.
**Then** the returned string contains the bare sequence `●·│✕` with no ANSI bytes.

**When** called with any non-blocked outcome (Warn, Inform, Allow), any value of `unicode`, any value of `colorize`.
**Then** the returned string contains no ANSI bytes.

**When** called with `outcome = Block`, `unicode = false`, any value of `colorize`.
**Then** the returned string contains no ANSI bytes (ASCII blocked form is never coloured).

**Hook-path assertion:** the `src/hooks.rs` integration always passes `colorize = false`. This is verified by the subprocess test in AC6 (zero ANSI bytes in hook output, regardless of whether the ochre helper would return colour for the same stream in other contexts).

---

### AC9 — Outcomes are distinguishable at a glance as Arai-native

**Given** the rendered Unicode and ASCII glyph sets as they appear in terminal and piped output.
**When** the dispatcher views the four glyph forms side-by-side (blocked, allowed, warned, and the ASCII equivalents).
**Then** each is distinct from the others, the gateway metaphor is legible, and the visual weight matches the severity (blocked is the heaviest, allowed the lightest, warned in between). *(Manual verification by the dispatcher — no automated assertion is possible for visual aesthetics.)*

---

### AC10 — Full gate passes with zero new dependency

**Given** the implementation is complete and all prior ACs pass.

**When** the verifier runs: `cargo fmt --all -- --check`.
**Then** the command exits with code 0 (all source files are formatted per project style).

**When** the verifier runs: `cargo clippy --all-targets`.
**Then** the command exits with code 0 and produces no warnings that were not present before this change.

**When** the verifier runs: `cargo test`.
**Then** all tests pass, including the new co-located unit matrix in `src/style.rs` and the new subprocess integration test under `tests/`.

**Regarding dependencies:** `Cargo.toml` and `Cargo.lock` show no new entries. Locale and environment-variable detection use standard facilities already available in the codebase. The subprocess integration test uses the build-provided binary path mechanism (`env!("CARGO_BIN_EXE_arai")`) and state isolation via `ARAI_BASE_DIR` with a temporary directory following the existing tests' approach — no new test-support crate is introduced.

---

## Hard contract boundaries (carve-outs)

These three constraints are not preferences — they are scope limits. An implementation that violates any of these fails the contract even if all AC assertions pass.

### Carve-out 1: Hook path — `colorize = false`, always

The `src/hooks.rs` call-site passes `colorize = false` to `outcome_glyph` unconditionally, without consulting `should_colorize`. This is not a dynamic decision. The glyph characters appear in hook output; ANSI colour never does. The contract for the hook path is: `outcome_glyph(outcome, should_use_unicode(), false)`, where the third argument is a literal `false`, not a variable derived from a gate.

Consequence: even if `should_colorize` were to return `true` in the hook context (which it currently does not, but the carve-out does not rely on that), the hook output still contains no colour. The carve-out is structural, not conditional.

### Carve-out 2: `--json` paths — no glyphs

No `--json` branch of any command routes its output through `outcome_glyph`. Machine consumers read the existing structured severity field. The `--json` output is byte-equivalent to its pre-existing form with respect to glyph characters. The carve-out applies to all `--json`-capable commands in the codebase: `arai audit --json`, `arai why --json`, `arai stats --json`, and any others.

Consequence: the implementation must not introduce any path by which a glyph character reaches a `--json` field value, even transitively.

### Carve-out 3: Audit-chain integrity markers — untouched

The `✓` and `✗` characters emitted by `arai audit --verify` (in `src/main.rs`) are hash-chain verification markers with a distinct semantic (chain valid / chain broken). They are not gateway passage glyphs, they are not routed through `outcome_glyph`, and this slice does not touch them. Their appearance, position, and meaning are unchanged.

---

## Integration surfaces (mechanical, not additional modules)

The following files have mechanical call-site changes only — no new decision logic, no new state, no new modules:

- **`src/style.rs`** — extended with the two new functions and co-located unit tests. This is the only file where new logic is added.
- **`src/main.rs`** — `arai audit` per-firing human render and `arai why` matched-rule human render each prefix the outcome row with `outcome_glyph(outcome, should_use_unicode(), colorize)`, where `colorize` is derived from the pre-existing `should_colorize` gate for the destination stream. The `--json` branches are untouched.
- **`src/stats.rs`** — the generic `⚠` icon (approximately line 537) is replaced with `outcome_glyph(Outcome::Warn, should_use_unicode(), colorize)`, where `colorize` is from the pre-existing gate for the stats output stream. The `--json` path is untouched.
- **`src/hooks.rs`** — the live Pre/Post human-facing strings (deny `reason` and human `additionalContext` line) are prefixed with `outcome_glyph(outcome, should_use_unicode(), false)`. The third argument is a literal `false` — hard-coded, unconditional. No gate is consulted on this path.
- **`tests/`** — one new subprocess integration test exercising AC2 (ASCII-only bytes under `ARAI_ASCII=1`), AC4 (glyph present in human audit/why), AC6 (hook output: glyph present, zero ANSI), and AC7 (no glyphs in any `--json` output). No new test dependency is introduced.
