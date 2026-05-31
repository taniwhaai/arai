---
version: 5
parent_brief_version: 7
tier: single_module
---

# brand-palette-styling

## Structural tier

**Selected:** single_module

**Justification:** The brief describes exactly one capability — a centralised
colour layer for human-readable CLI output — realised as one new source file
(`src/style.rs`) that exposes a small set of tightly-coupled functions over a
single fixed two-colour palette. The brief's own structure confirms this: AC1
asks for "a `style` module" (singular) that "centralises the palette +
should_colorize gate", and every other AC is a property of how that one module
behaves or where its output appears. The palette constants, the
`should_colorize` gate, and the semantic helpers all share the same state (the
two RGB triples and the per-stream colourisation decision) and none of them is
independently useful or independently swappable: a helper cannot style text
without the gate's verdict, and the gate is meaningless without a helper to
gate. The four call-site files named in the brief (`src/main.rs`,
`src/audit.rs`, `src/stats.rs`, `src/guardrails.rs`) are **mechanical
integration**, not separate modules — each one substitutes a bare print of a
string for a print of a styled-or-plain string returned by the one module, with
no new logic, no new state, and no decision of its own. Promoting any call-site
to a "module" would be layer-slicing (one module per file) against a single
coherent capability, which is the exact failure mode the tier rule exists to
prevent. The total scope is a few hundred lines (the module plus its unit gate
matrix, plus one subprocess integration test), squarely inside the
single_module envelope. There are no subsystems with differing failure
semantics, no composition to wire, and no part that would benefit from an
independent contract — so neither `small_multi_module` nor `full_decomposition`
is warranted.

**Module count:** 1 (`style`). No composition layer; no vocabulary file. The
call-site files are integration surfaces of the one module, enumerated under
**Integration surfaces** below, not modules in their own right.

## Purpose

Give Arai's human-facing terminal output a branded, semantic two-colour palette
— pounamu for structural/informational text and ochre for the "passage" moments
where Arai surfaces a decision (a rule firing, a prompt match) — so that the CLI
reads as branded on both dark and light terminals, while every byte of
machine-consumed output stays exactly as it is today.

## External boundaries

- **standard output / standard error (human-readable command paths)**: outbound,
  text — the rendered lines that `arai status`, `arai why`, the `arai guardrails`
  human listing, `arai audit`, `arai stats`, and the `arai: …` error/warning
  notices on stderr write to a terminal. This is the only boundary where styled
  bytes may appear.
- **standard output (machine-consumed paths)**: outbound, text/JSON — every
  `--json` rendering (audit / stats / why / guardrails / lint / diff), the
  hook-protocol JSON the stdio hook handler emits (`PreToolUse` / `PostToolUse`),
  and any output observed through a pipe or other non-terminal sink. This
  boundary must carry **zero** styling bytes; it is named explicitly as a
  carve-out, not merely left as a consequence of the gate.
- **process environment**: inbound, text — the `NO_COLOR` and `CLICOLOR_FORCE`
  environment variables, read at gate-decision time.
- **target-stream terminal status**: inbound, boolean — whether the specific
  output stream the module is about to write to is attached to a terminal,
  obtained through the platform terminal-detection facility already used
  elsewhere in the codebase (no new dependency).

## Modules

### style

**Responsible for:** Holding the two-colour brand palette as foreground-only
constants, deciding per output stream whether that stream may receive colour,
and offering a small set of semantic text-styling functions that each return the
given text either wrapped in the appropriate foreground truecolor escape (when
the stream is colourisable) or returned unchanged (when it is not).

**Not responsible for:** Deciding *what* text any command prints, *where* in a
command's output a given semantic role applies (that is each call-site's choice
of which helper to call), or styling any machine-consumed output — JSON
renderings and the hook-protocol output never route through this module.

**Inputs:**
- The text span to be styled — a piece of already-formed human-readable text the
  caller wants rendered in a given semantic role. Required, for every helper.
- The identity of the output stream the styled text is destined for (standard
  output versus standard error) — required by the gate so the terminal-status
  check and the per-stream verdict apply to the correct stream. A caller obtains
  the verdict for a stream and styles spans destined for that stream.
- The process environment (`NO_COLOR`, `CLICOLOR_FORCE`) and the target stream's
  terminal status — read by the gate, not passed by the caller.

**Outputs:**
- For each semantic helper: a string equal to the input text wrapped in a
  foreground truecolor escape sequence followed by a reset, **when** the gate
  returns colourise-on for the relevant stream; or a string byte-identical to the
  input text, **when** the gate returns colourise-off. The helper's return type
  is the same in both cases (see **StyledSpan**).
