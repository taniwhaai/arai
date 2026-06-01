---
version: 7
parent_brief_version: 9
tier: single_module
---

# copy-tone-audit (restrained-declarative voice retune)

## Structural tier

**Selected:** single_module

**Justification:** The brief describes exactly one capability: an editorial retune of
Arai's user-facing strings from a friendly-devtool voice to a restrained-declarative
register, governed by a single committed voice spec and a single self-reference
glossary. It is explicitly NOT new logic — AC8 makes "behaviour UNCHANGED — wording
only" a hard constraint, so there is no new module, no new data shape, no new control
flow, and no composition to wire. Every acceptance criterion (AC1–AC10) is either the
authoring of the one spec artefact (AC1), a property of how an existing string reads
once retuned (AC2–AC7), or a process guarantee about keeping behaviour and tests in
lockstep (AC8–AC10). The files named in scope (`src/hooks.rs`, `src/init.rs`,
`src/main.rs`, `src/stats.rs`, assorted `Result<_,String>`/`eprintln!` sites, and a
light `README.md` pass) are **edit surfaces of one coherent voice concern**, not
independent capabilities: each merely substitutes restrained-declarative wording for
the current wording at sites that already exist, with no new state and no decision of
its own. Promoting any file to a "module" would be layer-slicing against a single
copy/voice capability — the exact failure the tier rule exists to prevent. Total scope
is a bounded set of string edits plus one short in-repo Markdown artefact (`docs/voice.md`)
plus the lockstep test updates — well under a few hundred lines of diff. No subsystems,
no differing failure semantics, no independently-swappable parts. Neither
`small_multi_module` nor `full_decomposition` is warranted; the brief's own dispatcher
note ("a coherent copy/voice concern; do NOT use full_decomposition") concurs.

**Module count:** 1 (`copy-tone-audit`). No composition layer. One in-repo artefact
(`docs/voice.md`) is authored as part of the module's deliverable, not as a separate
module. The named source files are **edit surfaces** of the one module, enumerated under
**Edit surfaces** below, not modules in their own right.

## Open questions

None. The brief (v9) is detailed and the register was user-approved
("restrained declarative"). The voice spec rules, the self-reference glossary, the
bounded scope, the hard constraints, and the lockstep-test requirement are all stated.
This design commits the spec to a concrete, checkable form (below) so the subjective
ACs (AC2/AC5/AC6) reduce to "does this string obey these committed rules and use these
committed terms?" — a question the verifier and dispatcher can answer against
`docs/voice.md`. No load-bearing gap remains that would change the module structure,
the deliverables, or the behavioural guarantees.

## Purpose

