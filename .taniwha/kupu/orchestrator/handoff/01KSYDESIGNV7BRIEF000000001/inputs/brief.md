# Brief — Issue #83: apply pounamu/ochre/paper/ink palette across CLI output

## Source
GitHub issue #83 (parent #64, "epic: UI retune against new brand").

## Goal
Give Arai's terminal output a branded, semantic colour palette. Replace/introduce
colour so the CLI reads as branded without becoming hard to read on dark or light
terminals. Colour applied **semantically, not decoratively**.

## Spec (from the issue)
- **Pounamu `#1f4d3f`** (RGB 31,77,63) — structural / informational text.
- **Ochre `#b8763a`** (RGB 184,118,58) — reserved for the "passage" / decision
  moments (rule firings, prompts).
- **Paper / ink** for backgrounds — terminal-respecting: do NOT override the user's
  background (set foreground only).
- **Avoid stoplight** green-for-allow / red-for-block. Arai isn't a traffic light.

## Context / findings
- The codebase currently has **no colour** output — this introduces a palette layer
  rather than replacing ad-hoc colour.
- `std::io::IsTerminal` is already used (src/config.rs) — TTY detection needs no crate.
- Chosen implementation: **hand-rolled 24-bit truecolor ANSI, zero new dependency.**
- CRITICAL carve-out: the hook handler (src/hooks.rs) emits JSON CONSUMED BY THE
  AGENT (Claude/Grok) on stdout — that output and its additionalContext/reason
  strings must NEVER be coloured (ANSI would corrupt the protocol / pollute the
  agent context). The hook path runs with stdout captured (not a TTY) so the gate
  disables colour anyway, but this is an explicit scope boundary.

## Design
New `src/style.rs` module:
- Palette foreground-only constants: pounamu (31,77,63), ochre (184,118,58).
- `should_colorize(stream) -> bool` gate: false if NO_COLOR env set; false if the
  target stream is not a TTY (IsTerminal); CLICOLOR_FORCE overrides to force on.
  Emit truecolor when on; never approximate brand colours in 16/256 (emit no colour
  on the rare non-truecolor terminal — safe + simple).
- Semantic helpers returning styled-or-plain String: structural() (pounamu),
  passage() (ochre — decision moments), dim() (faint secondary), warn()/error()
  (ochre/bold, NOT red). Reset after each span.

Apply across the human-readable command paths (mechanical): src/main.rs
(cmd_status, cmd_why, cmd_guardrails listing, the eprintln! "arai: …" error/warning
paths on stderr), src/audit.rs (rule firings → ochre), src/stats.rs (structure →
pounamu), src/guardrails.rs (format_trace if human-facing).

## Acceptance criteria
- AC1: a `style` module centralises the pounamu/ochre palette + should_colorize gate.
- AC2: NO_COLOR set → zero ANSI in any output.
- AC3: output not a TTY (e.g. `arai status | cat`) → zero ANSI.
- AC4: every `--json` output (audit/stats/why/guardrails/lint/diff) contains zero ANSI escapes.
- AC5: pounamu applied to structural/info text; ochre reserved for decision/passage
  moments (rule firings, prompt matches) across status/why/audit/stats/guardrails.
- AC6: no stoplight green-for-allow / red-for-block introduced anywhere.
- AC7: foreground-only — no background is ever set (terminal background respected).
- AC8: hook-protocol JSON output (src/hooks.rs Pre/Post) byte-identical to today —
  no ANSI injected into agent-consumed JSON/additionalContext.
- AC9: readable on dark AND light terminals (foreground-only truecolor; verify manually).
- AC10: full gate — cargo fmt --all -- --check + cargo clippy --all-targets (no new
  warnings) + cargo test all pass.

## Scope / files
- New: src/style.rs (+ `mod style;` in main.rs).
- Modify: src/main.rs, src/audit.rs, src/stats.rs, src/guardrails.rs.
- Tests: style.rs unit tests (gate matrix: NO_COLOR/non-tty → plain; tty+colour →
  ANSI present) + a subprocess test asserting no ANSI in `--json` output and in the
  hook `guardrails --match-stdin` output, and no ANSI when piped / NO_COLOR set.

## Process requirements (standing lessons)
- Leaf + verifier MUST run the FULL gate (cargo fmt --all + clippy --all-targets +
  cargo test), not just test — CI gates rustfmt.
- Integration tests use the repo's subprocess pattern (env!("CARGO_BIN_EXE_arai"),
  ARAI_BASE_DIR temp isolation) — NO new dependency (no tempfile, no colour crate).

## Out of scope
- Glyph/iconography changes (issue #84), copy-tone audit (issue #85).
- Any colour in machine-consumed output (hook JSON, --json, piped) — must stay plain.
- Adding a colour crate or any new dependency.
- Theming/config for custom palettes; 16/256-colour approximation of brand colours.
- Changing the brand colours themselves or background handling beyond "leave it alone".