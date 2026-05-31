# Manifest: brand-palette-styling (`style` module)

---

## Responsibility

Holds the two fixed brand foreground colours (pounamu and ochre) as 24-bit RGB
constants, decides per output stream whether that stream may receive colour
(the `should_colorize` gate), and provides a closed set of semantic styling
helpers — `structural`, `passage`, `dim`, `warn`, `error` — each of which
returns its input text either wrapped in the appropriate foreground truecolor
escape followed by a reset, or returned byte-identical, according to the gate's
verdict.

---

## Not responsible for

Deciding what text any caller prints, determining which semantic role applies to
a given line of output (that is each call-site's own choice), or producing any
output itself — styled bytes appear only when the caller prints the returned
string. Machine-consumed paths (hook-protocol JSON, every `--json` rendering,
piped / non-terminal output) are an explicit scope boundary: no string on those
paths is ever routed through this module.

---

## Inputs

- **text** (text string, required): The already-formed human-readable span to be
  styled. Constraints: any UTF-8 string; no validation needed; the module
  neither inspects nor mutates the text's content.

- **target stream identity** (enumerated value, required per gate call):
  Identifies which output stream the styled text will be written to — standard
  output or standard error. Required so the gate queries terminal status for the
  correct stream. Two values only: `Stdout` or `Stderr`. Callers obtain a
  gate verdict for a given stream and then style all spans destined for that
  stream using that verdict.

- **process environment** (key-value store, read by the module): The module
  reads exactly two keys: `NO_COLOR` (any value, presence is what matters) and
  `CLICOLOR_FORCE` (any value, presence is what matters). These are not passed
  by the caller; they are read at gate-decision time from the process
  environment.

- **target stream terminal status** (boolean, read by the module): Whether the
  target stream is currently attached to a terminal. Obtained through the
  platform terminal-detection facility already used elsewhere in the codebase.
  No new dependency is introduced; the same mechanism already in use in
  `src/config.rs` is reused. This is not passed by the caller; it is observed
  by the gate.

---

## Outputs

- **gate verdict** (`ColorVerdict`): A boolean per stream — colourise-on or
  colourise-off — derived from the three fixed-precedence inputs described
  under Behavioural Guarantees. Returned by `should_colorize(stream)`.

- **styled span** (`StyledSpan`): The return value of every semantic helper.
  When the gate says colourise-on for the relevant stream: a string equal to
  the input text prefixed by exactly one foreground truecolor escape sequence
  (24-bit RGB, format `ESC[38;2;R;G;Bm`) and suffixed by exactly one reset
  sequence (`ESC[0m`), with nothing between the reset and the end of the string.
  When the gate says colourise-off: a string byte-identical to the input text,
  with no added, removed, or modified bytes. The return type is the same in
  both cases; callers print the return value without branching.

---

## Side effects

None. The module reads the process environment and queries the stream's terminal
status, but these are observations, not writes. The module performs no I/O of
its own. The caller owns all writes. No state is mutated outside the function
call.

---

## Error semantics

The module is total: it has no failure modes and signals no errors. Every
function returns a value for every input. There is no fallible path. The
project's fallible-function convention (`Result<T, String>` for fallible library
functions) does not apply here. Specifically:

- **Terminal-status undetermined**: If the terminal status of the target stream
  cannot be determined, the gate treats the stream as not-a-terminal and returns
  colourise-off. This is the safe direction (never injects styling into an
  unknown sink). This condition is not signalled to the caller; the verdict is
  simply off.

- **Environment variable not set**: Absence of `NO_COLOR` and `CLICOLOR_FORCE`
  is the normal case; it is not an error. The gate proceeds to the terminal-
  status check.

---

## Behavioural guarantees