Move Arai's user-facing strings from a friendly-devtool voice ("Arai is now watching
your rules", "Done.", bare "Failed to X") to a restrained, infrastructural register —
declarative, calm, specific, and adequately detailed at the moment a person is about to
lose work to a deny — so that Arai reads less like a chatty dev tool and more like a
piece of infrastructure, while every machine-consumed surface (JSON protocol keys/values,
the #83 colour treatment, and the #84 outcome glyphs) and every observable behaviour stay
exactly as they are today.

## External boundaries

These are the surfaces where the retuned **words** appear. None of their **structure**
changes — only the human-readable string content does.

- **standard output / standard error (human command paths)**: outbound, text — the
  prose lines of `arai init` / `deinit`, `arai status`, `arai why`, `arai audit`,
  `arai stats`, and the other command headers / "no rules" / hint lines. The colour
  (#83) and glyph (#84) bytes wrapping these lines are preserved unchanged; only the
  literal words inside them change.
- **hook-protocol surface (`PreToolUse` / `PostToolUse` / `UserPromptSubmit`)**:
  outbound, text inside JSON — the human-readable string *values* of the deny `reason` /
  `permissionDecisionReason`, the `additionalContext` prose, and the internal-error
  fail-closed reason. The JSON **keys and the structural/enum values** (`"decision"`,
  `"deny"`, `"allow"`, `"permissionDecision"`, `hookEventName`, etc.) are preserved
  exactly; only the human-readable string *content* is retuned. The #84 glyph prefix on
  these strings is preserved.
- **filesystem (read-only, additive)**: outbound — one new committed file,
  `docs/voice.md`, carrying the voice spec and the self-reference glossary (AC1). This is
  the only new file; it adds documentation, not behaviour.
- **README**: outbound, text — a light pass on the intro/tagline voice only (AC7). Deeper
  doc work is explicitly out of scope and flagged as follow-on.

## Modules

### copy-tone-audit

**Responsible for:** (1) Authoring `docs/voice.md` — a short voice spec (the
restrained-declarative register rules) plus the self-reference glossary — concrete enough
that the subjective acceptance criteria become checkable. (2) Retuning the user-facing
string *content* at the bounded edit surfaces below so each string obeys that spec and
uses the glossary's terms consistently. (3) Updating, in the same change, every test that
asserts a human string this retune alters, so no assertion is left on old wording.

**Not responsible for:** Changing any behaviour, control flow, or logic (AC8 — wording
only); changing JSON protocol keys or structural/enum values; changing the #83 colour
treatment or the #84 glyph behaviour (both preserved verbatim); rewording internal-only
strings that never reach a user (those stay accurate and untouched); telemetry/log
internals; a deep README or documentation rewrite; or adding any dependency.

**Inputs:**
- The current user-facing strings at the named edit surfaces (the literal text to be
  reworded).
- The committed voice spec + glossary in `docs/voice.md` (authored as part of this
  module) as the rule set every reworded string must satisfy.

**Outputs:**
- `docs/voice.md` — the committed spec + glossary (the **VoiceSpec** and
  **SelfReferenceGlossary** shapes below). [AC1]
- Retuned string content at each edit surface, each obeying the spec and glossary.
  [AC2–AC7]
- Updated test assertions wherever a reworded string was asserted. [AC9]

**Side effects:**
- Writes one new file (`docs/voice.md`) and edits existing source and test files. No
  runtime side effects — the program's observable behaviour is unchanged.

**Error semantics:**
- N/A for the retune itself (it is a content edit, not new fallible logic). The
  `Result<_,String>` and `eprintln!` *messages* on user-facing paths are reworded to the
  specific-declarative form, but the error-*handling control flow* (which branch returns
  an error, what is returned, fail-closed-on-PreToolUse behaviour) is preserved exactly.
  An internal-only error string that never reaches a user is left untouched and accurate.

**Behavioural guarantees:**
- **Behaviour unchanged.** Every change is to human-readable string content only. No
  branch condition, no return value other than message text, no exit code, no hook
  decision, no JSON shape changes. `cargo test` (behavioural assertions) stays green
  *after* the lockstep test updates. [AC8]
- **JSON protocol structure preserved.** Keys and structural/enum values in every hook
  response are byte-identical to today; only the human-readable string *content* of
  `reason` / `permissionDecisionReason` / `additionalContext` changes. [AC8]
- **#83 colour + #84 glyph behaviour preserved.** No colour code, no glyph code, no
  colour/glyph gate is touched. Where a retuned line is wrapped by #83's colour or
  prefixed by #84's outcome glyph, that wrapping/prefix is preserved; only the words
  inside change. [AC8]
- **Lockstep tests.** Every test that asserts a human string altered by this retune is
  updated in the same change. After the retune, no test still asserts old wording; the
  full gate is green. [AC9, AC10]
- **Zero new dependency.** No crate is added. [AC10]
- **Glossary consistency.** Across all retuned strings, each concept is named by exactly
  one glossary term (Arai / rule(s) + guardrails / the model / you), held consistently.
  [AC6]

**Dependencies:** None (no other module). The retune *reads alongside* the #83 colour
helpers (`src/style.rs`) and #84 glyph helpers but **calls and changes neither** — it
preserves their output wrapping while editing the words inside.

## The committed voice spec (to be written verbatim into `docs/voice.md`)

This is the substance of the deliverable. The spec below is concrete enough that AC2,
AC5, and AC6 reduce to mechanical checks against it.

### VoiceSpec — restrained-declarative register rules

1. **Declarative, calm sentences.** State what is or what happened. No exclamation
   marks, no "Done!"/"Done.", no cute or marketing flourish, no cheerleading.
2. **Specific over template.** An error names the thing and the consequence, not a bare
   "Failed to X". Form: `Could not <verb> <named thing>: {e}` (or an equally specific
   declarative). The underlying `{e}` detail is retained.
3. **Adequate at the deny.** A deny quotes the rule and its `source:line` — enough for
   the reader to act — with no lecture and no padding. The existing rule/source/line
   payload is kept; only the framing words change.
4. **No anthropomorphism.** Arai does not "watch your rules", "keep an eye on", "want",
   or "think". It enforces, records, blocks, allows. ("watching your rules" →
   "enforcing this project's rules" / "watching this project".)
5. **Sentence case; periods on full sentences.** Headers and labels use sentence case
   (not Title Case, not ALL CAPS). Full sentences end with a period. Fragments/labels
   (e.g. a status field) need not.
6. **Preserve #83 colour and #84 glyph behaviour.** This slice changes WORDS, not the
   colour bytes (#83) or the outcome-glyph prefix (#84). Where a line is coloured or
   glyph-prefixed today, it stays coloured/prefixed; only the words change.

### SelfReferenceGlossary — one term per concept, held consistently

| Concept | Committed term | Notes |
|---|---|---|
| the tool itself | **Arai** | Proper noun. Never "we", never "I". |
| a single constraint | **rule** (plural **rules**) | The unit a user authors / that fires. |
| the system / the command surface | **guardrails** | The `arai guardrails` command keeps its name; "guardrails" is the collective system, "rule" is the unit. |
| the AI actor | **the model** | Not "the agent", "the assistant", "Claude". |
| the human reader | **you** | Second person. Not "the user" in user-facing prose. |

The dispatcher/verifier may refine exact term spellings during review, but the spec MUST
commit to one term per concept and hold it consistently across every retuned string.

### Concrete before/after exemplars (grounding — these illustrate the register)

The examples below are drawn from current strings at the named surfaces, retuned to the
spec. They are illustrative of the target register; the leaf produces the final wording
and the verifier checks it against the committed spec.

| Surface | Current (devtool voice) | Retuned (restrained-declarative) | Rule(s) |
|---|---|---|---|
| `src/init.rs` init footer | `Done. Arai is now watching your rules (Claude + Grok TUI).` | `Arai is enforcing this project's rules (Claude Code and Grok TUI).` | 1, 4, 5 |
| `src/init.rs` deinit footer | `Arai is no longer watching this project.` | `Arai is no longer enforcing this project's rules.` | 4 |
| `src/init.rs` error | `Failed to read settings.json: {e}` | `Could not read settings.json: {e}` | 2 |
| `src/init.rs` error | `Failed to write settings.json: {e}` | `Could not write settings.json: {e}` | 2 |
| `src/hooks.rs` deny fallback | `Arai: blocking rule matched` | `Arai: a rule blocked this action.` | 1, 3, 5 |
| `src/hooks.rs` deny reason | `Arai: "<subj> <pred> <obj>" [from <src>:<line>]` | keep the quoted rule + `source:line` (adequate at the deny); align framing to sentence case / glossary only | 3, 5, 6 |
| `src/hooks.rs` internal error | `Arai: internal error, blocking for safety` | `Arai: an internal error occurred; blocking this action.` | 1, 5 |
| `src/main.rs` status / no-rules | `No guardrails found. Run \`arai init\` first.` | sentence-case declarative, glossary-aligned ("No rules found. Run \`arai init\` first.") | 5, 6 |
| `src/main.rs` hint | `Try phrasing it as an imperative (e.g. "Never force-push to main")` | calm declarative phrasing, no exhortation flourish | 1 |
| `README.md` tagline | friendly-devtool intro line | restrained, infrastructural one-line statement of what Arai is | 1, 4 |

(The glyph prefix #84 adds to the hook deny `reason`/`additionalContext` and the colour
#83 adds to status headers are **kept** in all of the above — only the words inside
change.)

## Edit surfaces (bounded — the named, user-visible sites; not modules)

These are the existing sites the brief names where current wording is replaced by
restrained-declarative wording. None contains new logic; each is a content edit.

- **`src/hooks.rs`** — the rendered human messages: the deny `reason` /
  `permissionDecisionReason` (incl. the `deny_reason` builder and its
  `"Arai: blocking rule matched"` fallback), the `additionalContext` prose (incl.
  `guardrails::format_context`'s `"Arai guardrails:"` header and per-rule lines, and the
  `"[Post-action review] …"` prefix), and the internal-error fail-closed text
  (`"Arai: internal error, blocking for safety"`). JSON keys/enum values untouched; #84
  glyph prefix preserved. [AC2]
- **`src/init.rs`** — the init / deinit flow strings: the `"Done. Arai is now watching
  your rules …"` footer, the `"Arai is no longer watching this project."` footer, step
  headers ("Scanning for instruction files…", "Extracting guardrails…", "Setting up
  hooks…"), and the user-facing `Failed to …` error messages on these paths. [AC3]
- **user-visible error messages across `src/`** — the `Result<_,String>` messages and
  `eprintln!` strings that **actually reach a user** (the init/deinit/`store` open paths,
  `main.rs` file-read paths, `upgrade.rs`/`extends.rs` user-driven commands, the
  `arai hook error: {e}` diagnostic). Retuned from bare `Failed to X` to specific
  declarative form. **Internal-only strings that are never shown to a user are NOT
  touched** (kept accurate). [AC4]
- **`src/main.rs` + `src/stats.rs` command-output prose** — `status` / `why` / `audit` /
  `stats` headers, the "No rules"/"No audit entries"/"No severity overrides" lines, and
  the hint lines ("Run \`arai init\` first.", "Try phrasing list items as imperatives…",
  "Run \`arai scan\` after saving…"). Sentence case, glossary-aligned, no flourish. #83
  colour on headers preserved. [AC5]
- **`README.md`** — a **light** pass on the intro/tagline voice only. Deeper doc work is
  out of scope and flagged as follow-on. [AC7]

### Carve-outs (named scope boundaries)

- **JSON keys and structural/enum values are never touched.** Only the human-readable
  string *content* of `reason` / `permissionDecisionReason` / `additionalContext`
  changes. `"decision"`, `"deny"`, `"allow"`, `"permissionDecision"`, `hookEventName`,
  and every other key/enum stay byte-identical. [AC8]
- **#83 colour and #84 glyph behaviour are never touched.** No colour or glyph code,
  table, or gate is edited. Retuned lines keep their colour wrapping and glyph prefix.
  [AC8]
- **Internal/never-shown strings stay as-is.** A `Result<_,String>` or `eprintln!` that
  is only ever consumed by code or by a developer reading source (never surfaced to a
  user) is left accurate and unchanged. Only user-reaching strings are retuned. [AC4]
- **No deep README/doc rewrite.** The README pass is intro/tagline voice only; broader
  doc/scope-honesty work is a separate follow-on. [AC7]

## Data shapes

### VoiceSpec
The committed register rule set (six rules above), written into `docs/voice.md`. The
governing artefact against which AC2/AC5/AC6 are checked.

### SelfReferenceGlossary
The committed one-term-per-concept table above (Arai / rule(s) + guardrails / the model /
you), written into `docs/voice.md`. The consistency backbone for AC6.

### RetunedString (conceptual)
A user-facing string after retune. Property, not a type: same *position* and *role* in the
program as before (same branch, same JSON key, same colour/glyph wrapping), different
human-readable *words*, satisfying VoiceSpec and using SelfReferenceGlossary terms.

## Acceptance criteria (carried verbatim from brief v9)

- **AC1:** a short voice spec + self-reference glossary committed in-repo (e.g.
  `docs/voice.md`) so the register stays maintainable.
- **AC2:** hooks.rs rendered human messages match the restrained-declarative register.
- **AC3:** init.rs / deinit flow strings retuned.
- **AC4:** user-visible error messages retuned to specific declarative form (no bare
  "Failed to X" on user-facing paths).
- **AC5:** command-output prose (status/why/audit/stats headers, "no rules"/hint lines)
  retuned.
- **AC6:** consistent self-reference per the glossary across all retuned strings.
- **AC7:** README intro/tagline voice aligned (light pass); deeper doc work flagged
  follow-on.
- **AC8:** behaviour UNCHANGED — wording only; no logic change; JSON protocol keys/values
  structure, #83 colour, and #84 glyph behaviour preserved.
- **AC9:** every test asserting a changed string is updated in lockstep; cargo test green
  (no assertion left on old wording).
- **AC10:** full gate — cargo fmt --all -- --check + cargo clippy --all-targets (no new
  warnings) + cargo test; ZERO new dependency.

### Where each AC lands + what the verifier checks

| AC | Surface | What the verifier checks (against committed `docs/voice.md`) |
|---|---|---|
| AC1 | `docs/voice.md` (new) | The file exists, is committed, and contains the VoiceSpec rules + SelfReferenceGlossary in a form concrete enough to check AC2/AC5/AC6 against. |
| AC2 | `src/hooks.rs` | The deny `reason`, `additionalContext` prose, and internal-error text read as calm declarative, specific, adequate-at-the-deny, no anthropomorphism, sentence case — and obey rules 1–6. JSON keys/enums and the #84 glyph prefix unchanged. |
| AC3 | `src/init.rs` | Init/deinit footers and step headers retuned; no "Done!"/"watching your rules"; sentence case. |
| AC4 | `Result<_,String>` + `eprintln!` across `src/` (user-facing only) | No bare "Failed to X" on a user-reaching path; each is specific-declarative (`Could not <verb> <thing>: {e}`). Internal-only strings confirmed untouched and accurate. |
| AC5 | `src/main.rs` + `src/stats.rs` | status/why/audit/stats headers, "no rules"/hint lines are sentence case, declarative, no flourish; #83 colour on headers preserved. |
| AC6 | all retuned strings | Each concept uses exactly one glossary term, held consistently (Arai / rule(s)+guardrails / the model / you). |
| AC7 | `README.md` | Intro/tagline reads in the restrained register; the pass is light; deeper doc work is flagged as follow-on, not done here. |
| AC8 | all surfaces | Diff is wording-only: no branch/return/exit-code/JSON-shape change; `git diff` shows no edits to colour (#83) or glyph (#84) code; JSON keys/enums byte-identical. |
| AC9 | `tests/hooks_safety.rs`, brand/glyph verifier test files, integration tests | After the retune, `grep` finds **no** test still asserting an old wording that was changed; every such assertion was updated in the same change; `cargo test` is green. |
| AC10 | full gate | `cargo fmt --all -- --check` clean, `cargo clippy --all-targets` no new warnings, `cargo test` green, and `Cargo.toml`/`Cargo.lock` show ZERO new dependency. |

## Hard constraints (carried into contract)

1. **Behaviour UNCHANGED — wording only.** No logic, branch, return value (beyond message
   text), exit code, or hook decision changes. [AC8]
2. **JSON protocol structure preserved.** Hook-response keys and structural/enum values
   are byte-identical; only human-readable string content changes. [AC8]
3. **#83 colour + #84 glyph behaviour preserved.** No colour/glyph code, table, or gate is
   edited; retuned lines keep their wrapping and prefix. [AC8]
4. **ZERO new dependency.** No crate added; `Cargo.toml`/`Cargo.lock` unchanged but for
   nothing. [AC10]
5. **Lockstep tests (CRITICAL).** Many tests assert exact human strings
   (`tests/hooks_safety.rs`, the brand/glyph verifier files, integration tests). Every
   test asserting a string this retune changes MUST be updated **in the same change**.
   The leaf greps for every changed string and updates the asserting tests; no assertion
   is left on old wording. [AC9]
6. **User-facing only.** Internal/never-shown strings stay accurate and untouched; only
   strings that actually reach a user are retuned. [AC4]

## Full-gate process note [AC10]

The leaf implementer and the verifier MUST each run the project's **full** local gate
before declaring work complete — not `cargo test` alone:
`cargo fmt --all -- --check` (formatting clean), `cargo clippy --all-targets` (no new
warnings), AND `cargo test` (all tests, including the lockstep-updated string assertions,
pass). CI gates rustfmt and clippy; running only tests locally is the failure this note
exists to prevent. The verifier additionally **reviews voice consistency against the
committed `docs/voice.md`** (does each retuned string obey the VoiceSpec rules and use the
SelfReferenceGlossary terms?) and confirms via `grep` that **no test still asserts old
wording**; the dispatcher independently eyeballs the real retuned output before commit.

## Out of scope

- **JSON protocol keys/values structure** — keys and structural/enum values stay
  byte-identical; only human-readable string content changes.
- **#83 colour and #84 glyph behaviour** — no colour/glyph code, table, or gate is
  edited; retuned lines keep their wrapping/prefix.
- **Internal / never-shown error strings** — kept accurate, untouched; only user-reaching
  strings are retuned.
- **Telemetry / log internals** — not user-facing prose; untouched.
- **Deep README / documentation rewrite** — the README pass is intro/tagline voice only;
  broader doc/scope-honesty work is a flagged follow-on.
- **Any new dependency** — no crate is added.
- **Any logic / behaviour change** — this is an editorial retune; observable behaviour is
  identical before and after.
