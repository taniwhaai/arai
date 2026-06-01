# Manifest: copy-tone-audit

---
derived_from: design_v7.md
tier: single_module
vocabulary_file: none (single_module — types defined inline)
composition: none
---

## Responsibility

Retune the human-readable string content at a bounded set of named edit surfaces from a friendly-devtool voice to a restrained-declarative register, governed by the committed VoiceSpec and SelfReferenceGlossary, while leaving all behaviour, JSON structure, colour treatment, glyph behaviour, and untouched internal strings identical.

## Not responsible for

Changing any logic, control flow, return value (beyond message text), exit code, hook decision, JSON protocol key or structural/enum value, #83 colour code, #84 glyph code, internal-only strings never shown to a user, telemetry or log internals, a deep README/documentation rewrite, or adding any dependency.

---

## The committed voice spec

> **committed — do not paraphrase**

### VoiceSpec — restrained-declarative register rules

1. **Declarative, calm sentences.** State what is or what happened. No exclamation marks, no "Done!"/"Done.", no cute or marketing flourish, no cheerleading.
2. **Specific over template.** An error names the thing and the consequence, not a bare "Failed to X". Form: `Could not <verb> <named thing>: {e}` (or an equally specific declarative). The underlying `{e}` detail is retained.
3. **Adequate at the deny.** A deny quotes the rule and its `source:line` — enough for the reader to act — with no lecture and no padding. The existing rule/source/line payload is kept; only the framing words change.
4. **No anthropomorphism.** Arai does not "watch your rules", "keep an eye on", "want", or "think". It enforces, records, blocks, allows. ("watching your rules" → "enforcing this project's rules" / "watching this project".)
5. **Sentence case; periods on full sentences.** Headers and labels use sentence case (not Title Case, not ALL CAPS). Full sentences end with a period. Fragments/labels (e.g. a status field) need not.
6. **Preserve #83 colour and #84 glyph behaviour.** This slice changes WORDS, not the colour bytes (#83) or the outcome-glyph prefix (#84). Where a line is coloured or glyph-prefixed today, it stays coloured/prefixed; only the words change.

### SelfReferenceGlossary — one term per concept, held consistently

> **committed — do not paraphrase**

| Concept | Committed term | Notes |
|---|---|---|
| the tool itself | **Arai** | Proper noun. Never "we", never "I". |
| a single constraint | **rule** (plural **rules**) | The unit a user authors / that fires. |
| the system / the command surface | **guardrails** | The `arai guardrails` command keeps its name; "guardrails" is the collective system, "rule" is the unit. |
| the AI actor | **the model** | Not "the agent", "the assistant", "Claude". |
| the human reader | **you** | Second person. Not "the user" in user-facing prose. |

---

## Before/after exemplar table

> **committed — do not paraphrase**
>
> These examples are drawn from current strings at the named surfaces, retuned to the spec. They are illustrative of the target register; the leaf produces the final wording and the verifier checks it against the committed VoiceSpec above.

| Surface | Current (devtool voice) | Retuned (restrained-declarative) | VoiceSpec rule(s) |
|---|---|---|---|
| `src/init.rs` init footer | `Done. Arai is now watching your rules (Claude + Grok TUI).` | `Arai is enforcing this project's rules (Claude Code and Grok TUI).` | 1, 4, 5 |
| `src/init.rs` deinit footer | `Arai is no longer watching this project.` | `Arai is no longer enforcing this project's rules.` | 4 |
| `src/init.rs` error | `Failed to read settings.json: {e}` | `Could not read settings.json: {e}` | 2 |
| `src/init.rs` error | `Failed to write settings.json: {e}` | `Could not write settings.json: {e}` | 2 |
| `src/hooks.rs` deny fallback | `Arai: blocking rule matched` | `Arai: a rule blocked this action.` | 1, 3, 5 |
| `src/hooks.rs` deny reason | `Arai: "<subj> <pred> <obj>" [from <src>:<line>]` | keep the quoted rule + `source:line` (adequate at the deny); align framing to sentence case / glossary only | 3, 5, 6 |
| `src/hooks.rs` internal error | `Arai: internal error, blocking for safety` | `Arai: an internal error occurred; blocking this action.` | 1, 5 |
| `src/main.rs` status / no-rules | `No guardrails found. Run \`arai init\` first.` | `No rules found. Run \`arai init\` first.` | 5, 6 |
| `src/main.rs` hint | `Try phrasing it as an imperative (e.g. "Never force-push to main")` | calm declarative phrasing, no exhortation flourish | 1 |
| `README.md` tagline | friendly-devtool intro line | restrained, infrastructural one-line statement of what Arai is | 1, 4 |

