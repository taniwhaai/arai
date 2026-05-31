# Task — design-doc (Arai design v5, issue #83)

Produce design v5 for issue #83 — apply the pounamu/ochre brand palette across CLI
output. Inputs (this handoff's inputs/): `brief.md` (v7, authoritative, AC1–AC10),
`project_context.yaml` (redacted), `design_v4_reference.md` (STYLE reference only —
different problem; do not extend it).

Output: `design_doc.md` in this handoff's outputs/ (dispatcher promotes to design v5).

Required:
1. Front matter: version: 5, parent_brief_version: 7, tier: <declared>.
2. **Tier declaration + justification.** Plausibly **single_module** (one new
   `src/style.rs`; the four call-site files are mechanical integration, not separate
   modules). Justify; do NOT use full_decomposition.
3. Module summary for `style` (palette constants foreground-only; `should_colorize`
   gate: NO_COLOR + IsTerminal(not-a-TTY) + CLICOLOR_FORCE override; semantic helpers
   structural/passage/dim/warn/error returning styled-or-plain String; truecolor when
   on, plain when off — no 16/256 approximation). Define boundaries, inputs/outputs
   (prose), side effects, error semantics.
4. **Explicit named carve-out:** hook-protocol JSON output (src/hooks.rs Pre/Post)
   stays uncoloured/byte-identical — name it as a deliberate scope boundary (AC8),
   not just an implicit gate consequence.
5. Carry AC1–AC10 verbatim from the brief and (if multi-module) assign each to a
   module; otherwise list them as the module's ACs.
6. Data shapes (prose): the palette colours, the helper return contract, the gate's
   decision inputs.
7. Files touched: src/style.rs (new) + mod decl in main.rs; src/main.rs, src/audit.rs,
   src/stats.rs, src/guardrails.rs (call-sites) + tests.
8. Constraints (binding): hand-rolled 24-bit truecolor, ZERO new dependency;
   foreground-only; no stoplight red/green; pounamu RGB(31,77,63), ochre RGB(184,118,58).
9. Testing: style.rs unit gate-matrix tests + a subprocess integration test
   (env!("CARGO_BIN_EXE_arai"), ARAI_BASE_DIR temp isolation, NO new dep) asserting
   zero ANSI in --json output, in the hook `guardrails --match-stdin` output, when
   piped, and when NO_COLOR set.
10. Full-gate process note (carry from prior cycles): leaf + verifier run cargo fmt
    --all + clippy --all-targets + cargo test (not just test).
11. Out of scope: glyphs (#84), copy tone (#85), colour in machine-consumed output,
    any new dependency, theming/custom palettes, 16/256 approximation, changing the
    brand colours / background handling.

Language-neutral (no Rust code). Emit `re_raise.yaml` instead ONLY if genuinely
ambiguous (it is detailed). Final message: tier chosen + one-line module summary.
