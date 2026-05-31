# Task — leaf-implementation (module: brand-palette-styling)

Implement the single module **brand-palette-styling** against contract v1
(`inputs/contract-brand-palette-styling-v1.md`). `inputs/project_context.yaml` has
the Rust/cargo conventions.

## What to build
1. **New file `src/style.rs`** + `mod style;` in `src/main.rs`. Contents:
   - Foreground-only palette constants: pounamu RGB(31,77,63), ochre RGB(184,118,58).
   - `should_colorize(stream) -> bool` gate with precedence: `NO_COLOR` env set → OFF;
     else `CLICOLOR_FORCE` set → ON; else ON iff the target stream is a TTY
     (`std::io::IsTerminal` — already used in src/config.rs, NO new crate). When OFF:
     emit plain string with ZERO ANSI (no 16/256 approximation).
   - Semantic helpers returning styled-or-plain `String`: `structural()` (pounamu),
     `passage()` (ochre — decision moments), `dim()` (faint), `warn()`/`error()`
     (ochre/bold, NOT red). Each helper resets after its span (`\x1b[0m`). Truecolor
     escape form `\x1b[38;2;R;G;Bm`.
2. **Apply helpers across human-readable call sites** (mechanical):
   - `src/main.rs`: `cmd_status`, `cmd_why`, `cmd_guardrails` (human listing), and the
     `eprintln!("arai: …")` error/warning paths (stderr-gated).
   - `src/audit.rs`: rule firings → ochre/passage.
   - `src/stats.rs`: structure/headers → pounamu/structural.
   - `src/guardrails.rs`: `format_trace` if it renders human-facing output.

## HARD carve-outs (AC8 — do not violate)
- Do NOT colour `src/hooks.rs` hook-protocol JSON output (PreToolUse/PostToolUse) or
  its additionalContext/reason strings — it is consumed by the agent; leave byte-identical.
- Do NOT route any `--json` rendering through the helpers (those use serde) — `--json`
  output must contain ZERO ANSI.

## Constraints
- ZERO new dependency (no colour crate; do NOT modify Cargo.toml deps).
- Foreground-only (never set a background). No stoplight red/green.

## Tests
- `src/style.rs` `#[cfg(test)]` gate-matrix unit tests: NO_COLOR → plain; non-TTY →
  plain; CLICOLOR_FORCE → styled; styled output has the expected truecolor ANSI;
  plain output has none.
- A subprocess integration test under `tests/` (repo pattern:
  `env!("CARGO_BIN_EXE_arai")`, `ARAI_BASE_DIR` temp isolation, NO new dep) asserting
  ZERO ANSI (`\x1b`) in: a `--json` output, the hook `guardrails --match-stdin`
  output, piped output, and `NO_COLOR=1` output.
- Existing test suite must still pass.

## Full gate (AC10 — MANDATORY, in order, fix all): `cargo fmt --all` →
`cargo clippy --all-targets` (no new warnings) → `cargo test`. Only output your
bundle after all three pass.

## DO NOT COMMIT, do NOT switch branches (branch is feat/83-cli-palette; working
tree only — dispatcher commits at end).

## Output: write `implementation_manifest.yaml` to outputs/ (module, contract_version:1,
files_created/modified, gate_results with test counts, prose summary, committed:false).
Emit outputs/re_raise.yaml instead ONLY for a genuine contract gap. Final message:
short confirmation incl. gate results.
