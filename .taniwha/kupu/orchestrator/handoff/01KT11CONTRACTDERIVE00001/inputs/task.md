# Task — contract-derivation (module: copy-tone-audit)

Derive ONE standalone contract for **copy-tone-audit** from approved design v7
(`inputs/design_v7.md`). Single_module ⇒ one contract, no vocabulary, no composition.
Output: `contract-copy-tone-audit-v1.md` in this handoff's outputs/.

Carry AC1–AC10 as verifiable given/when/then pass-fail descriptions (not summaries).

Carry verbatim from design v7 (mark "committed — do not paraphrase"):
- The **VoiceSpec** (6 register rules: declarative/calm; specific-over-template errors
  `Could not <verb> <thing>: {e}`; adequate-at-the-deny (quote rule + source:line);
  no anthropomorphism; sentence case + periods; keep #83 colour + #84 glyph behaviour).
- The **self-reference glossary**: the tool → Arai (never "we"); a single constraint →
  rule, the system/command → guardrails; the AI actor → the model; the human → you.
- The **before/after exemplar table** as the register's illustrative ground truth
  (e.g. init footer "Done. Arai is now watching your rules…" → "Arai is enforcing this
  project's rules (Claude Code and Grok TUI)."; error "Failed to read settings.json:
  {e}" → "Could not read settings.json: {e}"; deny fallback "Arai: blocking rule
  matched" → "Arai: a rule blocked this action.").

Bounded edit surfaces (the contract is for the copy/voice retune across these):
src/hooks.rs (deny reason / additionalContext prose / internal-error text), src/init.rs
(init/deinit flow), user-visible error messages across src/ (Result<_,String> + eprintln!
that reach a user — NOT internal-only), src/main.rs + src/stats.rs command-output prose
(status/why/audit/stats headers, "no rules"/hint lines), README.md (light intro/tagline
pass only), and the new committed docs/voice.md (AC1).

HARD constraints (numbered list in the contract):
1. Behaviour UNCHANGED — wording only; no logic change.
2. JSON protocol keys and structural/enum values preserved (only human-readable string
   *content* changes).
3. #83 colour + #84 glyph behaviour preserved (no colour/glyph code touched).
4. ZERO new dependency.
5. Glossary consistency — each concept named by exactly one term across all retuned strings.
6. Tests in LOCKSTEP — every test asserting a changed human string is updated in the same
   change (tests/hooks_safety.rs, the brand/glyph verifier files, integration tests).

Verifier obligations (encode as the AC checks + a checklist):
- Review voice consistency against the committed docs/voice.md.
- Run the FULL gate: cargo fmt --all -- --check + cargo clippy --all-targets (no new
  warnings) + cargo test — all pass.
- Lockstep grep: confirm no test still asserts old wording.
- Diff confirms: JSON keys/enums unchanged; no colour/glyph code changed; Cargo.toml
  unchanged (no new dep).

Language-neutral re: implementation, but DO include the before/after exemplars (they are
the deliverable's substance). Emit `re_raise.yaml` instead ONLY for a genuine design gap
(there should be none). Final message: short confirmation of the file written.