- For the gate: a single boolean verdict per stream — colourise on or off (see
  **ColorVerdict**).

**Side effects:**
- None. The module reads the environment and the stream's terminal status to form
  its verdict and returns strings; it performs no writes of its own. The caller
  owns the actual print. (Reading environment variables and querying terminal
  status are observations, not effects on the outside world.)

**Error semantics:**
- The module is total: it has no failure modes and signals no errors. Every
  function returns a value for every input. There is no fallible path, so the
  project's fallible-function error convention does not apply here. A stream whose
  terminal status cannot be determined is treated as not-a-terminal (the
  colour-off, safe direction), never as an error.

**Behavioural guarantees:**
- **Gate precedence (fixed order).** The gate's verdict for a stream is computed
  as: if `NO_COLOR` is present in the environment → off (regardless of anything
  else); else if `CLICOLOR_FORCE` is present → on (overriding terminal status);
  else → on if and only if the target stream is attached to a terminal. `NO_COLOR`
  dominates `CLICOLOR_FORCE`; both dominate terminal detection. [AC2, AC3]
- **Foreground only.** Every escape this module can emit sets a foreground colour
  and nothing else. No background colour is ever set under any input, so the
  user's terminal background is always respected. [AC7]
- **Truecolor-or-plain, never approximate.** When the gate says on, colours are
  emitted as 24-bit truecolor using the exact brand RGB triples. The module never
  falls back to a 16- or 256-colour approximation of a brand colour; on a terminal
  that the gate has cleared for colour, the truecolor sequence is what is written.
  There is no separate "reduced palette" path. [palette fidelity]
- **No stoplight semantics.** The palette has exactly two brand colours plus a
  dim/faint treatment. No allow-is-green / block-is-red mapping exists; the
  warn and error helpers render in ochre (with emphasis), not red, and there is
  no green helper at all. [AC6]
- **Reset discipline.** Every styled span emitted when colour is on is
  self-contained: the foreground escape is closed by a reset at the end of the
  same returned span, so styling never bleeds into adjacent unstyled text.
- **Idempotent verdict for a stream.** Given the same environment and the same
  stream terminal status, the gate returns the same verdict every time; helpers
  are pure functions of their text input and the gate verdict.
- **Plain output is byte-identical.** When the gate says off, every helper
  returns its input text unchanged — not merely visually equal but byte-for-byte
  equal — so non-terminal, piped, `NO_COLOR`, and machine-consumed paths carry no
  added bytes. [AC2, AC3, AC4, AC8]

**Dependencies:** None (no other module in this system). It uses only the
platform's standard terminal-detection facility and environment access, both
already available in the codebase with no new crate.

## Integration surfaces (mechanical call-site changes — not modules)

These are the four existing files the brief names, where bare prints of strings
are replaced by prints of `style`-helper return values. None contains new
decision logic; each merely chooses which semantic helper a given existing line
should route through. They are listed here so the design is complete, but they
are integration of the one module, not separate modules.

- **command surface (`src/main.rs`)** — adds the module declaration for `style`;
  routes the human-readable lines of `cmd_status`, `cmd_why`, the human
  `guardrails` listing, and the `arai: …` error/warning notices on standard
  error through the appropriate helpers (structural/passage/dim/warn/error). The
  `--json` branches of these commands are left untouched.
- **audit rendering (`src/audit.rs`)** — routes rule-firing lines (a decision /
  "passage" moment) through the passage helper; leaves the JSONL / `--json`
  rendering paths untouched.
- **stats rendering (`src/stats.rs`)** — routes structural/section text through
  the structural helper and any per-firing decision text through the passage
  helper; leaves `--json` untouched.
- **guardrail trace (`src/guardrails.rs`)** — routes `format_trace` through the
  helpers **only on its human-facing rendering path**; the trace text that feeds
  the hook handler's machine-consumed output is not styled.

### Carve-out: hook-protocol JSON output (deliberate scope boundary) [AC8]

The stdio hook handler in `src/hooks.rs` emits JSON consumed by the coding agent
(Claude Code / Grok) on standard output — including the `additionalContext` and
`reason` strings inside `PreToolUse` / `PostToolUse` responses. This output is a
**named, deliberate carve-out**: it must remain byte-identical to today, and
`src/hooks.rs` is intentionally **not** among the files this change touches. No
helper from the `style` module is ever called on any string that becomes part of
the hook-protocol output. This is stated as a first-class scope boundary, not
left to be an incidental consequence of the gate: even though the hook path runs
with its standard output captured (not a terminal, so the gate would return
colour-off anyway), the design forbids routing agent-consumed strings through the
styling module at all, so the guarantee does not depend on the gate firing
correctly in that context. The subprocess integration test asserts this boundary
directly (zero ANSI in the `guardrails --match-stdin` hook output).