- **Gate precedence (fixed, non-configurable).**
  The verdict for a stream is computed in this exact order, with no
  configurable overrides:
  1. If `NO_COLOR` is present in the environment (any value, including empty):
     verdict is **off**. This terminates the decision regardless of
     `CLICOLOR_FORCE` or terminal status.
  2. Else if `CLICOLOR_FORCE` is present in the environment (any value,
     including empty): verdict is **on**, regardless of terminal status.
  3. Else: verdict is **on** if and only if the target stream is attached to a
     terminal; **off** otherwise.
  `NO_COLOR` dominates `CLICOLOR_FORCE`; both dominate terminal detection.
  [AC2, AC3]

- **Foreground-only, unconditionally.**
  Every escape sequence this module can emit sets exactly one foreground colour
  attribute. No escape sequence ever sets a background colour attribute. This
  holds for every helper, for every combination of palette entry and intensity
  attribute, and is not contingent on the gate verdict (when off, no escapes
  are emitted at all). [AC7]

- **Truecolor-or-plain — no palette approximation.**
  When the gate verdict is on, colours are emitted using the exact brand RGB
  triples as 24-bit truecolor escapes. The module does not contain a 16-colour
  or 256-colour fallback path. On a terminal where the gate has returned on,
  the truecolor sequence is what is written. If a terminal does not render
  truecolor correctly, that is the terminal's limitation; the module does not
  attempt to detect it or compensate.

- **Reset discipline.**
  Every styled span is self-contained. The foreground escape at the start and
  the reset at the end are within the same returned string, so colour state
  never leaks into text outside the span. Callers do not need to emit resets
  themselves.

- **Plain output is byte-identical to input.**
  When the gate verdict is off, every helper returns exactly the bytes of the
  input text. No bytes are added (no invisible escapes, no zero-width
  characters, no trailing spaces). No bytes are removed or substituted. A
  comparison of the input and the helper's return value on a colourise-off
  stream will always show zero difference. [AC2, AC3, AC4, AC8]

- **Closed semantic-role set — no stoplight.**
  The module exposes exactly five semantic helpers: `structural`, `passage`,
  `dim`, `warn`, `error`. There is no helper that emits green. There is no
  helper that emits red. The `warn` and `error` helpers render in ochre (with
  bold/emphasis emphasis), not in any shade of red. This set is closed; no
  additional helper may be added without a contract revision. [AC6]

- **Machine-consumed paths are a named, deliberate carve-out.**
  The hook-protocol JSON emitted by the stdio hook handler (`src/hooks.rs`),
  every `--json` rendering (audit, stats, why, guardrails, lint, diff), and
  any output observed through a non-terminal sink carry zero bytes from this
  module. This guarantee does not depend on the gate firing correctly: the
  strings on those paths are intentionally never routed through this module's
  helpers. `src/hooks.rs` is not modified. The guarantee is structural, not
  emergent. [AC4, AC8]

- **Determinism.**
  Given the same input text, the same stream identity, the same environment
  state, and the same stream terminal status, every helper returns the same
  string on every invocation. The module holds no mutable state; there is no
  internal counter, cache, or random element.

- **Safe under concurrent invocation.**
  The module holds no shared mutable state. Concurrent invocations with
  independent inputs do not require synchronisation and will not interfere with
  one another.

- **No allocation beyond output string.**
  The module allocates memory proportional to the size of the output string
  (input text plus at most a small, fixed number of escape bytes). It does not
  accumulate state across calls.

---

## Dependencies

None (no other module in this system). The module uses:

- The platform's process environment access (already available; no new
  dependency).
- The platform's terminal-detection facility (already used in `src/config.rs`;
  no new dependency).

No external crate is introduced. No colour crate, no terminal-detection crate,
no temp-dir crate.

---

## Referenced data shapes

All shapes are internal to this single module. No shared vocabulary file is
produced (single_module tier). Shapes are defined inline below.

### Palette

The two fixed brand foreground colours, expressed as exact 24-bit RGB triples.
These values are constants; they are not configurable and do not vary at
runtime.

| Name     | Red | Green | Blue | Role                                              |
|----------|-----|-------|------|---------------------------------------------------|
| pounamu  | 31  | 77    | 63   | Structural and informational text.                |
| ochre    | 184 | 118   | 58   | Decision / passage moments; warn and error styling. |