> The glyph prefix #84 adds to the hook deny `reason`/`additionalContext` and the colour #83 adds to status headers are **kept** in all of the above — only the words inside change.

---

## Inputs

- **Current user-facing string content** (text, required): the literal human-readable strings at the named edit surfaces (hooks deny reason, additionalContext, internal-error text; init/deinit footers and step headers; user-visible `Result<_, String>` messages and diagnostic lines across `src/`; command-output prose in `src/main.rs` and `src/stats.rs`; README intro/tagline). These are read from existing source; no external input is required.
- **VoiceSpec** (committed text, required): the six register rules above, authored into `docs/voice.md` as part of this module. Each retuned string must satisfy all applicable rules.
- **SelfReferenceGlossary** (committed table, required): the one-term-per-concept table above, authored into `docs/voice.md` as part of this module. Each concept must use exactly the committed term, held consistently across all retuned strings.

## Outputs

- **`docs/voice.md`** (new committed file): contains the VoiceSpec (six rules) and the SelfReferenceGlossary (five-row table) verbatim as written above. Concrete enough that AC2/AC5/AC6 reduce to mechanical checks against it. [AC1]
- **Retuned string content at each edit surface**: the human-readable words at every named site replaced with restrained-declarative wording satisfying the VoiceSpec and using the SelfReferenceGlossary terms. The before/after exemplar table above is the register's ground truth. [AC2–AC7]
- **Updated test assertions**: every test that asserted a human string altered by this retune is updated in the same change, so no assertion remains on old wording. [AC9]

## Side effects

- Writes one new file (`docs/voice.md`). This is documentation only; no runtime behaviour changes.
- Edits existing source files at the named edit surfaces and the test files that assert those strings. No file is created beyond `docs/voice.md`.
- No runtime side effects — the program's observable behaviour, exit codes, JSON output structure, colour treatment, and glyph behaviour are unchanged before and after.

## Error semantics

- N/A for the retune itself. This is a content edit, not new fallible logic. The `Result<_, String>` and `eprintln!` messages on user-facing paths are reworded, but the error-handling control flow (which branch returns an error, what is returned, the fail-closed PreToolUse behaviour) is preserved exactly and is not this module's concern.
- An internal-only error string that never reaches a user is left untouched and accurate; the implementor must not retune it.

## Behavioural guarantees