## Data shapes

### Palette
The fixed brand colours, foreground-only, expressed as 24-bit RGB triples:
- **pounamu** = RGB (31, 77, 63) — structural / informational text.
- **ochre** = RGB (184, 118, 58) — the "passage" / decision moments (rule
  firings, prompt matches) and the warn/error emphasis colour.
These two triples are the whole palette. There is no third brand colour, no
background colour, and no per-theme or configurable variant. A "dim/faint"
secondary treatment is also available but is an intensity attribute applied to
the existing text colour, not a new brand colour.

### ColorVerdict
The boolean result of the gate for one output stream: colourise-on or
colourise-off. Derived solely from the fixed-precedence inputs `NO_COLOR`
(present → off), `CLICOLOR_FORCE` (present → on, unless NO_COLOR), and the
target stream's terminal status (terminal → on, otherwise off). It carries no
other state.

### StyledSpan
The return contract shared by every semantic helper: a string that is **either**
the input text wrapped in a single foreground truecolor escape closed by a reset
(when the verdict for the destination stream is on) **or** the input text
returned with no added bytes (when the verdict is off). The two cases are the
same return type, so callers print the result identically regardless of the
verdict; the call-site never branches on colour state.

### Semantic role set
The closed set of styling roles the module exposes, each mapping to a palette
treatment:
- **structural** → pounamu foreground (informational / structural text).
- **passage** → ochre foreground (decision / "passage" moments).
- **dim** → faint/secondary treatment of the existing colour.
- **warn** → ochre with emphasis (explicitly **not** red).
- **error** → ochre with emphasis (explicitly **not** red).
This set is closed: there is no allow/green or block/red role, by AC6.

## Acceptance criteria

Single module, so all criteria are owned by `style` and its integration
surfaces; carried verbatim from brief v7.

- **AC1:** a `style` module centralises the pounamu/ochre palette + should_colorize gate.
- **AC2:** NO_COLOR set → zero ANSI in any output.
- **AC3:** output not a TTY (e.g. `arai status | cat`) → zero ANSI.
- **AC4:** every `--json` output (audit/stats/why/guardrails/lint/diff) contains zero ANSI escapes.
- **AC5:** pounamu applied to structural/info text; ochre reserved for decision/passage moments (rule firings, prompt matches) across status/why/audit/stats/guardrails.
- **AC6:** no stoplight green-for-allow / red-for-block introduced anywhere.
- **AC7:** foreground-only — no background is ever set (terminal background respected).
- **AC8:** hook-protocol JSON output (src/hooks.rs Pre/Post) byte-identical to today — no ANSI injected into agent-consumed JSON/additionalContext.
- **AC9:** readable on dark AND light terminals (foreground-only truecolor; verify manually).
- **AC10:** full gate — cargo fmt --all -- --check + cargo clippy --all-targets (no new warnings) + cargo test all pass.

### Where each AC lands

| AC   | Lands on |
|------|----------|
| AC1  | `style` module existence: the palette constants + the gate function live here and nowhere else. |
| AC2  | Gate precedence (NO_COLOR → off) + plain-output byte-identity guarantee. |
| AC3  | Gate precedence (non-terminal → off) + plain-output byte-identity. |
| AC4  | Integration surfaces leave every `--json` branch unrouted through `style`; asserted by the subprocess test. |
| AC5  | Integration surfaces choose structural (pounamu) for info text and passage (ochre) for firing/prompt-match moments across the five human commands. |
| AC6  | Closed semantic-role set: no green role; warn/error render ochre, not red. |
| AC7  | Foreground-only guarantee: no escape ever sets a background. |
| AC8  | Hook-protocol carve-out: `src/hooks.rs` untouched; no agent-consumed string is routed through `style`; asserted by the subprocess hook-output test. |
| AC9  | Foreground-only truecolor on both backgrounds; manual verification (no automated assertion possible for visual readability). |
| AC10 | Full-gate process note below — leaf + verifier run fmt + clippy + test. |

## Testing

### Unit gate-matrix tests (in `src/style.rs`)
Co-located `#[cfg(test)]` tests exercising the gate's full decision matrix and
the helpers' two return shapes:
- NO_COLOR present → verdict off → helper returns input byte-identical (even if
  the stream were a terminal and even with CLICOLOR_FORCE also present, NO_COLOR
  dominates).