There is no third brand colour. There is no background colour entry in this
palette.

A "dim/faint" treatment is not a new colour; it is an intensity attribute
(`ESC[2m`) applied to the text colour already in effect, not a new RGB triple.

### ColorVerdict

A two-valued result (on or off) representing whether a given output stream may
receive colour at the moment the gate is called. Derived deterministically from
three inputs in the fixed precedence order described under Behavioural
Guarantees. Carries no state beyond the binary verdict.

### StyledSpan

The return type shared by all five semantic helpers. Concretely, an owned text
string that is either:

- When verdict is on: `ESC[38;2;R;G;Bm` + input text + `ESC[0m` (for
  `structural`, `passage`) or a variant with an additional intensity attribute
  (for `dim`, `warn`, `error` — see Semantic Role Set below).
- When verdict is off: the input text, byte-identical, with no other bytes.

The return type is the same in both cases so callers do not branch on the
verdict.

### Semantic Role Set

The closed set of five helpers, their palette mapping, and their exact escape
sequences when the verdict is on:

| Helper       | Palette colour | Intensity attribute | Escape sequence emitted (verdict on)                  |
|--------------|---------------|--------------------|---------------------------------------------------------|
| `structural` | pounamu        | none               | `ESC[38;2;31;77;63m` + text + `ESC[0m`                |
| `passage`    | ochre          | none               | `ESC[38;2;184;118;58m` + text + `ESC[0m`              |
| `dim`        | (current fg)  | faint (`ESC[2m`)   | `ESC[2m` + text + `ESC[0m`                             |
| `warn`       | ochre          | bold (`ESC[1m`)    | `ESC[38;2;184;118;58m` + `ESC[1m` + text + `ESC[0m`   |
| `error`      | ochre          | bold (`ESC[1m`)    | `ESC[38;2;184;118;58m` + `ESC[1m` + text + `ESC[0m`   |

Notes:
- `dim` applies the faint attribute to the text in its current colour; it does
  not set a new RGB colour.
- `warn` and `error` are deliberately identical in output. Their semantic
  distinction (warn vs. error) is meaningful to callers choosing which to call;
  the rendered result is the same.
- No helper emits green. No helper emits red. No helper sets a background.
- When verdict is off, all five helpers return input text unchanged regardless
  of which helper was called.

---

## Acceptance criteria

### AC1 — Module existence and palette centralisation

**Given** the codebase after the change is applied,
**when** a reviewer searches for any ANSI escape-related string construction or
any reference to the pounamu or ochre RGB triples,
**then** all such constructions are found only inside `src/style.rs`; no other
source file constructs truecolor ANSI escapes or embeds the literal RGB values
`(31, 77, 63)` or `(184, 118, 58)` outside of a call to a `style` module
helper.

Pass condition: exactly one location in the codebase contains the palette
constants and escape-building logic. Fail condition: any other source file
embeds palette values or hand-rolls ANSI escapes.

---

### AC2 — NO_COLOR produces zero ANSI output

**Given** `NO_COLOR` is present in the process environment (any value),
**when** the unit gate-matrix test calls `should_colorize` for any stream
identity with any combination of `CLICOLOR_FORCE` present or absent and any
terminal-status value,
**then** the returned `ColorVerdict` is off,
**and** every helper called with that verdict returns a string that passes a
byte-level check for zero ANSI escape bytes (no byte with value 0x1B appears in
the returned string).

**Also:** The subprocess integration test invokes the binary with `NO_COLOR`
set. For every command whose output is observed, the output contains zero bytes
with value 0x1B.

Pass condition: all assertions pass. Fail condition: any byte 0x1B appears in
helper output when `NO_COLOR` is set, or any assertion fails.

---

### AC3 — Non-terminal stream produces zero ANSI output

