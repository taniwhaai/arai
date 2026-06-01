# Brief — Issue #85: copy-tone audit (restrained-declarative register)

## Source
GitHub issue #85 (parent #64). The last code/editorial-buildable issue in the
backlog. Builds on #83 (palette, merged) and #84 (gateway glyphs, merged PR #141).

## Goal
Editorial retune (NOT new logic): pass over Arai's user-facing strings (CLI output,
prompts, error messages, hook messages) and move them from a friendly-devtool voice
to a restrained, infrastructural register — "less devtool, more piece-of-
infrastructure" — while keeping ADEQUATE detail at the moment a user is about to lose
work to a deny. Chosen register (user-approved): **restrained declarative**.

## Current voice (sampled) — what to move away from
- Anthropomorphic / friendly: "Arai is now watching your rules", "Done.".
- Generic error templates: "Failed to read settings.json: {e}", "Failed to write X".
- Terse deny: "Arai: blocking rule matched".
- Inconsistent self-reference (how Arai names itself, the rules, the model, the user).

## Voice spec (restrained declarative)
1. Declarative, calm sentences — state what is / what happened. No exclamation, no
   "Done!", no cute/marketing flourish.
2. Specific over template — errors name the thing + consequence ("Could not read
   settings.json: {e}"), not bare "Failed to X".
3. Adequate at the deny — quote the rule + source:line; enough to act, no lecture.
4. No anthropomorphism — "watching your rules" → "watching this project".
5. Sentence case, periods on full sentences. KEEP the #84 functional glyphs and the
   #83 structural colour unchanged — this slice changes WORDS, not colour/glyph behaviour.

## Self-reference glossary (consistency backbone — the issue asks for this)
- the tool → **Arai** (proper noun; never "we").
- a single constraint → **rule**; the system/command → **guardrails** (the
  `arai guardrails` command keeps its name). One term per concept, held consistently.
- the AI actor → **the model**.
- the human → **you**.
(Design/contract may refine exact glossary values but MUST commit to consistency.)

## Scope (bounded — highest-traffic, user-visible surfaces)
- src/hooks.rs — rendered human messages: deny reason (deny_reason), additionalContext
  prose, the internal-error fail-closed text.
- src/init.rs — init / deinit flow strings.
- User-visible error messages across src/ — the Result<_, String> messages and
  eprintln! that actually reach a user (NOT internal-only strings never shown).
- Command-output prose in src/main.rs + src/stats.rs — status, why, audit, stats
  headers, the "no rules" / hint lines.
- README.md — a LIGHT pass on intro/tagline voice only (deeper doc work overlaps the
  scope-honesty epic; follow-on).

## Acceptance criteria
- AC1: a short voice spec + self-reference glossary committed in-repo (e.g.
  docs/voice.md) so the register stays maintainable.
- AC2: hooks.rs rendered human messages match the restrained-declarative register.
- AC3: init.rs / deinit flow strings retuned.
- AC4: user-visible error messages retuned to specific declarative form (no bare
  "Failed to X" on user-facing paths).
- AC5: command-output prose (status/why/audit/stats headers, "no rules"/hint lines) retuned.
- AC6: consistent self-reference per the glossary across all retuned strings.
- AC7: README intro/tagline voice aligned (light pass); deeper doc work flagged follow-on.
- AC8: behaviour UNCHANGED — wording only; no logic change; JSON protocol keys/values
  structure, #83 colour, and #84 glyph behaviour preserved.
- AC9: every test asserting a changed string is updated in lockstep; cargo test green
  (no assertion left on old wording).
- AC10: full gate — cargo fmt --all -- --check + cargo clippy --all-targets (no new
  warnings) + cargo test; ZERO new dependency.

## CRITICAL implementation note (carry into contract)
Many tests assert exact human strings (tests/hooks_safety.rs, the brand/glyph
verifier files, integration tests). JSON protocol keys/values are UNCHANGED, but
reworded human reason/flow strings WILL break their asserting tests. The leaf MUST
grep for every changed string and update the asserting tests in the same change. The
verifier independently re-runs the full gate AND reviews voice consistency against
the committed spec, confirming no test still asserts old wording.

## Out of scope
- JSON protocol keys/values structure; #83 colour and #84 glyph behaviour.
- Internal/never-shown error strings (keep accurate).
- Telemetry/log internals; deep README/doc rewrite.
- Any new dependency; any logic/behaviour change.

## Process requirements (standing lessons)
- Leaf + verifier run the FULL gate (cargo fmt --all + clippy --all-targets + cargo
  test), not just test.
- Subjective ACs (AC2/AC5/AC6) are checked against the committed voice spec; the
  verifier reviews consistency and the dispatcher independently eyeballs the real
  retuned output before commit.