- **Behaviour unchanged**: every change is to human-readable string content only. No branch condition, no return value other than message text, no exit code, no hook decision, no JSON shape changes. The full gate passes (see hard constraint HC-1 and HC-2) after the lockstep test updates. [AC8]
- **JSON protocol structure preserved**: keys and structural/enum values in every hook response (`"decision"`, `"deny"`, `"allow"`, `"permissionDecision"`, `hookEventName`, and all other keys/enums) are byte-identical to today's values; only the human-readable string content of `reason`, `permissionDecisionReason`, and `additionalContext` changes. [AC8]
- **#83 colour behaviour preserved**: no colour code, colour table, or colour gate is touched. Where a retuned line is wrapped by #83's colour treatment today, that wrapping is preserved; only the words inside change. [AC8]
- **#84 glyph behaviour preserved**: no glyph code, glyph table, or glyph gate is touched. Where a retuned string is prefixed by #84's outcome glyph today, that prefix is preserved; only the words inside change. [AC8]
- **Lockstep tests**: every test that asserts a human string altered by this retune is updated in the same change. After the retune, no test still asserts old wording; the full gate is green. [AC9, AC10]
- **Zero new dependency**: no crate is added; `Cargo.toml` and `Cargo.lock` are unchanged in respect of dependencies. [AC10]
- **Glossary consistency**: across all retuned strings, each concept is named by exactly one glossary term (Arai / rule(s) + guardrails / the model / you), held consistently throughout. [AC6]
- **User-facing scope only**: internal-only strings that are never surfaced to a user are left accurate and untouched. The implementor must distinguish user-reaching strings from developer-only strings and retune only the former. [AC4]
- **Idempotency**: the retune is a one-time edit of static string content; the operation has no state of its own. Applying the same wording change twice produces the same result.
- **Ordering**: edits to different files are independent. There are no ordering constraints between individual string changes.
- **Atomicity**: all string changes and their corresponding test updates must be delivered in a single change (one commit or PR). A partial retune — some strings retuned, their test assertions not yet updated — is not an acceptable intermediate state.
- **Concurrency**: not applicable. This module performs file edits; concurrent invocation of the editing process is outside scope.
- **Resource bounds**: not applicable. This is an authoring/editing task with no runtime resource profile.

## Dependencies

None. The retune reads alongside the #83 colour helpers and #84 glyph helpers to understand their wrapping context but calls and changes neither. No other module or contract is depended upon.

## Referenced data shapes

- **VoiceSpec**: the six register rules committed into `docs/voice.md` (see above).
- **SelfReferenceGlossary**: the five-row one-term-per-concept table committed into `docs/voice.md` (see above).
- **RetunedString** (conceptual, not a type): a user-facing string after retune — same position and role in the program as before (same branch, same JSON key, same colour/glyph wrapping), different human-readable words, satisfying VoiceSpec and using SelfReferenceGlossary terms.

---

## Edit surfaces (bounded — named, user-visible sites)

These are the existing sites where current wording is replaced by restrained-declarative wording. Each is a content edit only; none contains new logic.

### `src/hooks.rs`
The rendered human messages:
- The deny `reason` / `permissionDecisionReason` (including the `deny_reason` builder and its `"Arai: blocking rule matched"` fallback).
- The `additionalContext` prose (including the `"Arai guardrails:"` header and per-rule lines, and the `"[Post-action review] …"` prefix).
- The internal-error fail-closed text (`"Arai: internal error, blocking for safety"`).

JSON keys and enum values are not touched. The #84 glyph prefix on these strings is preserved. [AC2]

### `src/init.rs`
The init / deinit flow strings:
- The `"Done. Arai is now watching your rules …"` footer.
- The `"Arai is no longer watching this project."` footer.
- Step headers ("Scanning for instruction files…", "Extracting guardrails…", "Setting up hooks…").
- User-facing `Failed to …` error messages on these paths.

[AC3]

### User-visible error messages across `src/`
The `Result<_, String>` messages and `eprintln!` strings that actually reach a user (the init/deinit/store-open paths, `main.rs` file-read paths, `upgrade.rs`/`extends.rs` user-driven commands, the `arai hook error: {e}` diagnostic). Retuned from bare `Failed to X` to specific declarative form. Internal-only strings that are never shown to a user are NOT touched. [AC4]

### `src/main.rs` + `src/stats.rs` command-output prose
- `status` / `why` / `audit` / `stats` headers.
- "No rules"/"No audit entries"/"No severity overrides" lines.
- Hint lines ("Run `arai init` first.", "Try phrasing list items as imperatives…", "Run `arai scan` after saving…").

Sentence case, glossary-aligned, no flourish. The #83 colour on headers is preserved. [AC5]

### `README.md`
A light pass on the intro/tagline voice only. Deeper doc work is explicitly out of scope and must be flagged as a follow-on, not done here. [AC7]

### Carve-outs (scope boundaries — what is never touched)

