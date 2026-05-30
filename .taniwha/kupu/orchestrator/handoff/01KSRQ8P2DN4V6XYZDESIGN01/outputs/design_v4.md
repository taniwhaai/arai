---
version: 4
parent_brief_version: 6
tier: small_multi_module
---

# extends-pinning-signing-tiering

## Structural tier

**Selected:** small_multi_module

**Justification:** The brief explicitly identifies three separable concerns that
sit at different points in the system and have meaningfully different failure
semantics, dependencies, and ownership boundaries. (a) Directive-grammar
tokenisation is a pure string-classification concern with a single failure mode
(unknown token → fail-closed skip) and no external dependencies. (b) Fetch-time
verification combines pinning (a content-hash comparison reusing the existing
`sha2` helper) and signing (a detached ed25519 sidecar check that introduces the
one new crate dependency and a trust-file schema migration) — its failure
semantics are "obtain content, then reject if integrity/authenticity checks
fail," all confined to the network/cache boundary inside `resolve()`/`fetch()`.
(c) Tiering is the one concern the brief calls out as **cross-cutting**: it
originates at the directive but must be carried as provenance from `resolve()`
through `parser.rs` (rule extraction) into `store.rs` and `guardrails.rs`
(ranking, shadowing, override) — a flow that does not exist today because
`resolve()` currently inlines markdown and loses all upstream provenance.
Collapsing all three into one module would conflate a pure tokeniser, a
network-side verifier, and a pipeline-spanning provenance carrier whose changes
land in four different source files; that is exactly the layer-coupling the
small-multi-module tier exists to avoid. The concerns are not numerous or
independently-deployable enough to warrant full decomposition (no nested
subsystems; one composition node — `resolve()` — wires them).

**Module count:** 3, plus one composition node.

- **directive-tokenisation** — separate from the others because the brief gives
  it its own frozen grammar and its own fail-closed contract ("a token starting
  with `@` is the pin; `tier=<enum>` is the tier; any other trailing token, or an
  unknown `tier=` value, is a malformed directive → fail-closed"). It is pure
  computation with no side effects and no dependency on the network or store; it
  can be specified and verified entirely from string inputs (AC1 bare-directive
  regression, AC3 malformed-pin token, AC12 unknown token/value). Folding it into
  the verifier would couple a pure parser to the network boundary.

- **fetch-verification** — separate because it owns everything that happens at
  the moment content is obtained: the sha256 pin comparison on **both** the
  fresh-remote and stale-cache paths (AC2, AC3 hash-mismatch, AC4), the ed25519
  detached-sidecar check gated on a configured public key (AC5, AC6, AC7), and the
  trust-file schema migration that the signing feature requires (AC8, plus the
  CLI surface AC13). It is the only module that introduces a new dependency
  (`ed25519-dalek`), touches the network/cache, and reads/writes the trust file.
  Its failure semantics ("skip + warn, preserve local content") are distinct from
  the tokeniser's ("skip the directive") and from tiering (which never skips, only
  reweights). This is the brief's "fetch-time verification" concern; pinning and
  signing are kept in one module rather than two because they share the same
  trigger (content obtained in `fetch()`), the same fail-closed outcome, and the
  same encoding convention (hex), and neither is independently useful without the
  other being present in the same code path.

- **tier-provenance** — separate because it is the cross-cutting concern the brief
  flags explicitly. It does not own a single file; it owns a *property* (tier +
  upstream-source label) that must survive the inline step in `resolve()` and be
  read by the parser, store, and guardrails. Its three behaviours (strict =
  no-shadow, advisory = deprioritise, override = local-drop) land in different
  files (`parser.rs`, `store.rs`, `guardrails.rs`) but serve one coherent
  capability: honouring the declared trust tier of an upstream block. Keeping it
  as one module is correct under the "capability not layer" rule — splitting it
  per-file would produce three layer-modules (parse / store / rank) that all exist
  to serve the same property. It covers AC9, AC10, AC11.

## Open questions

The brief is structurally complete; the two questions below are peripheral
(they affect a parser/validation knob and a directive sub-syntax, not the module
decomposition). They are deferred to the user rather than decided silently.

1. **`tier=override` local-drop syntax (AC11).** The brief says "the local
   instruction file may drop specific upstream rules by id/triple" but does not
   define the surface syntax by which a local instruction file names the upstream
   rule to drop (e.g. a dedicated directive line, an annotation on a local rule, or
   a triple-equality match). The module boundary is unaffected — the drop logic
   lives in `tier-provenance` regardless — but the exact local-file grammar is a
   user-facing behaviour decision. `requires_user_decision: true`.

