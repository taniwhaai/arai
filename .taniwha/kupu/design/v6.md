---
version: 6
parent_brief_version: 8
tier: single_module
---

# gateway-outcome-glyphs

## Structural tier

**Selected:** single_module

**Justification:** The brief describes exactly one capability — turning a Pre/Post
tool-call outcome (Block / Warn / Inform / Allow) into a short gateway-derived
glyph string, with a Unicode-or-ASCII decision and an ochre-or-bare decision baked
into that one function — plus mechanical application of that string at the human
render sites the brief names. The brief's own structure confirms the singularity:
AC1 asks for "an `outcome_glyph` function" (singular) that maps the outcome set to
the glyph table, and every other AC is either a property of how that one function
behaves (AC2 ASCII fallback, AC3 semantics, AC8 ochre gate) or a statement of
*where* its output appears (AC4 audit/why, AC5 stats, AC6 hook, AC7 the --json
exclusion). The two functions in scope share the same closed inputs (the outcome
enum, the unicode flag, the colorize flag) and the same fixed glyph table; neither
is independently useful or independently swappable — `outcome_glyph` is meaningless
without the table, and `should_use_unicode` exists only to feed `outcome_glyph`'s
`unicode` argument. The call-site files the brief names (`src/main.rs`,
`src/stats.rs`, `src/hooks.rs`) are **mechanical integration**, not modules: each
substitutes a bare or generic-icon string for the glyph string returned by the one
module, with no new state and no decision of its own beyond *which* outcome a given
existing line represents. Promoting any call-site to a module would be layer-slicing
against a single coherent capability — the exact failure the tier rule exists to
prevent. Total scope is well under a few hundred lines (two functions, a fixed
table, a co-located unit matrix, and one subprocess integration test). No
subsystems, no differing failure semantics, no composition to wire — neither
`small_multi_module` nor `full_decomposition` is warranted.

**Module count:** 1 (`outcome-glyphs`). No composition layer; no vocabulary file.
The call-site files are integration surfaces of the one module, enumerated under
**Integration surfaces** below, not modules in their own right.

## Module placement decision (committed)

**Decision: extend the existing `src/style.rs` — do NOT add a sibling `src/glyph.rs`.**

