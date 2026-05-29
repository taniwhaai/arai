# Brief — Issue #29: Shared-policy versioning, signing & tiering for `arai:extends`

## Source
GitHub issue #29 ("Shared policy: versioning, signing, tiering — gated on user
demand"). The user has now requested all three gaps in one cycle, and chose the
**ed25519 detached sidecar** mechanism for signing.

## Background
`arai:extends <url>` (v0.2.1) is a minimal shared-policy mechanism in
`src/extends.rs`: HTTPS-only, explicit trust list (`trusted_extends.toml`),
512 KB cap, 24h on-disk cache with sidecar `.sha256` integrity, single-level,
non-recursive. Directives are parsed at the top of an instruction file and the
upstream markdown is inlined ahead of local content by `resolve()` (called from
`src/discovery.rs`). Existing reusable pieces: `content_sha256_hex()`,
`read_cache_verified()`, `fetch()`, `parse_directive()`, `extract_urls()`,
`resolve()`, `TrustFile { trusted: Vec<String> }`. `sha2` is already a dependency.

## Goal
Extend the directive grammar and trust file with optional **pinning**, **signing**,
and **tiering** — all strictly backward-compatible: a bare `# arai:extends <url>`
and an existing `trusted_extends.toml` must behave byte-identically to today.
This backward-compat guarantee is the central constraint (the issue's explicit
worry about freezing the wrong shape).

## Frozen shapes

### Directive grammar (both `<!-- ... -->` and `#` forms)
```
# arai:extends <url> [@<sha256-hex>] [tier=strict|advisory|override]
```
Tokens after `<url>` are whitespace-separated and classified by shape: a token
starting with `@` is the pin; `tier=<enum>` is the tier; any other trailing token,
or an unknown `tier=` value, is a malformed directive → **fail-closed** (skip the
directive, emit a stderr warning, preserve local content). A `@` inside the URL
(userinfo) is safe — it has no whitespace, so it stays within the URL token.

### Gap 1 — Pinning (`@<sha256-hex>`, full 64-char lowercase hex)
After content is obtained in `fetch()` — on BOTH the fresh-remote and the
stale-cache fallback paths — compute `content_sha256_hex(&content)` and compare
to the pin (case-insensitive). Mismatch or malformed pin → reject (skip + warn).
Reuse the existing helper; no new dependency.

### Gap 2 — Signing (ed25519 detached sidecar; fail-closed only when configured)
- `TrustFile` grows an optional per-URL ed25519 public key. The schema MUST remain
  backward-compatible with the current `trusted: Vec<String>` form — deserialize
  both the legacy list-of-strings and a new richer per-entry form.
- When a URL has a configured public key: fetch the sibling `<url>.sig` (a detached
  ed25519 signature over the upstream content bytes) and verify with
  `ed25519-dalek`. Missing or invalid signature → **skip + warn** (fail-closed).
- When no key is configured for a URL: NO signature check (unchanged behaviour).
- Key + signature encoding: hex (to match the existing sha256 hex convention).
- New dependency: `ed25519-dalek` (the only new dependency permitted).
- `arai trust --add <url>` gains optional `--pubkey <hex>`; trust listing surfaces
  which URLs are keyed.

### Gap 3 — Tiering (`tier=strict|advisory|override`; enum; default = peer when absent)
Propagate the tier from the directive through the inlined upstream block into the
extracted rules' provenance, so the rule pipeline can honour it:
- **strict**: an upstream rule's subject is never shadowed/overridden by a local
  rule with the same subject.
- **advisory**: lower the rule's confidence/severity so the ranker deprioritises it.
- **override**: the local instruction file may drop specific upstream rules by
  id/triple.
This is the cross-cutting concern: today `resolve()` inlines markdown and rule
provenance is lost, so tiering requires carrying tier+source from `resolve()` →
`parser.rs` (rule extraction/provenance) → `store.rs` / `guardrails.rs` (ranking,
shadowing, override). The design phase fixes the exact module boundaries.

## Acceptance criteria
- AC1: Bare `# arai:extends <url>` (no suffix) + existing trust file behave exactly
  as today (regression — all existing extends tests pass).
- AC2: `@<sha256>` matching upstream content → upstream inlined.
- AC3: `@<sha256>` not matching → skip + stderr warn, local content preserved;
  malformed pin token → same fail-closed skip.
- AC4: Pin verified on the stale-cache fallback path, not only fresh fetch.
- AC5: URL with configured ed25519 pubkey + valid `<url>.sig` → inlined.
- AC6: URL with configured pubkey but missing/invalid `.sig` → skip + warn (fail-closed).
- AC7: URL with no configured pubkey → no signature check (backward compatible).
- AC8: Legacy `trusted_extends.toml` (list-of-strings) still parses and works.
- AC9: `tier=strict` upstream rule not shadowed by a same-subject local rule.
- AC10: `tier=advisory` upstream rule deprioritised by the ranker.
- AC11: `tier=override` — local file can drop a named upstream rule.
- AC12: Unknown `tier=` value or unknown trailing token → fail-closed skip + warn.
- AC13: `arai trust --add <url> --pubkey <hex>` records the key; trust listing
  surfaces pinned/keyed status.
- AC14: Full gate green — `cargo fmt --all -- --check`, `cargo clippy --all-targets`
  (no new warnings), and `cargo test` all pass.

## Scope / files
- `src/extends.rs` (primary), `src/parser.rs`, `src/store.rs`, `src/guardrails.rs`
  (tiering), `src/main.rs` (`trust --pubkey` flag + listing), `Cargo.toml`
  (ed25519-dalek). Tests in `src/extends.rs` `#[cfg(test)]` + a new integration
  test under `tests/` (seed the cache directly as existing extends tests do — no
  real network).

## Process requirement (carried from the #122 cycle)
The leaf-implementation and verifier MUST run the project's FULL gate from
project_context — `cargo fmt --all`, `cargo clippy --all-targets`, AND
`cargo test` — not just `cargo test`. CI gates on rustfmt and the prior cycle's CI
failed because only tests were run locally.

## Out of scope
- Semver / policy-registry resolution (the issue's heavier pinning alternative).
- Sigstore/cosign/minisign (ed25519 sidecar was chosen instead).
- Recursive extends, multi-level trust, changing the 512 KB cap / 24h TTL / HTTPS-only posture.
- Any change to the un-annotated directive path or legacy trust-file behaviour.