2. **Duplicate-token handling in a directive (peripheral to AC12).** The frozen
   grammar lists at most one `@<pin>` and one `tier=` token, but does not state
   what happens if a directive carries two pin tokens or two `tier=` tokens. The
   safe reading consistent with the brief's fail-closed posture is to treat a
   repeated classified token as a malformed directive (skip + warn), but this is
   not stated. `requires_user_decision: true` (low-stakes; the fail-closed default
   is the natural resolution if the user does not care).

## Purpose

This slice extends Arai's `arai:extends` shared-policy mechanism so an operator
can pin an upstream policy to an exact content hash, require that upstream content
carry a valid ed25519 signature from a trusted key, and declare how strongly an
upstream block's rules bind relative to local rules — while guaranteeing that a
bare directive and a legacy trust file behave byte-identically to today.

## External boundaries

- **instruction-file directive line**: inbound, text — the `# arai:extends <url>
  [@<sha256-hex>] [tier=strict|advisory|override]` line (and its `<!-- ... -->`
  HTML-comment form) read from the top of a discovered instruction file.
- **upstream policy content**: inbound, byte stream — the markdown body fetched
  over HTTPS (or read from the 24h on-disk cache) at the pinned/extended URL;
  size-capped and HTTPS-only posture unchanged.
- **detached signature sidecar**: inbound, byte stream — the `<url>.sig` resource
  (hex-encoded ed25519 detached signature over the upstream content bytes),
  fetched only when a public key is configured for that URL.
- **trust file (`trusted_extends.toml`)**: inbound and outbound, on-disk file —
  read at resolve time to determine which URLs are trusted and which carry a
  configured public key; written by `arai trust --add` to record new entries
  (now optionally with a `--pubkey`).
- **on-disk cache + sha256 sidecar**: inbound and outbound — the existing 24h
  content cache and its `.sha256` integrity sidecar; reused unchanged.
- **stderr**: outbound, text — fail-closed warnings emitted when a directive is
  malformed, a pin mismatches, or a required signature is missing/invalid.
- **CLI (`arai trust`)**: inbound, command invocation — the `trust --add <url>
  [--pubkey <hex>]` subcommand and the trust-listing output that now surfaces
  keyed/pinned status.
- **inlined instruction content + provenance**: outbound, in-process — the
  resolved instruction text (upstream block inlined ahead of local content) plus
  the per-block tier + source provenance carried to the rule pipeline.

## Modules

### directive-tokenisation

**Responsible for:** Parsing a single `arai:extends` directive line into its
components — the URL, an optional content pin, and an optional tier — by
whitespace-token shape classification, and signalling a malformed directive when
any trailing token cannot be classified.

**Not responsible for:** Fetching content, verifying pins or signatures, reading
the trust file, or applying the tier to any rule. It produces a parsed value or a
"malformed" signal; it performs no I/O.

**Inputs:**
- `directive_line`: the raw directive text after the `arai:extends` marker, in
  either the `#` form or the `<!-- ... -->` HTML-comment form. Required.

**Outputs:**
- A parsed directive value carrying: the URL token (required); an optional pin
  (the 64-character lowercase-normalised hex following an `@`, with the `@`
  stripped); and an optional tier enum value. See **ParsedDirective**.
- Alternatively, a "malformed directive" signal (the directive is to be skipped),
  carrying enough context for the caller to emit a stderr warning naming the
  offending token.

**Side effects:**
- None. Pure computation.

**Error semantics (fail-closed paths named):**
- **Unknown trailing token** — any whitespace-separated token after the URL that
  does not start with `@` and is not of the form `tier=<value>` → malformed signal
  (caller skips the directive, warns, preserves local content). [AC12]
- **Unknown `tier=` value** — a `tier=` token whose value is not one of
  `strict|advisory|override` → malformed signal. [AC12]
