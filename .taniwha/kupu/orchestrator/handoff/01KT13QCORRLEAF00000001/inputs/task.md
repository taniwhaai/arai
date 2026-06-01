# Corrective leaf — AC6 glossary-consistency nits (issue #85, copy-tone-audit)

## Context

You are a corrective leaf-implementation subagent. The main leaf for issue #85 (copy-tone-audit) already completed successfully — the gate is green (fmt clean, clippy 0 new, 550+ tests pass), all hard constraints are met, and all acceptance criteria AC1–AC10 pass. The verifier returned `overall: partial` due to three narrow glossary-consistency nits that were missed.

This task is scoped to ONLY those three nit categories: string literals only in four source files. Nothing else.

## The approved glossary (already committed in docs/voice.md)

From the SelfReferenceGlossary:
- "rule" (plural "rules") is the unit term for a single constraint
- "guardrails" is the collective system / the `arai guardrails` command surface only
- User-visible errors use the form: `Could not <verb> <named thing>: {e}`

## Exact fixes required

### Fix 1 — src/init.rs, one string

Line ~24: Step header `"Extracting guardrails..."` uses "guardrails" as unit term.

Change to: `"Extracting rules..."`

### Fix 2 — src/main.rs, three strings

Lines 1213, 1337, 1559 (approximately): `"No guardrail database found.  Run \`arai init\` first."`

Change all three to: `"No rule database found.  Run \`arai init\` first."`

(Double space between sentences is already present in the original — preserve it.)

### Fix 3 — src/enrich.rs, ~10 user-visible error strings

All user-reaching error strings of the form `"Failed to <verb> <thing>: {e}"` on user-visible paths. Change to `"Could not <verb> <thing>: {e}"`.

Examples of what to search for (not exhaustive — grep the file):
- `"Failed to create ONNX session: {e}"`
- `"Failed to load ONNX model: {e}"`
- `"Failed to run inference: {e}"`
- `"Failed to download model: {e}"`
- `"Failed to read response: {e}"`
- and any others matching the pattern

Do NOT change internal-only strings that are never shown to a user. Do NOT change strings outside error/diagnostic paths.

### Fix 4 — src/scenarios.rs, one string

Line ~89: `"Failed to read scenario file {path}: {e}"` or similar.

Change to: `"Could not read scenario file {path}: {e}"` (exact form may vary — match the pattern and apply).

## Hard constraints — non-negotiable

- **String literals ONLY.** No logic, branch, control-flow, return type, or function signature changes.
- **JSON keys/enums untouched.** "decision", "deny", "allow", "permissionDecision", etc. stay byte-identical.
- **Colour and glyph unchanged.** Do not touch src/style.rs, any glyph table, any colour constant.
- **docs/voice.md untouched.** The committed voice spec file stays exactly as it is.
- **No new dependency.** Cargo.toml and Cargo.lock must be unchanged.
- **Branch: feat/85-copy-tone.** Working-tree only — do NOT commit.

## Gate — run in this order after making changes

1. `cargo fmt --all` (auto-fix formatting)
2. `cargo fmt --all -- --check` (confirm clean)
3. `cargo clippy --all-targets` (zero new warnings; pre-existing ones are fine)
4. `cargo test` (all suites green)

## Lockstep test check

Before reporting done, grep for each changed string in the test suite:

```bash
grep -r "Extracting guardrails" tests/ src/
grep -r "No guardrail database found" tests/ src/
grep -r "Failed to" tests/ src/enrich.rs  # to find any test that asserts old wording
grep -r "Failed to read scenario" tests/ src/
```

If any test asserts old wording, update the test to match the new string.

## Output

When done, write a brief implementation note to the output path:
`.taniwha/kupu/orchestrator/handoff/01KT13QCORRLEAF00000001/outputs/implementation_note.md`

Include:
- Files changed and specific lines
- Gate results (fmt/clippy/test counts)
- Lockstep grep result (expected: zero matches on old wording)
- Any test assertions updated (expected: none)