- Stream is not a terminal, NO_COLOR absent, CLICOLOR_FORCE absent → verdict off →
  plain output.
- CLICOLOR_FORCE present, NO_COLOR absent → verdict on regardless of terminal
  status → helper output contains the expected truecolor escape and a trailing
  reset, and no background escape.
- Verdict-on helpers emit foreground-only truecolor with the exact brand RGB
  triples; no helper emits a background escape; there is no green-coloured role.

### Subprocess integration test (under `tests/`, repo subprocess pattern)
A cross-module test driving the built binary through the project's standard
subprocess pattern — invoking the binary via the build-provided binary-path
mechanism (`env!("CARGO_BIN_EXE_arai")`), isolating state with an `ARAI_BASE_DIR`
temporary directory, and using **no new dependency** (no temp-dir crate, no
colour crate; temp isolation follows the existing tests' approach). It asserts
the absence of any ANSI escape byte in each machine-consumed path:
- every `--json` rendering (audit / stats / why / guardrails / lint / diff)
  contains zero ANSI escapes; [AC4]
- the hook handler's output for `guardrails --match-stdin` (the agent-consumed
  JSON / additionalContext path) contains zero ANSI escapes; [AC8]
- output observed through a pipe (non-terminal sink) contains zero ANSI escapes;
  [AC3]
- output produced with `NO_COLOR` set contains zero ANSI escapes. [AC2]

The presence-of-colour direction (a terminal with colour forced on yields the
expected escape) is covered by the unit tests rather than the subprocess test,
because the subprocess test's sink is a captured pipe (non-terminal) and the
brief's subprocess assertions are all zero-ANSI assertions.

### Full-gate process note (carried from prior cycles) [AC10]
The leaf implementer and the verifier MUST each run the project's full local
gate before declaring work complete — not `cargo test` alone:
`cargo fmt --all -- --check` (formatting clean), `cargo clippy --all-targets`
(no new warnings), AND `cargo test` (all tests, including the new unit gate
matrix and the new subprocess integration test, pass). CI gates rustfmt and
clippy; running only tests locally is the failure that this note exists to
prevent.

## Files touched

| File              | Change                                                                                                   |
|-------------------|----------------------------------------------------------------------------------------------------------|
| `src/style.rs`    | **New.** Palette constants (foreground-only pounamu/ochre); `should_colorize` gate (NO_COLOR / CLICOLOR_FORCE / terminal-status precedence); semantic helpers (structural / passage / dim / warn / error) returning **StyledSpan**; co-located unit gate-matrix tests. |
| `src/main.rs`     | Add `mod style;`; route `cmd_status`, `cmd_why`, the human `guardrails` listing, and the `arai: …` stderr notices through helpers. `--json` branches untouched. |
| `src/audit.rs`    | Route rule-firing lines through the passage helper. JSONL / `--json` paths untouched.                    |
| `src/stats.rs`    | Route structural/section text through the structural helper; per-firing decision text through passage. `--json` untouched. |
| `src/guardrails.rs` | Route `format_trace` through helpers on its human-facing rendering path only; machine-fed trace text untouched. |
| `tests/`          | New subprocess integration test (build-provided binary path, `ARAI_BASE_DIR` temp isolation, no new dependency) asserting zero ANSI in `--json`, in the hook `guardrails --match-stdin` output, when piped, and with `NO_COLOR` set. |
| `src/hooks.rs`    | **Untouched — deliberate carve-out (AC8).** Listed to make the boundary explicit: no styling reaches agent-consumed output. |

## Out of scope

- **Glyph / iconography changes** (issue #84) — this slice is colour only.
- **Copy-tone audit** (issue #85) — wording is not touched.
- **Any colour in machine-consumed output** — the hook-protocol JSON
  (`src/hooks.rs` Pre/Post), every `--json` rendering, and any piped /
  non-terminal output stay byte-identical to today. This is a hard boundary,
  enforced both by the gate and by not routing those strings through `style` at
  all.
- **Any new dependency** — no colour crate, no temp-dir crate, no terminal crate;
  truecolor escapes are hand-rolled and terminal detection reuses the existing
  facility.
- **16- or 256-colour approximation** of the brand colours — on a non-truecolor
  terminal the safe-and-simple behaviour is no colour, not an approximated
  palette.
- **Theming / configurable or custom palettes** — the two brand colours are
  fixed.
- **Changing the brand colours themselves** or any **background handling** beyond
  "leave the user's background alone" (foreground-only).
- **Automated assertion of visual readability** (AC9) — readability on dark and
  light terminals is verified manually; only the absence/presence of escapes and
  the foreground-only property are machine-checked.