**Given** `NO_COLOR` is absent, `CLICOLOR_FORCE` is absent, and the target
stream is not attached to a terminal (non-TTY),
**when** the unit gate-matrix test calls `should_colorize`,
**then** the returned `ColorVerdict` is off,
**and** every helper returns its input text byte-identical (zero ANSI bytes).

**Also:** The subprocess integration test captures the binary's output through
a pipe (a non-terminal sink). The captured output contains zero bytes with
value 0x1B.

Pass condition: all assertions pass. Fail condition: any 0x1B byte appears in
the captured output of a piped subprocess invocation, or any unit assertion
fails.

---

### AC4 — Every `--json` output contains zero ANSI escapes

**Given** any command is invoked with a `--json` flag (audit, stats, why,
guardrails, lint, diff),
**when** the subprocess integration test invokes the binary for each of those
commands with `--json` and captures standard output,
**then** the captured bytes contain zero bytes with value 0x1B.

Pass condition: for each `--json` variant tested, zero 0x1B bytes in output.
Fail condition: any 0x1B byte present in any `--json` output for any tested
command.

---

### AC5 — Correct semantic role applied at integration surfaces

**Given** the integration surfaces (`src/main.rs`, `src/audit.rs`,
`src/stats.rs`, `src/guardrails.rs`) have been updated,
**when** a reviewer reads the routing choices at each call site,
**then**:
- Structural / informational text (section headers, status lines, field labels)
  routes through `structural()`.
- Decision / passage moments (rule firings, prompt matches) route through
  `passage()`.
- The `arai: …` error notices on standard error route through `error()` or
  `warn()`.
- No human-facing line of output routes through a helper whose semantic role is
  inconsistent with its content (e.g., a rule-firing line routed through
  `structural`, or an informational label routed through `passage`).

Pass condition: reviewer confirms each integration surface's routing choices are
consistent with the semantic role set. Fail condition: a call site routes text
through a semantically inappropriate helper, or any `--json` branch routes
through any helper.

---

### AC6 — No stoplight colours introduced

**Given** the full change is applied,
**when** a reviewer inspects all ANSI escapes the module can emit (the closed
helper set),
**then**:
- No helper emits a green foreground escape (no RGB triple that is visually
  green).
- No helper emits a red foreground escape (no RGB triple that is visually red).
- The `warn` and `error` helpers emit ochre (RGB 184, 118, 58) with bold
  emphasis, not any shade of red.
- There is no helper whose name or doc comment implies allow/green or block/red
  semantics.

**Also (unit test):** A unit test asserts that calling `warn("x")` and
`error("x")` with a forced colourise-on verdict produces strings containing the
ochre escape sequence bytes and not any byte pattern corresponding to a red
escape sequence.

Pass condition: all assertions pass and reviewer finds no green or red escape
construction anywhere. Fail condition: any green or red escape byte sequence
present in any helper output, or any helper with stoplight-semantics naming.

---

### AC7 — Foreground-only — no background escape ever emitted

**Given** the module is invoked with colourise-on for any helper with any input
text,
**when** the returned `StyledSpan` string is inspected byte by byte,
**then** the string contains no ANSI background-colour escape sequence. The
background attribute codes 40–47, 48 (256-colour background), 100–107, and the
48;2 (truecolor background) form must not appear in any byte sequence produced
by the module.

**Unit test:** For each of the five helpers, with colourise-on forced, assert
that the returned string contains no byte sequence matching the background-
colour pattern `ESC[4…m`, `ESC[48;…m`, or `ESC[10…m`.

Pass condition: all five helpers pass the assertion. Fail condition: any
background escape byte pattern appears in any helper's output.

---

### AC8 — Hook-protocol output byte-identical to pre-change baseline

**Given** `src/hooks.rs` is not modified,
**when** the subprocess integration test pipes a valid hook-protocol JSON object
(simulating a `PreToolUse` or `PostToolUse` event) to the binary invoked as
`guardrails --match-stdin`,
**then** the output on standard output contains zero bytes with value 0x1B
(zero ANSI escapes), and the JSON structure of the response is parseable and
contains no ANSI escape characters inside any string field (including
`additionalContext` and `reason`).