- **JSON keys and structural/enum values**: `"decision"`, `"deny"`, `"allow"`, `"permissionDecision"`, `hookEventName`, and every other key/enum stay byte-identical. Only the human-readable string content of `reason`, `permissionDecisionReason`, and `additionalContext` changes.
- **#83 colour and #84 glyph behaviour**: no colour code, glyph code, table, or gate is edited. Retuned lines keep their colour wrapping and glyph prefix.
- **Internal/never-shown strings**: left accurate and untouched.
- **Telemetry / log internals**: not user-facing prose; untouched.
- **Deep README/documentation rewrite**: the README pass is intro/tagline voice only; broader doc work is a flagged follow-on.
- **Any new dependency**: no crate is added.
- **Any logic/behaviour change**: this is an editorial retune; observable behaviour is identical before and after.

---

## Hard constraints

These are non-negotiable. A deliverable that violates any one of these does not satisfy this contract, regardless of how well the wording reads.

- **HC-1 — Behaviour UNCHANGED — wording only.** No logic, branch, return value (beyond message text), exit code, or hook decision changes. [AC8]
- **HC-2 — JSON protocol structure preserved.** Hook-response keys and structural/enum values are byte-identical to today's values; only human-readable string content changes. [AC8]
- **HC-3 — #83 colour + #84 glyph behaviour preserved.** No colour/glyph code, table, or gate is edited; retuned lines keep their wrapping and prefix. [AC8]
- **HC-4 — Zero new dependency.** No crate added; `Cargo.toml` and `Cargo.lock` are unchanged in respect of dependencies. [AC10]
- **HC-5 — Glossary consistency.** Each concept is named by exactly one glossary term across all retuned strings, held consistently. [AC6]
- **HC-6 — Lockstep tests (CRITICAL).** Many tests assert exact human strings (`tests/hooks_safety.rs`, the brand/glyph verifier files, integration tests). Every test asserting a string this retune changes MUST be updated in the same change. The implementor greps for every changed string and updates the asserting tests; no assertion is left on old wording. [AC9]

---

## Acceptance criteria

Each AC is stated as a verifiable given/when/then pass-fail description.

### AC1 — Voice spec committed

**Given** the module deliverable has been applied to the repository;  
**when** a reviewer navigates to `docs/voice.md`;  
**then** the file exists and is tracked by version control, contains the six VoiceSpec rules verbatim (as written in this contract), contains the five-row SelfReferenceGlossary table verbatim (as written in this contract), and is concrete enough that the question "does this retuned string obey the spec?" has a yes/no answer without further consultation.

**Fails if:** `docs/voice.md` is absent, is not committed, is missing any of the six rules, is missing any row of the glossary, or uses paraphrased/abbreviated versions of the committed text.

---

### AC2 — `src/hooks.rs` rendered messages retuned

**Given** the module deliverable has been applied;  
**when** a reviewer reads the human-readable string values produced by the deny-reason builder, the `additionalContext` prose, and the internal-error fail-closed path in `src/hooks.rs`;  
**then** every string obeys VoiceSpec rules 1–6: it is calm and declarative (rule 1); any error is specific, not a bare "Failed to X" (rule 2); a deny quotes the rule and its `source:line` (rule 3); no anthropomorphism (rule 4); full sentences use sentence case and end with a period (rule 5); and the #84 glyph prefix on these strings is unchanged (rule 6).

**Fails if:** any of the above strings retains a devtool-voice phrase; if the deny fallback text is `"Arai: blocking rule matched"` or equivalent; if the internal-error text is `"Arai: internal error, blocking for safety"` or equivalent; if the #84 glyph prefix has been removed or altered; if any JSON key or enum value has changed.

---

### AC3 — `src/init.rs` / deinit flow strings retuned