- **Malformed pin token** — an `@`-prefixed token whose remainder is not valid
  full 64-char hex → malformed signal (the actual content comparison is the
  verifier's job, but a syntactically invalid pin is rejected here). [AC3]
- **No trailing tokens** — the bare `# arai:extends <url>` case is the success
  path with no pin and no tier; it must classify identically to today, with no new
  branch taken. [AC1]
- A `@` appearing inside the URL (userinfo) is *not* a separate token because it
  carries no surrounding whitespace; it stays within the URL token and is never
  classified as a pin.

**Behavioural guarantees:**
- For an input with no trailing tokens, the produced value is observably identical
  to the legacy parse (URL only, no pin, no tier) — the backward-compatibility
  invariant for the directive path. [AC1]
- Token classification is by shape and is order-independent among the optional
  tokens (a directive may list `@<pin>` then `tier=`, or the reverse).
- Absent tier defaults to the *peer* tier (see **Tier**); absence is not an error.
- Deterministic and side-effect free; safe to call repeatedly.

**Dependencies:** None (no other module in this system).

**Files changed:** `src/extends.rs` (extends `parse_directive` / the directive
token classification; the existing `extract_urls` continues to surface URLs).

---

### fetch-verification

**Responsible for:** Establishing that obtained upstream content is both the
pinned content (sha256 match) and, when a public key is configured for the URL,
authentic (valid ed25519 detached signature over the content bytes) — on every
path by which content is obtained — and owning the backward-compatible trust-file
schema that records per-URL public keys.

**Not responsible for:** Classifying directive tokens (that is
directive-tokenisation's input to this module), or interpreting the tier
(tier-provenance). It decides only *whether the upstream block is admitted*; once
admitted, what happens to its rules is tiering's concern.

**Inputs:**
- `url`: the upstream URL from the parsed directive. Required.
- `pin`: the optional normalised content hash from the parsed directive. Optional.
- `obtained_content`: the upstream bytes returned by the existing fetch/cache
  path — required to be checked on **both** the fresh-remote and the stale-cache
  fallback paths.
- `trust_entry`: the resolved trust-file entry for `url`, carrying whether the URL
  is trusted and an optional configured public key. See **TrustEntry**.

**Outputs:**
- An admit/reject decision for the upstream block. On admit, the verified content
  is passed forward for inlining. On reject, the caller preserves local content.
- For the CLI surface: a recorded trust entry (on `trust --add`) and a listing
  that surfaces which URLs are keyed.

**Side effects:**
- **Signature sidecar fetch**: when (and only when) a public key is configured for
  the URL, fetch the sibling `<url>.sig` resource using the existing fetch posture
  (HTTPS-only, size cap, cache). No sidecar fetch occurs when no key is
  configured. [AC7]
- **Trust-file read**: deserialise `trusted_extends.toml` at resolve time.
- **Trust-file write**: `arai trust --add <url> [--pubkey <hex>]` appends/updates
  an entry, preserving the file's existing entries. [AC13]
- **stderr warning** on any reject path.

**Error semantics (fail-closed paths named):**
- **Pin mismatch (fresh path)** — computed `content_sha256_hex(obtained_content)`
  does not equal `pin` (compared case-insensitively) → reject (skip + warn),
  preserve local content. [AC3]
- **Pin mismatch (stale-cache fallback path)** — the identical comparison runs on
  the stale-cache fallback content, not only the fresh fetch → reject if it
  mismatches. [AC4]
- **Missing signature** — a public key is configured but the `<url>.sig` resource
  cannot be obtained → reject (skip + warn). [AC6]
- **Invalid signature** — the `<url>.sig` content fails ed25519 verification
  against the configured public key over the content bytes → reject (skip + warn).
  [AC6]
- **Malformed configured key** — a configured public key that is not valid hex /
  not a valid ed25519 public key → reject (skip + warn): a key that cannot be used
  to verify must fail closed, never silently downgrade to no-check.
- **No configured key** — signature verification is skipped entirely; only the pin
  (if present) is checked. This is the unchanged path. [AC7]
- **No pin and no key** — neither check runs; behaviour is byte-identical to
  today. [AC1, AC7]
- **Untrusted URL** — unchanged existing behaviour (the URL is not in the trust
  file → not fetched/inlined).

**Behavioural guarantees:**
- Pin and signature checks both run *after* content is obtained and *before*
  inlining; admission requires every configured check to pass (pin AND, if a key
  is configured, signature). A configured check is never bypassed by the presence
  or absence of the other.
- The pin comparison reuses the existing `content_sha256_hex` helper; pinning
  introduces no new dependency.
- The trust-file deserialiser accepts both the legacy `trusted: Vec<String>`
  (list-of-strings) form and the new richer per-entry form, mapping a legacy
  string entry to a **TrustEntry** with no configured key — guaranteeing legacy
  files parse and behave identically to today. [AC8]
- Key and signature encodings are hex, matching the existing sha256 hex
  convention.
- `ed25519-dalek` is the only new crate dependency.

**Dependencies:**
- directive-tokenisation (consumes its parsed `url` and `pin`).
- Existing in-file helpers: `fetch`, `read_cache_verified`, `content_sha256_hex`,
  the trust-list loader.

**Files changed:** `src/extends.rs` (pin comparison on both content paths;
sidecar fetch + ed25519 verify; `TrustFile`/`TrustEntry` schema migration and
deserialiser); `src/main.rs` (`trust --add --pubkey` flag and keyed/pinned trust
listing); `Cargo.toml` (add `ed25519-dalek`).

---

### tier-provenance

**Responsible for:** Carrying an admitted upstream block's declared tier and
upstream-source label through the inline step into rule provenance, and honouring
that tier when rules are extracted, stored, and ranked — strict (an upstream
rule's subject is never shadowed by a same-subject local rule), advisory (the
rule is deprioritised by the ranker), and override (the local instruction file may
drop a named upstream rule).

**Not responsible for:** Deciding whether the upstream block is admitted (that is
fetch-verification) or classifying the tier token (directive-tokenisation). It
acts only on already-admitted, already-parsed content.

**Inputs:**
- `admitted_upstream_content`: the verified upstream markdown for a block.
- `tier`: the **Tier** value for that block (peer when the directive omitted it).
- `source_label`: an identifier for the upstream origin (the URL) so a rule's
  provenance can record that it came from an extended block and at what tier.
- `local_content`: the local instruction content inlined after the upstream block.
- For override: the local file's expression of which upstream rule(s) to drop
  (surface syntax is **Open question 1**).

**Outputs:**
- Extracted rules whose provenance records carry `tier` and `source_label`
  alongside the existing layer/line/severity fields.
- Ranking/shadowing behaviour that reflects the tier at match time.

**Side effects:**
- **Rule-store writes**: tier + source provenance is persisted with the rule
  triple (in `store.rs`) so the guardrail matcher can read it without re-resolving.

**Error semantics (fail-closed paths named):**
- An *admitted* block whose tier is `peer` (the default) produces rules whose
  ranking and shadowing behaviour is identical to today — the no-annotation path
  must not change. [AC1]
- A reference in a `tier=override` local file to an upstream rule that does not
  exist is a no-op drop, not an error (dropping something absent has no effect);
  it must not skip or fail the block. (Subject to Open question 1's chosen
  syntax.)
- Tiering never rejects a block — admission is already decided by
  fetch-verification. A tier value reaching this module is always one of the four
  valid **Tier** variants (the tokeniser already fail-closed on unknown values).

**Behavioural guarantees:**
- **strict**: when an upstream rule and a local rule share the same subject, the
  upstream rule is retained and is not shadowed/overridden by the local rule.
  [AC9]
- **advisory**: an advisory-tier rule's confidence/severity is lowered so the
  ranker deprioritises it relative to peer/strict rules. [AC10]
- **override**: the local instruction file may drop specific upstream rules,
  identified by id/triple; non-dropped upstream rules are retained. [AC11]
- **peer** (default, tier absent): no shadowing change, no deprioritisation, no
  drop — identical to current behaviour. [AC1]
- Provenance (tier + source) flows one-way from `resolve()` output → parser
  extraction → store → guardrails; it is never re-derived downstream, so the four
  files agree on a single representation.

**Dependencies:**
- fetch-verification (consumes admitted content and the resolved tier for a
  block).
- directive-tokenisation (the tier value originates there).

**Files changed:** `src/extends.rs` (`resolve()` emits tier + source alongside
inlined content rather than discarding provenance); `src/parser.rs` (rule
extraction records tier + source in provenance); `src/store.rs` (persist tier +
source with the triple; ranking/shadowing read path); `src/guardrails.rs`
(apply strict no-shadow, advisory deprioritisation, and override-drop at match
time).

## Data shapes

### ParsedDirective
The structured result of tokenising one directive line. Carries: `url` (the
required URL token, unmodified, including any in-URL `@` userinfo); `pin` (an
optional content hash — the normalised lowercase 64-char hex following a
whitespace-delimited `@` token, with the `@` removed); and `tier` (an optional
**Tier**; absence means peer). A directive that fails classification produces a
distinct "malformed" outcome instead of this value, carrying the offending token
text for the warning.

### Tier
An enumeration with four variants: **Strict**, **Advisory**, **Override**, and
**Peer**. `Peer` is the default applied when the directive omits `tier=`. Only
`strict`, `advisory`, and `override` are spellable in a directive; `peer` is the
implicit absence value and is not a writable token. Any other `tier=` value is a
malformed directive (handled by directive-tokenisation, never reaching the
**Tier** type).

### TrustEntry
The per-URL trust record. Carries: `url` (the trusted URL) and `pubkey` (an
optional hex-encoded ed25519 public key). A `pubkey` of "none" means no signature
check is performed for that URL.

### TrustFile (migrated schema)
The deserialised `trusted_extends.toml`. Backward-compatible across two on-disk
forms: the **legacy** form (`trusted` = list of URL strings) and the **new** form
(a list of per-entry records, each a URL with an optional `pubkey`). Both forms
deserialise into the same in-memory representation — a collection of
**TrustEntry** — with each legacy string mapping to a **TrustEntry** whose
`pubkey` is absent. The serialiser used by `arai trust --add` writes the new form;
a file already in legacy form is read without rewrite unless an entry is added.
This dual-form deserialiser is the trust-file half of the backward-compatibility
invariant. [AC8]

### RuleProvenance (extended)
The existing per-rule provenance (already carrying source line, layer, and
severity) is extended with two additive fields: `tier` (a **Tier**) and
`source_label` (the upstream URL the rule's block came from, or absent for purely
local rules). Older stored rules without these fields read as `tier = peer` and
no source label, preserving current ranking behaviour. This is how tier
provenance flows from `resolve()` to `guardrails.rs`.

## Composition

There is one composition node: **`resolve()`** in `src/extends.rs`, which already
orchestrates directive parsing → fetch → inline today. Its extended wiring:

- **instruction-file directive line → inlined content + provenance**:
  1. `resolve()` calls **directive-tokenisation** on each directive line. A
     malformed result is skipped with a stderr warning; local content is
     preserved (no fetch). [AC1, AC3, AC12]
  2. For a well-formed directive, `resolve()` resolves the **TrustEntry** for the
     URL and invokes the existing fetch/cache path to obtain content.
  3. `resolve()` invokes **fetch-verification** with the obtained content, the
     optional pin, and the trust entry. On both the fresh and stale-cache paths,
     the pin (if present) is checked; if the trust entry carries a key, the
     `<url>.sig` sidecar is fetched and verified. A reject skips the block with a
     warning and preserves local content. [AC2–AC8]
  4. On admit, `resolve()` hands the verified content, the **Tier**, and the
     source label to **tier-provenance**, which inlines the upstream block ahead
     of local content and tags the resulting rules' provenance. Downstream,
     `parser.rs` extraction and `store.rs` persistence carry that provenance, and
     `guardrails.rs` honours it at match time. [AC9–AC11]

- **CLI (`arai trust`)**: `src/main.rs` invokes fetch-verification's trust-file
  read/write surface for `--add [--pubkey]` and for the keyed/pinned listing.
  [AC13]

### Backward-compatibility invariant (hard)

A bare `# arai:extends <url>` directive combined with a legacy list-of-strings
`trusted_extends.toml` MUST resolve byte-identically to today. Two mechanisms
guarantee this, one per data path:

- **Tokeniser path:** directive-tokenisation, given a line with no trailing tokens,
  produces a value with no pin and no tier (tier defaults to peer). No pin → no
  hash comparison beyond today's existing cache sidecar; peer tier → no shadowing,
  deprioritisation, or drop. The new branches are entered only when a pin or
  `tier=` token is present, so the bare path takes exactly the code path it does
  today. [AC1]
- **Trust-file path:** the **TrustFile** deserialiser accepts the legacy
  `trusted: Vec<String>` form and maps each string to a **TrustEntry** with no
  `pubkey`. No `pubkey` → fetch-verification performs no signature check and
  fetches no `<url>.sig` sidecar, so a legacy trust file drives exactly today's
  behaviour. [AC7, AC8]

### Full-gate requirement (hard, carried from the #122 CI lesson)

The leaf-implementation and verifier for every module above MUST run the
project's full local gate before declaring work complete — not `cargo test`
alone. Specifically: `cargo fmt --all -- --check` (formatting clean),
`cargo clippy --all-targets` (no new warnings), AND `cargo test` (all tests,
including the existing extends suite and the new integration test, pass). The
prior cycle's CI failed because only tests were run locally; rustfmt and clippy
are gated in CI and must be satisfied locally first. [AC14]

## Acceptance-criteria assignment

Each criterion is owned by exactly one module; none unassigned.

| AC   | Owning module           | Note |
|------|-------------------------|------|
| AC1  | directive-tokenisation  | Bare directive + legacy trust file → byte-identical (tokeniser half; trust half is fetch-verification's deserialiser, but the regression test for the un-annotated path is driven by the tokeniser producing a no-pin/no-tier value). |
| AC2  | fetch-verification      | Matching `@<sha256>` → inlined. |
| AC3  | fetch-verification      | Pin mismatch → skip + warn. (Malformed *pin token* syntax is rejected in directive-tokenisation; the content-comparison reject is here.) |
| AC4  | fetch-verification      | Pin checked on stale-cache fallback path. |
| AC5  | fetch-verification      | Configured pubkey + valid `.sig` → inlined. |
| AC6  | fetch-verification      | Configured pubkey + missing/invalid `.sig` → skip + warn. |
| AC7  | fetch-verification      | No configured pubkey → no signature check. |
| AC8  | fetch-verification      | Legacy list-of-strings trust file still parses/works. |
| AC9  | tier-provenance         | `tier=strict` upstream rule not shadowed by same-subject local rule. |
| AC10 | tier-provenance         | `tier=advisory` rule deprioritised by ranker. |
| AC11 | tier-provenance         | `tier=override` local file drops a named upstream rule. |
| AC12 | directive-tokenisation  | Unknown `tier=` value or unknown trailing token → fail-closed skip + warn. |
| AC13 | fetch-verification      | `trust --add --pubkey` records key; listing surfaces keyed/pinned status. |
| AC14 | (all three + verifier)  | Full gate green (fmt + clippy + test); a process requirement on every leaf and the verifier, not a single module's behaviour. |

## Files touched

| File             | Change                                                                                          | Owning module                          |
|------------------|-------------------------------------------------------------------------------------------------|----------------------------------------|
| `src/extends.rs` | Directive token classification (pin/tier); pin comparison on fresh + stale paths; ed25519 sidecar fetch + verify; `TrustFile`/`TrustEntry` dual-form deserialiser; `resolve()` emits tier + source provenance. | directive-tokenisation, fetch-verification, tier-provenance |
| `src/parser.rs`  | Rule extraction records `tier` + `source_label` in provenance.                                  | tier-provenance                        |
| `src/store.rs`   | Persist tier + source with the rule triple; ranking/shadowing read path.                        | tier-provenance                        |
| `src/guardrails.rs` | Apply strict no-shadow, advisory deprioritisation, override-drop at match time.              | tier-provenance                        |
| `src/main.rs`    | `trust --add <url> --pubkey <hex>` flag; trust listing surfaces keyed/pinned status.            | fetch-verification                     |
| `Cargo.toml`     | Add `ed25519-dalek` (the only new dependency; leaf selects the exact version).                  | fetch-verification                     |
| `tests/`         | New cross-module integration test seeding the cache directly (no real network), exercising pin, signing, and tiering end-to-end through `resolve()`. Existing `#[cfg(test)]` blocks in `src/extends.rs` extended for unit coverage. | all three |

## Dependency declaration

- **`ed25519-dalek`** — the only new crate dependency permitted, owned by
  fetch-verification, used for detached-signature verification over upstream
  content bytes. Crate name only; the leaf selects the exact version.
- **`sha2`** — already a project dependency; the pin comparison reuses the
  existing `content_sha256_hex` helper. No new dependency for pinning.
- No other new dependencies. The project's `Result<T, String>` error convention
  and rustfmt/clippy defaults apply throughout.

## Out of scope

- Semver / policy-registry resolution (the issue's heavier pinning alternative).
- Sigstore, cosign, or minisign (the ed25519 detached sidecar was chosen instead).
- Recursive extends and multi-level trust (single-level, non-recursive posture
  unchanged).
- Changing the 512 KB content cap, the 24h cache TTL, or the HTTPS-only posture.
- Any change to the un-annotated directive path or to legacy trust-file behaviour
  beyond making both deserialise into the migrated in-memory shape.
- Real-network testing — the new integration test seeds the cache directly, as the
  existing extends tests do.
- Custom error-enum machinery — the project's `Result<T, String>` convention is
  retained for new fallible functions.