Pass condition: captured output from `guardrails --match-stdin` contains zero
0x1B bytes and all string fields in the JSON response are ANSI-free. Fail
condition: any 0x1B byte or ANSI escape character sequence present in the hook
output or in any JSON string field.

---

### AC9 — Readable on dark and light terminals (manual verification)

**Given** a terminal configured with a dark background and the same binary run
on a terminal configured with a light background,
**when** a reviewer observes the output of `arai status`, `arai why`, and
`arai guardrails` with colour enabled (a TTY, no `NO_COLOR`),
**then** all text is legible with acceptable contrast on both backgrounds. The
foreground-only truecolor colours (pounamu and ochre) do not become illegible
when the background changes.

Pass condition: reviewer confirms legibility on both dark and light backgrounds.
Fail condition: any text is illegible or unacceptably low contrast on either
background.

Note: This criterion requires human visual inspection. It cannot be satisfied
by automated test. The implementor is responsible for performing this check
before declaring the work complete.

---

### AC10 — Full gate passes

**Given** the change is complete (all files listed under Files Touched are
updated),
**when** the verifier runs the full local gate in the order specified:
1. `cargo fmt --all -- --check`
2. `cargo clippy --all-targets`
3. `cargo test`

**then** all three commands exit with status 0, with no formatting diffs, no
new clippy warnings (warnings introduced by this change — pre-existing warnings
do not count), and all tests pass (including the new unit gate-matrix tests in
`src/style.rs` and the new subprocess integration test in `tests/`).

Pass condition: all three commands return exit status 0. Fail condition: any
command returns non-zero, or clippy reports a warning attributable to code
introduced by this change.

This requirement applies to both the leaf implementor before submitting and the
verifier before approving. Running only `cargo test` is insufficient.

---

## Files touched

| File                    | Change type | Description                                                                                                                                                      |
|-------------------------|-------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `src/style.rs`          | New         | Palette constants; `should_colorize` gate; `structural`, `passage`, `dim`, `warn`, `error` helpers; co-located unit gate-matrix tests.                          |
| `src/main.rs`           | Modified    | Adds `mod style;`; routes human-readable lines in `cmd_status`, `cmd_why`, human `guardrails` listing, and `arai: …` stderr notices through the helpers. `--json` branches left untouched. |
| `src/audit.rs`          | Modified    | Routes rule-firing lines through `passage`. JSONL and `--json` paths untouched.                                                                                 |
| `src/stats.rs`          | Modified    | Routes structural/section text through `structural`; per-firing decision text through `passage`. `--json` untouched.                                             |
| `src/guardrails.rs`     | Modified    | Routes `format_trace` through helpers on the human-facing rendering path only; the trace text that feeds machine-consumed output is not routed through `style`.  |
| `tests/`                | New         | Subprocess integration test: binary path via build-provided mechanism, `ARAI_BASE_DIR` temp isolation, no new dependency. Asserts zero ANSI in `--json`, hook output, piped output, and `NO_COLOR` output. |
| `src/hooks.rs`          | Untouched   | Deliberate carve-out (AC8). Not modified. Listed to make the boundary explicit.                                                                                  |

---

## Out of scope (explicit boundary)

- Glyph or iconography changes.
- Copy-tone or wording changes.
- Any colour in machine-consumed output (hook-protocol JSON, every `--json`
  rendering, piped / non-terminal output). These paths stay byte-identical to
  today.
- Any new dependency (no colour crate, no terminal crate, no temp-dir crate).
- 16- or 256-colour approximation of the brand colours. When colour is off,
  output is plain; there is no reduced-palette fallback.
- Theming, configurable palettes, or per-user colour preferences.
- Background colour handling beyond "do not set any background" (foreground-
  only, unconditionally).
- Automated assertion of visual readability on dark/light backgrounds (AC9 is
  manual).