**Given** the module deliverable has been applied;  
**when** a reviewer reads the strings emitted during `arai init` and `arai deinit` (footers, step headers, user-facing error messages on these paths);  
**then** the init footer reads as a declarative statement of enforcement (not "Done. Arai is now watching your rules …"); the deinit footer reads as a declarative statement of non-enforcement (not "Arai is no longer watching this project."); step headers use sentence case; and no user-facing error on these paths uses a bare "Failed to X" form.

**Fails if:** the init footer retains "Done." or "watching your rules" or equivalent friendly-devtool phrasing; if the deinit footer retains "watching"; if any step header uses Title Case; if any user-facing error on these paths retains a bare "Failed to X".

---

### AC4 — User-visible error messages across `src/` retuned to specific declarative form

**Given** the module deliverable has been applied;  
**when** a reviewer audits every `Result<_, String>` message and `eprintln!` string that reaches a user (the init/deinit/store-open paths, `main.rs` file-read paths, `upgrade.rs`/`extends.rs` user-driven commands, the `arai hook error: {e}` diagnostic);  
**then** no user-reaching string uses a bare "Failed to X" form; each uses the specific declarative form `Could not <verb> <named thing>: {e}` (or an equally specific declarative retaining the `{e}` detail); and internal-only strings that are never surfaced to a user are confirmed untouched and accurate.

**Fails if:** any user-reaching string retains a bare "Failed to X" form; if any internal-only string has been altered; if the `{e}` detail has been dropped from any error message.

---

### AC5 — Command-output prose retuned (`src/main.rs` + `src/stats.rs`)

**Given** the module deliverable has been applied;  
**when** a reviewer reads the output of `arai status`, `arai why`, `arai audit`, and `arai stats` (headers, "no rules"/"no audit entries"/"no severity overrides" lines, hint lines);  
**then** every header uses sentence case; every full sentence ends with a period; no line uses marketing flourish or exclamation marks; the #83 colour wrapping on headers is unchanged; and every hint line is calm and declarative (e.g. `Run \`arai init\` first.` rather than an imperative exhortation).

**Fails if:** any header uses Title Case or ALL CAPS where it did not previously; if any line retains a friendly-devtool phrase; if the #83 colour on any header has been altered; if any hint line retains exhortation flourish.

---

### AC6 — Glossary consistency across all retuned strings

**Given** the module deliverable has been applied;  
**when** a reviewer reads all retuned strings across all edit surfaces and searches for each concept (the tool, a single constraint, the system/command surface, the AI actor, the human reader);  
**then** the tool is called "Arai" and never "we" or "I"; a single constraint is called "rule" (or "rules"); the collective system is called "guardrails" when referring to the command surface and "rules" at the unit level; the AI actor is called "the model"; the human reader is addressed as "you" and never "the user" in user-facing prose; and each concept is named by exactly one term, held consistently throughout.

**Fails if:** "we", "I", "the agent", "the assistant", "Claude", or "the user" appears in any user-facing retuned string; if "rule" and "guardrails" are used interchangeably for the same concept in user-facing prose; if any concept is named inconsistently across the retuned surfaces.

---

### AC7 — README intro/tagline voice aligned

**Given** the module deliverable has been applied;  
**when** a reviewer reads the README intro/tagline section;  
**then** it reads in the restrained-declarative register (declarative, no friendly-devtool flourish, no anthropomorphism, infrastructural statement of what Arai is); the pass is light (only the intro/tagline, not a deep rewrite); and a note in the PR or commit description flags deeper doc work as a follow-on.

**Fails if:** the README intro/tagline retains a friendly-devtool voice; if the pass extends beyond the intro/tagline to other README sections; if no follow-on note flags the deeper doc work.

---

### AC8 — Behaviour unchanged; JSON/colour/glyph preserved

