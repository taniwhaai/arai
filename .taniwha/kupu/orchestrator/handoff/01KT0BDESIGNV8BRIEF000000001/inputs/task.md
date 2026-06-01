# Task — design-doc (Arai design v6, issue #84)

Produce design v6 for issue #84 — gateway-derived outcome glyphs. Inputs (this
handoff's inputs/): `brief.md` (v8, authoritative, AC1–AC10, the chosen glyph set),
`project_context.yaml` (redacted; note src/style.rs already exists from #83),
`design_v5_reference.md` (STYLE reference only — the #83 palette design; do not extend).

Output: `design_doc.md` in this handoff's outputs/ (dispatcher promotes to design v6).

Required:
1. Front matter: version: 6, parent_brief_version: 8, tier: <declared>.
2. **Tier declaration + justification** (plausibly single_module; the glyph logic +
   call-site application is one cohesive capability). Do NOT use full_decomposition.
3. **Module placement decision (commit + justify):** extend src/style.rs vs new
   src/glyph.rs. Either is acceptable; whichever you choose MUST reuse the existing
   style.rs `should_colorize` + ochre helpers for the cross.
4. Public surface (prose, no Rust): `should_use_unicode()` (true unless ARAI_ASCII/
   NO_UNICODE env set, and locale LC_ALL/LC_CTYPE/LANG looks UTF-8; else ASCII;
   TTY-independent) and `outcome_glyph(outcome, unicode, colorize) -> String`
   (Block→blocked, Warn|Inform→warned, Allow→allowed; the exact glyph table below;
   ochre on the ✕ ONLY when colorize).
   Glyph table: blocked `●·│✕` / ascii `o.|x`; allowed `│●│` / ascii `|o|`;
   warned `●·│` / ascii `o.|`.
5. Integration surfaces + carve-outs (name explicitly):
   - src/main.rs: `arai audit` + `arai why` human (non-json) render — prefix outcome glyph.
   - src/stats.rs: replace the generic `⚠` with the warned glyph.
   - src/hooks.rs: live Pre/Post surface (deny reason / human additionalContext line)
     carries the glyph — **chars only, colorize=false ALWAYS on the hook path** (no
     ANSI colour — preserves the #83 hook carve-out; the cross is the bare glyph there).
   - **--json EXCLUSION:** no glyphs in any --json field value (structured severity only).
   - Leave the audit-chain ✓/✗ integrity markers as-is (different semantic).
6. Carry AC1–AC10 verbatim from the brief as the module's ACs.
7. Data shapes (prose): the outcome→glyph mapping table; the unicode/colorize decision inputs.
8. Files touched: the glyph module (style.rs or glyph.rs) + main.rs, stats.rs, hooks.rs + tests.
9. Constraints (binding): ZERO new dependency; reuse #83 style gate/ochre; ASCII
   fallback mandatory; ochre cross only in human TTY (never hook / NO_COLOR / piped).
10. Testing: unit tests (mapping, unicode vs ascii, ochre only when colorize) +
    subprocess integration test (env!("CARGO_BIN_EXE_arai"), ARAI_BASE_DIR, NO new dep)
    asserting glyph in human audit/why, ARAI_ASCII=1 ⇒ ascii-only, hook output has
    glyph but ZERO ANSI, --json has no glyphs.
11. Full-gate process note (carry from prior cycles): leaf + verifier run cargo fmt
    --all + clippy --all-targets + cargo test (not just test).
12. Out of scope: brand-mark designs (#65-67), copy tone (#85), --json glyphs,
    any new dependency, theming/custom glyph config, the audit-verify ✓/✗ markers.

Language-neutral (no Rust code). Emit `re_raise.yaml` instead ONLY if genuinely
ambiguous (it is detailed). Final message: tier chosen + module placement + one-line summary.
