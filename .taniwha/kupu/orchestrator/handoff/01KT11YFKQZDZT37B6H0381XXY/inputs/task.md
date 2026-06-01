# Task — leaf-implementation (module: copy-tone-audit)

Implement **copy-tone-audit** against contract v1 (`inputs/contract-copy-tone-audit-v1.md`).
This is an EDITORIAL retune (wording only) to the restrained-declarative register.
`inputs/project_context.yaml` has conventions.

## What to do
1. **Create `docs/voice.md`** — the committed VoiceSpec (6 rules) + self-reference
   glossary, verbatim from the contract (AC1).
2. **Retune user-facing strings** to the restrained-declarative register across the
   bounded surfaces (use the contract's before/after exemplars as ground truth):
   - `src/hooks.rs` — deny reason (`deny_reason`), additionalContext prose, the
     internal-error fail-closed text.
   - `src/init.rs` — init / deinit flow strings (footers, step lines).
   - User-visible error messages across `src/` — `Result<_,String>` messages and
     `eprintln!` that reach a user (e.g. "Failed to X: {e}" → "Could not <verb>
     <thing>: {e}"). NOT internal-only strings the user never sees.
   - `src/main.rs` + `src/stats.rs` — command-output prose: status/why/audit/stats
     headers, "no rules"/hint lines. Sentence case, glossary-aligned, no flourish.
   - `README.md` — a LIGHT pass on the intro/tagline voice only (do not rewrite the doc).
3. **Apply the glossary consistently:** the tool → Arai (never "we"); a single
   constraint → rule; the system/command → guardrails (the `arai guardrails` command
   name stays); the AI actor → the model; the human → you.

## HARD constraints
- Behaviour UNCHANGED — wording only; no logic, no control-flow change.
- JSON protocol keys and structural/enum values UNTOUCHED (only human-readable string
  *content* changes; e.g. `permissionDecision`, `"deny"`, `hookEventName` stay exact).
- #83 colour + #84 glyph behaviour UNTOUCHED (no colour/glyph code changed; the deny
  reason still flows through the existing glyph/colour wrappers — you only change the words).
- ZERO new dependency (do not touch Cargo.toml).

## CRITICAL — lockstep tests (HC-6)
Every test that asserts a human string you change MUST be updated in the same change,
or `cargo test` breaks. Before finishing: grep the repo for the OLD wording of each
string you changed (tests/hooks_safety.rs, tests/brand_palette_verifier.rs,
tests/gateway_glyphs_verifier.rs, tests/gateway_outcome_glyphs.rs, other tests/, and
all `#[cfg(test)]` blocks) and update each assertion to the new wording. Do NOT change
test logic — only the asserted string literals. Do NOT weaken a test (e.g. don't delete
an assertion to make it pass).

## Full gate (AC10 — MANDATORY, in order, fix all): `cargo fmt --all` →
`cargo clippy --all-targets` (no new warnings) → `cargo test`.

## DO NOT COMMIT, do NOT switch branches (branch feat/85-copy-tone; working tree only —
dispatcher commits at end).

## Output: `implementation_manifest.yaml` to outputs/ (files_created [docs/voice.md],
files_modified, a list of the notable before/after string changes, the tests updated in
lockstep, gate_results + counts, prose summary, committed:false). Emit outputs/re_raise.yaml
ONLY for a genuine contract gap. Final message: short confirmation incl. gate results +
which tests were updated in lockstep.