**Given** the module deliverable has been applied;  
**when** a reviewer runs `git diff` against the pre-retune baseline and inspects the diff;  
**then** no diff line changes a branch condition, return value (beyond message text), exit code, hook decision, or control-flow path; no diff line changes a JSON key or structural/enum value; no diff line touches any colour (#83) code, table, or gate; no diff line touches any glyph (#84) code, table, or gate; and `cargo test` (after the lockstep test updates in AC9) is green.

**Fails if:** any diff line alters logic, control flow, or an exit code; if any JSON key or enum value is different after the retune; if any colour or glyph code has been modified; if `cargo test` is not green after the lockstep test updates.

---

### AC9 — Tests updated in lockstep; no assertion on old wording

**Given** the module deliverable has been applied;  
**when** a reviewer runs `grep` against the test suite (`tests/hooks_safety.rs`, the brand/glyph verifier test files, and integration tests) searching for each pre-retune string that was changed;  
**then** no test still contains an assertion on old wording; every test that previously asserted a changed string now asserts the corresponding retuned string; and `cargo test` is green.

**Fails if:** any test still asserts a pre-retune wording that was changed; if `cargo test` fails; if the test updates and the string changes were delivered in separate changes (they must be in the same change).

---

### AC10 — Full gate passes; zero new dependency

**Given** the module deliverable has been applied;  
**when** a reviewer runs the full gate:
1. `cargo fmt --all -- --check`
2. `cargo clippy --all-targets`
3. `cargo test`
4. `diff Cargo.toml` and `diff Cargo.lock` (or equivalent inspection) against the pre-retune baseline;

**then** step 1 exits clean (no formatting errors); step 2 exits with no new warnings; step 3 exits green (all tests pass, including the lockstep-updated assertions); and step 4 shows no new dependency has been added.

**Fails if:** `cargo fmt --all -- --check` reports formatting errors; if `cargo clippy --all-targets` reports new warnings; if `cargo test` fails; if `Cargo.toml` or `Cargo.lock` shows a new crate dependency.

---

## Verifier obligations

The verifier (the agent or human who signs off on this deliverable) is contracted to perform each of the following checks. Partial verification is not acceptable.

### Checklist

- [ ] **Voice consistency review.** Open `docs/voice.md`. For each retuned string at each edit surface, check the string against every applicable VoiceSpec rule (rules 1–6) and confirm the SelfReferenceGlossary terms are used. This is a reading check, not a mechanical one — judgement is required, but the committed spec makes it anchored.
- [ ] **AC1 — `docs/voice.md` present and correct.** File exists, is committed, contains VoiceSpec and SelfReferenceGlossary verbatim as specified.
- [ ] **AC2 — `src/hooks.rs` strings checked.** Deny reason, additionalContext prose, internal-error text — all calm, declarative, adequate-at-the-deny, no anthropomorphism, sentence case; #84 glyph prefix present and unchanged; JSON keys/enums byte-identical.
- [ ] **AC3 — `src/init.rs` strings checked.** Init/deinit footers and step headers — no "Done." / "watching your rules" / equivalent; sentence case throughout.
- [ ] **AC4 — User-facing errors across `src/` checked.** No bare "Failed to X" on a user-reaching path; `{e}` detail retained; internal-only strings confirmed untouched.
- [ ] **AC5 — Command-output prose checked.** Headers sentence case; #83 colour unchanged; hint lines calm declarative.
- [ ] **AC6 — Glossary consistency checked.** No "we", "I", "the agent", "the assistant", "Claude", "the user" in user-facing strings; every concept named by its one committed term.
- [ ] **AC7 — README pass is light.** Intro/tagline voice aligned; no deeper rewrite; follow-on flagged.
- [ ] **Full gate — run in this order:**
  - `cargo fmt --all -- --check` (clean)
  - `cargo clippy --all-targets` (no new warnings)
  - `cargo test` (all green, including lockstep-updated test assertions)
- [ ] **Lockstep grep — no old wording in tests.** For each string that was changed, `grep` the test suite for the pre-retune wording. Result must be zero matches.
- [ ] **JSON/colour/glyph diff clean.** `git diff` confirms: no JSON key or enum value changed; no line in a colour (#83) or glyph (#84) file or function was edited.
- [ ] **`Cargo.toml` unchanged.** Diff confirms zero new dependencies.