The brief permits either ("Add glyph logic to `src/style.rs` … a sibling
`src/glyph.rs` is acceptable if the design prefers, but reuse `should_colorize`/
ochre regardless"). This design commits to **extending `src/style.rs`** for the
following brief-tied reasons:

- The ochre cross (`✕`) on the blocked glyph **must** be rendered with the same
  ochre treatment #83 already exposes, and the colorize decision **must** obey the
  same `should_colorize` gate #83 already owns. Both the ochre helper and the gate
  live in `src/style.rs`. Keeping the glyph function in the same module makes that
  reuse a same-module call rather than a cross-module dependency, and keeps the
  single presentation concern — "how Arai's human-facing terminal output looks" —
  in one file, which is what #83's own design committed to ("one presentation
  module — palette + gating + glyphs").
- A sibling `src/glyph.rs` would have exactly one dependency (`style`) and exist
  only to call back into it for the ochre cross and the colour gate. That is an
  extra module boundary with no independent contract, no independent state, and no
  independent verifier — the agent's decomposition bias, not a brief requirement.
- The reuse is **mandatory and explicit**: the blocked glyph's `✕` is wrapped by
  #83's ochre/error helper **only when `colorize` is true**, and the `colorize`
  argument the call-sites pass is itself derived from #83's `should_colorize` gate
  for the destination stream (except the hook path, which always passes
  `colorize=false` — see carve-outs). No new colour code, no new gate logic, no new
  dependency is introduced by this slice.

## Open questions

None. The brief (v8) is detailed: the glyph table is binding and user-approved,
the outcome→glyph mapping is fixed (Block→blocked, Warn|Inform→warned,
Allow→allowed), the unicode-decision inputs are enumerated (ARAI_ASCII / NO_UNICODE
override plus a UTF-8 locale check, TTY-independent), the ochre-only-when-colorize
rule is stated, and every carve-out (hook colorize=false, --json exclusion,
audit-verify markers left as-is) is named. No load-bearing gap remains that would
change module structure, contracts, or behavioural guarantees.

## Purpose

Give Arai a single, recognisably-Arai outcome vocabulary — gateway-derived glyphs
(an ochre dot passing, or failing to pass, a vertical threshold) — so that a person
reading a Pre/Post tool-call event in the CLI can tell at a glance whether the call
was blocked, allowed, or warned, with an ASCII-safe rendering for terminals that
won't show Unicode cleanly, while machine-consumed output stays exactly as it is
today.

## External boundaries

- **standard output / standard error (human-readable command paths)**: outbound,
  text — the rendered lines of `arai audit`, `arai why`, and `arai stats` that go
  to a terminal. The only boundary where the ochre-coloured cross may appear.
- **hook-protocol surface (`PreToolUse` / `PostToolUse`)**: outbound, text inside
  JSON — the deny `reason` and the human-readable `additionalContext` line the
  stdio hook handler emits to the coding agent. The glyph **characters** may appear
  here (they are plain, JSON-safe characters); **no ANSI colour ever may** (the
  cross is the bare glyph on this path — preserves #83's hook carve-out).
- **machine-consumed `--json` paths**: outbound, text/JSON — every `--json`
  rendering (audit / stats / why / …). This boundary carries **no glyphs at all**;
  it expresses outcome via the existing structured severity field only. Named as a
  carve-out, not left as a consequence.
- **process environment**: inbound, text — `ARAI_ASCII`, `NO_UNICODE`, and the
  locale variables `LC_ALL` / `LC_CTYPE` / `LANG`, read by `should_use_unicode`;
  plus `NO_COLOR` / `CLICOLOR_FORCE` and the target stream's terminal status, read
  by the **reused** #83 `should_colorize` gate (not by this slice's own code).

## Modules

### outcome-glyphs

**Responsible for:** Deciding once per process whether glyphs should render as
Unicode or ASCII (from the override env vars and the locale, independent of TTY),
and mapping a tool-call outcome to its gateway glyph string in the chosen
character set — colouring only the blocked glyph's cross in ochre, and only when
the caller passes `colorize=true` (which the caller derives from #83's existing
colour gate). These two functions are added to the existing `src/style.rs`.

**Not responsible for:** Deciding which outcome a given rendered line represents
(each call-site already knows its severity/outcome and chooses it), deciding
whether the destination stream may receive colour (that is #83's `should_colorize`
gate, reused, not re-implemented here), emitting any colour on the hook path or any
glyph into `--json` output, or touching the audit-chain `✓`/`✗` integrity markers
(different semantic — left as-is).

**Inputs:**
- For `should_use_unicode`: no caller-passed arguments. It reads the process
  environment — the override variables `ARAI_ASCII` and `NO_UNICODE`, and the
  locale variables `LC_ALL`, `LC_CTYPE`, `LANG`. Required reads; see
  **UnicodeDecision** for the precedence.
- For `outcome_glyph`: the **Outcome** to render (required); a `unicode` boolean
  (required — whether to use the Unicode or the ASCII column of the glyph table,
  normally the result of `should_use_unicode`); a `colorize` boolean (required —
  whether the blocked glyph's `✕` may be wrapped in ochre, normally the verdict of
  #83's `should_colorize` for the destination stream, but **always `false` on the
  hook path**). See **GlyphArgs**.

**Outputs:**
- For `should_use_unicode`: a single boolean — render glyphs as Unicode (true) or
  ASCII (false). See **UnicodeDecision**.
- For `outcome_glyph`: a string — the gateway glyph for the given outcome, drawn
  from the Unicode or ASCII column per `unicode`, with the blocked glyph's cross
  wrapped in #83's ochre/error treatment **only** when `colorize` is true. For
  every non-blocked outcome, and for the blocked outcome when `colorize` is false,
  the returned string contains only the bare glyph characters with no colour bytes.
  See **OutcomeGlyph**.

**Side effects:**
- None. Both functions read the environment (and, transitively through the reused
  gate, the stream's terminal status) and return values; they perform no writes.
  The caller owns the actual print. Reading environment variables is an
  observation, not an effect.

**Error semantics:**
- Both functions are total: no failure modes, no errors signalled, a value returned
  for every input. The project's `Result<T, String>` convention does not apply
  here. A missing or unparseable locale is treated as "not clearly UTF-8" →
  ASCII (the safe, conservative direction), never as an error. An undeterminable
  terminal status (inside the reused gate) is already treated by #83 as
  not-a-terminal → colour-off; this slice inherits that behaviour unchanged.

**Behavioural guarantees:**
- **Unicode decision precedence (fixed order), TTY-independent.** `should_use_unicode`
  returns **false (ASCII)** if `ARAI_ASCII` is present in the environment, OR
  `NO_UNICODE` is present, OR none of `LC_ALL` / `LC_CTYPE` / `LANG` looks UTF-8.
  It returns **true (Unicode)** only when no override is set AND at least one of
  those locale variables, read in that priority order, contains `utf-8` or `utf8`
  (case-insensitive). The decision **does not consult terminal status** — glyphs
  are plain characters and are safe when piped, so the unicode-vs-ascii choice is
  independent of whether the stream is a TTY. [AC2]
- **ASCII output is 7-bit clean.** When `unicode` is false, every glyph string
  `outcome_glyph` returns contains only bytes `<= 0x7F` — including for the blocked
  outcome, whose ASCII form is `o.|x` and whose cross is never coloured in ASCII
  mode. [AC2]
- **Outcome→glyph mapping is fixed and total.** Block → **blocked**, Warn → **warned**,
  Inform → **warned**, Allow → **allowed**. Warn and Inform deliberately collapse to
  the same warned glyph (dot adjacent, pre-passage). The glyph characters are
  exactly the user-approved table in **OutcomeGlyph**; no other glyphs exist and no
  outcome is unmapped. [AC1, AC3]
- **Glyph semantics match the gateway mark.** blocked = dot **outside** the gateway
  plus a cross (`●·│✕` / `o.|x`); allowed = dot **centered**, passing through
  (`│●│` / `|o|`); warned/informed = dot **adjacent**, pre-passage (`●·│` / `o.|`).
  [AC3]
- **Ochre cross only when colorize.** The ochre treatment (#83's error/ochre helper)
  is applied to the `✕` **if and only if** `colorize` is true **and** the outcome is
  blocked **and** `unicode` is true. For every other combination the cross/character
  is bare. The ochre wrapping reuses #83's helper verbatim — this slice adds no
  colour code of its own. [AC8]
- **Plain except for the one optional ochre cross.** The only byte sequence this
  module can ever add beyond the literal glyph characters is the single ochre
  foreground escape (and its reset) around the blocked cross, and only under the
  condition above. No other glyph or outcome ever carries colour; no background is
  ever set.
- **Idempotent / pure.** `should_use_unicode` returns the same value for the same
  environment; `outcome_glyph` is a pure function of `(outcome, unicode, colorize)`
  given #83's gate behaviour. Calling either repeatedly has no observable effect.

**Dependencies:** The existing `src/style.rs` facilities from #83 — the
`should_colorize` gate (used by *call-sites* to derive the `colorize` argument,
except the hook path which hard-codes `colorize=false`) and the ochre/error helper
(used internally to wrap the blocked cross when `colorize` is true). No other module.
No new crate: locale and override detection use the platform's standard environment
access already available in the codebase.

## Integration surfaces (mechanical call-site changes — not modules)

These are the existing files the brief names, where bare or generic-icon strings
are replaced by the `outcome_glyph` return value. None contains new decision logic;
each merely identifies which outcome an existing line represents and passes the
matching `colorize` argument. They are listed for completeness, not as modules.

- **command surface (`src/main.rs`)** — `arai audit` per-firing render and
  `arai why` matched-rule render (the **human, non-json** paths only): prefix each
  outcome row with `outcome_glyph(outcome, should_use_unicode(), colorize)`, where
  `colorize` is #83's `should_colorize` verdict for the destination stream. The
  `--json` branches of both commands are left untouched (no glyph). [AC4, AC7]
- **stats rendering (`src/stats.rs`)** — replace the generic `⚠` (around
  src/stats.rs ~537) with the **warned** glyph from `outcome_glyph`, ASCII-fallback
  aware, with `colorize` from #83's gate for the stats output stream. The `--json`
  path is left untouched (no glyph). [AC5, AC7]
- **hook surface (`src/hooks.rs`)** — the live Pre/Post human-facing strings: the
  deny `reason` and the human-readable `additionalContext` line. Prefix these with
  `outcome_glyph(outcome, should_use_unicode(), false)` — **`colorize` is hard-coded
  `false` on this path, always**, so the glyph characters appear but **no ANSI
  colour is ever added** (the cross is the bare glyph here). This preserves #83's
  standing hook carve-out: glyphs are plain JSON-safe characters and so may appear,
  but the hook path never receives colour. [AC6, AC8]

### Carve-outs (named, first-class scope boundaries)

- **Hook path always passes `colorize=false`.** [AC6, AC8] The `src/hooks.rs`
  integration calls `outcome_glyph` with `colorize=false` unconditionally — it does
  **not** consult `should_colorize`, so the guarantee does not depend on the gate
  happening to return colour-off in the hook context. Glyph characters are emitted;
  ANSI colour is not. The subprocess test asserts this directly (glyph present in
  hook output, zero ANSI bytes).
- **`--json` carries no glyphs.** [AC7] No `--json` field value in any command is
  routed through `outcome_glyph`. Machine consumers continue to read the structured
  severity field; glyphs are a human-presentation concern only. Asserted by the
  subprocess test (zero glyph characters in `--json` output).
- **Audit-chain `✓`/`✗` integrity markers stay as-is.** The `✓`/`✗` markers in the
  `arai audit --verify` hash-chain output (src/main.rs ~847-850) are a *verification*
  semantic (chain valid / broken), **not** a gateway *passage* semantic. They are
  **not** touched by this slice and are **not** routed through `outcome_glyph`.

## Data shapes

### Outcome
The closed set of tool-call outcomes the glyph function maps over, already modelled
in the codebase as `intent::Severity` plus the Allow case:
- **Block** — the call was denied.
- **Warn** — the call fired a warn-severity rule.
- **Inform** — the call fired an inform-severity rule.
- **Allow** — the call passed with no blocking/warning outcome.
The mapping collapses Warn and Inform onto the same warned glyph.

### OutcomeGlyph (the binding glyph table)
The user-approved gateway glyph set. Each outcome has a Unicode form and an ASCII
fallback; the cross on the blocked form is the only character that may be coloured:

| outcome  | source severity | Unicode | ASCII   | semantics                              |
|----------|-----------------|---------|---------|----------------------------------------|
| blocked  | Block           | `●·│✕`  | `o.|x`  | dot **outside** the gateway + ochre cross |
| allowed  | Allow           | `│●│`   | `|o|`   | dot **centered**, passing through         |
| warned   | Warn or Inform  | `●·│`   | `o.|`   | dot **adjacent**, pre-passage             |

The `✕` (Unicode blocked form only) is wrapped in #83's ochre/error treatment iff
`colorize` is true; in all other cases every character above is rendered bare. ASCII
forms are 7-bit clean and never coloured.

### UnicodeDecision
The boolean produced by `should_use_unicode`, with fixed-precedence inputs:
- `ARAI_ASCII` present in the environment → **ASCII (false)**.
- else `NO_UNICODE` present → **ASCII (false)**.
- else none of `LC_ALL` / `LC_CTYPE` / `LANG` (read in that order) contains `utf-8`
  or `utf8` (case-insensitive) → **ASCII (false)**.
- else → **Unicode (true)**.
TTY status is **not** an input — glyphs are safe when piped, so the decision is
independent of whether the stream is a terminal.

### GlyphArgs
The argument triple `outcome_glyph` receives: the **Outcome**; a `unicode` boolean
(normally the **UnicodeDecision**, selecting the table column); and a `colorize`
boolean (normally #83's `should_colorize` verdict for the destination stream, but
**hard-coded `false` on the hook path**). The function branches on these three only;
it reads no other state directly (the colour gate's own inputs are read by the gate
when the call-site computes `colorize`, not by `outcome_glyph`).

## Acceptance criteria

Single module, so all criteria are owned by `outcome-glyphs` and its integration
surfaces; carried verbatim from brief v8.

- **AC1:** an `outcome_glyph` function maps Block / Warn|Inform / Allow to the
  gateway glyph set above (Unicode + ASCII).
- **AC2:** Unicode by default; ASCII fallback when ARAI_ASCII set OR locale
  non-UTF-8; ARAI_ASCII=1 glyph output contains only bytes <= 0x7F.
- **AC3:** glyph semantics match spec — blocked=dot-outside+cross,
  allowed=dot-centered, warned/informed=dot-adjacent.
- **AC4:** `arai audit` and `arai why` human (non-json) output show the per-outcome
  glyph.
- **AC5:** the generic `⚠` in stats.rs is replaced by the warned glyph.
- **AC6:** the live hook Pre/Post surface carries the glyph (ASCII-fallback-aware)
  with NO ANSI colour added to hook output (the cross is the bare glyph there).
- **AC7:** every `--json` output is unchanged (no glyphs in json field values).
- **AC8:** ochre colour on the `✕` appears only in human TTY output and obeys the
  #83 gate (absent under NO_COLOR / non-TTY / hook path).
- **AC9:** outcomes distinguishable at a glance and read as Arai-native (manual —
  the dispatcher will eyeball the rendered set + ASCII fallback, as it did the WCAG
  check on #83).
- **AC10:** full gate — cargo fmt --all -- --check + cargo clippy --all-targets (no
  new warnings) + cargo test; ZERO new dependency.

### Where each AC lands

| AC   | Lands on |
|------|----------|
| AC1  | `outcome_glyph` existence in `src/style.rs`: the fixed outcome→glyph table lives here and nowhere else. |
| AC2  | `should_use_unicode` precedence (override or non-UTF-8 locale → ASCII) + the 7-bit-clean ASCII guarantee; TTY-independent. |
| AC3  | The **OutcomeGlyph** table's character forms and semantics. |
| AC4  | `src/main.rs` audit + why human-render integration prefixes each row with the glyph; `--json` untouched. |
| AC5  | `src/stats.rs` integration swaps the generic `⚠` for the warned glyph. |
| AC6  | `src/hooks.rs` integration passes `colorize=false` always — glyph chars present, zero ANSI. |
| AC7  | --json exclusion carve-out: no `--json` field routed through `outcome_glyph`. |
| AC8  | Ochre-cross-only-when-colorize guarantee, reusing #83's `should_colorize` gate + ochre helper; hook path forces colorize=false. |
| AC9  | Manual eyeball of the rendered set + ASCII fallback (no automated visual assertion possible). |
| AC10 | Full-gate process note below — leaf + verifier run fmt + clippy + test. |

## Testing

### Unit tests (co-located in `src/style.rs`)
Co-located `#[cfg(test)]` tests exercising the mapping and both decision axes:
- **Mapping:** each Outcome maps to the correct glyph string — Block→blocked,
  Warn→warned, Inform→warned, Allow→allowed — in both the Unicode and the ASCII
  column. [AC1, AC3]
- **Unicode vs ASCII:** `should_use_unicode` returns ASCII when `ARAI_ASCII` is set,
  when `NO_UNICODE` is set, and when the locale variables are absent or non-UTF-8;
  returns Unicode when a locale variable contains `utf-8`/`utf8` and no override is
  set; and is independent of terminal status. Every ASCII-mode glyph string is
  verified to contain only bytes `<= 0x7F`. [AC2]
- **Ochre only when colorize:** the blocked Unicode glyph contains the ochre escape
  around the `✕` **only** when `colorize=true`; with `colorize=false` (the hook
  case) the blocked glyph is the bare `●·│✕` with no colour bytes; no non-blocked
  outcome ever carries colour; ASCII-mode blocked never carries colour. [AC8]

### Subprocess integration test (under `tests/`, repo subprocess pattern)
A cross-module test driving the built binary through the project's standard
subprocess pattern — invoking the binary via the build-provided binary-path
mechanism (`env!("CARGO_BIN_EXE_arai")`), isolating state with an `ARAI_BASE_DIR`
temporary directory, and using **no new dependency** (temp isolation follows the
existing tests' approach). It asserts:
- the per-outcome glyph is **present** in the human (non-json) `arai audit` and
  `arai why` output; [AC4]
- with `ARAI_ASCII=1` set, the human glyph output contains **only** bytes `<= 0x7F`
  (no Unicode glyph forms); [AC2]
- the live hook Pre/Post output (e.g. `guardrails --match-stdin`) **contains the
  glyph characters** but **zero ANSI escape bytes**; [AC6, AC8]
- every `--json` rendering contains **no glyph characters** (Unicode or ASCII forms)
  in its field values. [AC7]

The presence-of-ochre direction (a colorized blocked cross carries the ochre escape)
is covered by the unit tests, because the subprocess test's sink is a captured pipe
(non-terminal) where #83's gate returns colour-off.

### Full-gate process note (carried from prior cycles) [AC10]
The leaf implementer and the verifier MUST each run the project's **full** local
gate before declaring work complete — not `cargo test` alone:
`cargo fmt --all -- --check` (formatting clean), `cargo clippy --all-targets` (no
new warnings), AND `cargo test` (all tests, including the new unit matrix and the
new subprocess integration test, pass). CI gates rustfmt and clippy; running only
tests locally is the failure this note exists to prevent.

## Files touched

| File              | Change                                                                                                   |
|-------------------|----------------------------------------------------------------------------------------------------------|
| `src/style.rs`    | **Extended.** Add `should_use_unicode` (override + UTF-8-locale precedence, TTY-independent) and `outcome_glyph(outcome, unicode, colorize)` over the fixed **OutcomeGlyph** table; reuse #83's `should_colorize` (via call-sites) and the ochre/error helper (for the cross). Co-located unit tests (mapping, unicode vs ascii, ochre-only-when-colorize). |
| `src/main.rs`     | Route `arai audit` per-firing + `arai why` matched-rule **human** renders through `outcome_glyph`, `colorize` from #83's gate. `--json` branches untouched. |
| `src/stats.rs`    | Replace the generic `⚠` (~537) with the **warned** glyph from `outcome_glyph`, ASCII-fallback aware. `--json` untouched. |
| `src/hooks.rs`    | Prefix the live Pre/Post human strings (deny `reason` / `additionalContext` line) with `outcome_glyph(outcome, should_use_unicode(), false)` — **colorize=false always**, glyph chars only, no ANSI. |
| `tests/`          | New subprocess integration test (build-provided binary path, `ARAI_BASE_DIR` temp isolation, no new dependency): glyph present in human audit/why; `ARAI_ASCII=1` → ASCII-only; hook output has the glyph but zero ANSI; `--json` has no glyphs. |

## Out of scope

- **Brand-mark glyph designs (#65-67)** — the gateway *mark* artwork is separate;
  this slice only derives outcome glyphs from it.
- **Copy-tone audit (#85)** — wording is not touched.
- **Glyphs in any `--json` output** — machine consumers use the structured severity
  field; `--json` stays byte-equivalent to today with respect to glyphs.
- **ANSI colour on the hook path** — `src/hooks.rs` always passes `colorize=false`;
  glyph characters only, never colour. Hard boundary, preserving #83's carve-out.
- **The audit-chain `✓`/`✗` integrity markers** — verification semantic, not a
  gateway passage; left exactly as-is, never routed through `outcome_glyph`.
- **Any new dependency** — no glyph crate, no locale crate, no temp-dir crate;
  override/locale detection and temp isolation reuse existing facilities.
- **Theming / configurable or custom glyph sets** — the glyph table is fixed and
  user-approved; there is no per-user override of the characters themselves.
- **Re-deriving or extending the #83 colour palette** — this slice reuses #83's
  `should_colorize` gate and ochre helper unchanged and adds no new colours.
