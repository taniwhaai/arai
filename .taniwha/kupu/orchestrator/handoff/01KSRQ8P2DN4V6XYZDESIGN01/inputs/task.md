# Task — design-doc (Arai design v4, issue #29)

Produce design v4 for brief v6 (GitHub issue #29): extend `arai:extends` with
optional pinning (`@<sha256-hex>`), ed25519 detached-sidecar signing, and tiering
(`strict|advisory|override`).

Inputs (in this handoff's `inputs/`):
- `brief.md` — full spec with AC1–AC14 and scope list (canonical: kupu/brief/v6.md)
- `project_context.yaml` (redacted) — language, repo layout, conventions
- `design_v3.md` — prior design, STYLE/STRUCTURE reference ONLY; do not extend it

Output: `design_v4.md` in this handoff's `outputs/` (dispatcher places it at
kupu/design/v4.md).

Required content:
1. Front matter: `version: 4`, `parent_brief_version: 6`, `tier: <declared>`.
2. **Structural tier declaration + justification.** Three separable concerns:
   (a) directive-grammar tokenisation (parse `@<sha256>` + `tier=<enum>`; fail-closed
   on unknown tokens); (b) fetch-time verification (sha256 pin + ed25519 sig over
   `<url>.sig` via ed25519-dalek; trust-file schema migration legacy-list →
   per-entry-with-optional-pubkey); (c) tiering — cross-cutting from `resolve()` →
   `parser.rs` → `store.rs` → `guardrails.rs` (strict/advisory/override). Decide
   whether these collapse or warrant separate modules. Do NOT default to
   single_module without explicit reasoning.
3. **Modules** — per module: name + purpose; responsible-for / not-responsible-for;
   inputs/outputs (prose types, no Rust); side effects; behavioural guarantees;
   error semantics (name every fail-closed path); inter-module deps; files changed.
4. **AC assignment** — assign each of AC1–AC14 to exactly one module; none unassigned.
5. **Data shapes** (prose): extended directive token (url + optional pin + optional
   tier); extended TrustFile entry (url + optional pubkey, backward-compatible with
   legacy list-of-strings); tier enum (Strict/Advisory/Override/Peer + default
   handling); how tier provenance flows from resolve() output to guardrails.
6. **Files touched** — src/extends.rs, src/parser.rs, src/store.rs,
   src/guardrails.rs, src/main.rs, Cargo.toml, tests/ — change + owning module.
7. **Dependency declaration** — ed25519-dalek is the ONLY new permitted dep (crate
   name only; leaf picks exact version).
8. **Backward-compatibility invariant (hard):** bare `# arai:extends <url>` + legacy
   list-of-strings trust file → byte-identical to today. State how the tokeniser and
   the trust-file deserialiser each preserve this.
9. **Full-gate requirement (from #122 CI lesson):** leaf + verifier MUST run
   `cargo fmt --all -- --check`, `cargo clippy --all-targets` (no new warnings), AND
   `cargo test` — not just test. State this in the design.
10. **Out of scope** (from brief): semver/registry, Sigstore/cosign/minisign,
    recursive extends, multi-level trust, changing 512 KB / 24h / HTTPS-only,
    changing the un-annotated path or legacy trust behaviour.

Constraints: language-neutral (no Rust code); v4 is fresh (do not parent on v3);
if small_multi_module, describe the composition node's wiring at a high level; the
design must be complete enough for contract-derivation without revisiting the brief.

Emit `re_raise.yaml` instead only if the brief is genuinely ambiguous. Final
message: short confirmation of tier chosen + module list.
