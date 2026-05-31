# Task — contract-derivation (module: brand-palette-styling)

Derive ONE standalone contract/manifest for the single module **brand-palette-styling**
from approved design v5 (`inputs/design_v5.md`). Single_module ⇒ one contract, no
shared vocabulary, no composition contract. Output:
`contract-brand-palette-styling-v1.md` in this handoff's outputs/.

The contract must be buildable by an isolated implementor who never saw the design.
Carry AC1–AC10 as **verifiable pass/fail test descriptions** (given/when/then), not
restatements.

Binding constraints to encode:
- Hand-rolled **24-bit truecolor ANSI**; **ZERO new dependency** (no colour crate).
- **Foreground-only** — never set a background.
- **No stoplight** red/green.
- Palette: pounamu RGB(31,77,63) `#1f4d3f`; ochre RGB(184,118,58) `#b8763a`.
- `should_colorize` gate precedence: `NO_COLOR` set → OFF; else `CLICOLOR_FORCE`
  set → ON; else ON iff the target stream is a TTY (`std::io::IsTerminal`, already
  used in src/config.rs — no crate). When OFF: emit plain string, ZERO ANSI (no
  16/256 approximation).
- Semantic helpers returning styled-or-plain String: `structural()` (pounamu),
  `passage()` (ochre — decision moments: rule firings, prompt matches), `dim()`
  (faint), `warn()`/`error()` (ochre/bold, NOT red). Reset after each span.
- Pure-ish: the helpers' only effect is producing a string; no I/O. The gate reads
  env + stream TTY status.

Carve-out (AC8): **machine-consumed output stays byte-identical** — hook-protocol
JSON (src/hooks.rs Pre/Post), every `--json` rendering, and piped/non-TTY output
contain ZERO ANSI. Name this as an explicit contract boundary.

Integration surfaces (where the module's helpers are applied — name them, but the
contract is for the `style` module itself): src/main.rs (cmd_status, cmd_why,
cmd_guardrails human listing, eprintln! "arai: …" error/warning on stderr),
src/audit.rs (rule firings → ochre), src/stats.rs (structure → pounamu),
src/guardrails.rs (format_trace if human-facing).

Testing requirements (encode as ACs the verifier will check):
- Unit gate-matrix tests for `should_colorize` / helpers: NO_COLOR → plain; non-TTY
  → plain; CLICOLOR_FORCE → styled; styled output contains the expected truecolor
  ANSI; plain output contains none.
- A subprocess integration test (repo pattern: env!("CARGO_BIN_EXE_arai"),
  ARAI_BASE_DIR temp isolation, NO new dep) asserting ZERO ANSI in: any `--json`
  output, the hook `guardrails --match-stdin` output, piped output, and NO_COLOR output.
- AC10 full gate: `cargo fmt --all -- --check` + `cargo clippy --all-targets` (no new
  warnings) + `cargo test` — state this as a verifier requirement in the contract.

Language-neutral (no Rust code; field/return types in neutral notation). Emit
`re_raise.yaml` instead ONLY for a genuine design gap (there should be none). Final
message: short confirmation of the file written.
