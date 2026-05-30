# Manifest: fetch-verification

## Responsibility

Given obtained upstream content, an optional pin, and a resolved trust entry, determines whether the content is admitted (all configured integrity and authenticity checks pass) or rejected (any configured check fails), and owns the backward-compatible trust-file schema that records per-URL public keys.

## Not responsible for

Classifying directive tokens or parsing the pin/tier from the directive line (that is directive-tokenisation's concern). Deciding what to do with admitted content's tier (that is tier-provenance's concern). Deciding whether a URL is trusted at all (unchanged existing logic — untrusted URLs are not fetched; this module operates only on content that has already been obtained for a trusted URL).

---

## Settled decisions (BINDING — do not reopen or hedge)

**AC11_drop_syntax:** tier=override drop syntax is triple-equality (implicit). No new directive keyword. This decision does not affect fetch-verification; it is stated here for completeness as a binding constraint on the slice.

**AC12_duplicate_token:** Duplicate directive tokens = fail-closed. This constraint is satisfied upstream by directive-tokenisation before any input reaches this module.

---

## Inputs

- **url** (`non-empty string`, required): The upstream URL extracted from a `ParsedDirective`. Used to look up the `TrustEntry` in the trust file and to construct the sidecar URL (`<url>.sig`).
- **pin** (`optional 64-character lowercase hex string`, optional): The content-pin from the `ParsedDirective`. Absent when the directive carried no `@<pin>` token.
- **obtained_content** (`byte sequence`, required): The upstream content bytes returned by the existing fetch/cache path. The pin and signature checks both operate on this value. Must be supplied on both the fresh-remote path and the stale-cache fallback path — there is no path through which content is admitted without passing through this module.
- **trust_entry** (`TrustEntry`, required): The trust-file entry for `url`. Carries `url` and optional `pubkey`. This value is resolved by the caller (resolve-composition) from the trust file before invoking this module.

## Outputs

- **admit decision** (`boolean` or equivalent two-variant type): When all configured checks pass (or when no checks are configured), the upstream block is admitted. The caller proceeds to inline the verified content. When any configured check fails, the upstream block is rejected; the caller preserves local content and emits a stderr warning.
- **trust-file mutation** (side effect, not a return value): When invoked via the `arai trust --add` CLI path, a new or updated `TrustEntry` is written to the trust file. See Side effects.

## Side effects

- **Signature sidecar fetch**: When (and only when) `trust_entry.pubkey` is present and valid, this module fetches the resource at `<url>.sig` using the existing fetch posture (HTTPS-only, same 512 KB size cap, same cache). No sidecar fetch occurs when `trust_entry.pubkey` is absent. [AC7]
- **Trust-file read**: The `TrustFile` is deserialised from `trusted_extends.toml` at resolve time (by the caller before invoking this module, or by this module — the contract requires the result is the same). The deserialisaton must accept both on-disk forms (see Behavioural guarantees).
- **Trust-file write** (CLI path only): `arai trust --add <url> [--pubkey <hex>]` appends a new entry or updates an existing entry in the trust file, writing in the new per-entry TOML form. Existing entries are preserved. [AC13]
- **stderr warning**: A human-readable warning is written to stderr on every reject path. The warning names the URL and the reason for rejection (pin mismatch, missing sidecar, invalid signature, malformed key).

## Error semantics

- **Pin mismatch (fresh-remote path)** — the sha256 hex of `obtained_content` does not equal `pin` (case-insensitive comparison after normalising both to lowercase): signalled as reject; caller preserves local content and emits a stderr warning. [AC3]
- **Pin mismatch (stale-cache fallback path)** — the identical pin comparison runs when content is obtained from the on-disk stale cache, not only when content is freshly fetched; a mismatch on the stale path is treated identically to a mismatch on the fresh path: reject + warn. [AC4]
- **Missing signature** — `trust_entry.pubkey` is present, but the `<url>.sig` resource cannot be obtained (network failure, 404, or any other retrieval error): signalled as reject; caller preserves local content and emits a stderr warning. [AC6]
- **Invalid signature** — `trust_entry.pubkey` is present, the sidecar was obtained, but the sidecar content fails ed25519 verification against the configured public key over the exact bytes of `obtained_content`: signalled as reject + warn. [AC6]
- **Malformed configured key** — `trust_entry.pubkey` is present but its value is not valid hex or does not decode to a valid 32-byte ed25519 public key: signalled as reject + warn. This is fail-closed: a key that cannot be used to verify must never cause a silent downgrade to no-check.
- **No configured key (`trust_entry.pubkey` absent)** — signature verification is skipped entirely; no sidecar fetch is attempted; only the pin (if present) is checked. [AC7]
- **No pin and no configured key** — neither pin check nor signature check is performed; the admit decision is always true for a trusted URL. Behaviour is byte-identical to today. [AC1 — fetch-verification half, AC7]
- **Untrusted URL** — unchanged existing behaviour; the URL is not in the trust file and is not fetched or inlined. This module does not alter the existing trust-list gate.

The module never partially admits: the admit decision is a single boolean. A passing pin check combined with a failing signature check is still a reject.

## Behavioural guarantees

- **Both checks must pass when both are configured:** Pin and signature checks both run after content is obtained and before admission. Admission requires every configured check to pass (pin AND signature, if a key is configured). A passing pin check does not bypass a failing signature check, and a present sidecar does not bypass a missing pin.
- **Backward-compatibility invariant (trust-file half):** The `TrustFile` dual-form deserialiser accepts the legacy on-disk form where the `trusted` key holds a plain list of URL strings (no per-entry objects), and maps each string to a `TrustEntry` with `pubkey` absent. A `TrustEntry` with `pubkey` absent drives no signature check and no sidecar fetch, so a legacy trust file produces behaviour byte-identical to today. No rewrite of the legacy file occurs on read. [AC8]
- **Pin comparison reuses existing helper:** The pin check uses the same `content_sha256_hex` function already present in the project. No new dependency is introduced for pinning.
- **Signing uses `ed25519-dalek` exclusively:** ed25519 signature verification is performed using the `ed25519-dalek` crate. This is the only new crate dependency introduced by this slice. The leaf implementor selects the exact version.
- **Hex encoding convention:** All binary values (hashes, public keys, signatures) are handled as lowercase hex strings. Pin comparison is case-insensitive (both sides normalised to lowercase before comparison).
- **Sidecar URL construction is deterministic:** The sidecar URL is always `<url>.sig` with no other transformation. No query-string manipulation, no path rewriting.
- **Idempotency:** Given identical inputs, the admit decision is identical. No state accumulates between calls on the verification path. (The trust-file write path is not idempotent in general but is append/update semantics: writing the same entry twice is safe and leaves the file in the same logical state.)
- **Concurrent invocation safety (verification path):** The verification path (admit/reject) holds no shared mutable state and is safe under concurrent invocation. The trust-file write path is not required to be safe under concurrent invocation (single-user CLI).
- **No new failure modes for the un-annotated path:** When `pin` is absent and `trust_entry.pubkey` is absent, this module's logic adds zero new failure modes relative to today.

## Dependencies

- **directive-tokenisation contract**: This module consumes the `url` and `pin` fields of a `ParsedDirective` produced by directive-tokenisation. It does not invoke the tokenisation function directly; it receives the already-parsed values from the caller (resolve-composition).
- **Existing in-project helpers**: `content_sha256_hex` (pin comparison), existing fetch function (fresh-remote content and sidecar), existing cache-read function (stale-cache content). These are used as-is; their contracts are not altered by this slice.

## Referenced data shapes

All defined in the shared vocabulary file:
- `ParsedDirective` — the source of `url` and `pin` inputs (consumed via caller)
- `TrustEntry` — the per-URL trust record
- `TrustFile` — the in-memory representation of the trust file, with dual-form deserialiser

## Acceptance criteria

**AC2 — Matching pin admits content:**
Given `obtained_content` whose sha256 hex equals `pin` (case-insensitive), and `trust_entry.pubkey` absent, when fetch-verification is invoked, then the admit decision is true and no warning is emitted.

**AC3 — Pin mismatch rejects content:**
Given `obtained_content` whose sha256 hex does NOT equal `pin`, when fetch-verification is invoked, then the admit decision is false, local content is preserved, and a stderr warning naming the URL is emitted.

**AC4 — Pin checked on stale-cache fallback path:**
Given a scenario where the fresh-remote fetch fails and content is obtained from the stale on-disk cache, and `pin` is present, when fetch-verification is invoked with the cache-sourced `obtained_content`, then the pin comparison runs on that content and a mismatch produces a reject identical to the fresh-path reject. (Test setup: seed the on-disk cache directly with content whose sha256 does not equal the pin; invoke resolve() with network unavailable or bypassed; observe reject + warn.)

**AC5 — Configured pubkey + valid signature admits content:**
Given `trust_entry.pubkey` set to a valid ed25519 public key hex, and a `<url>.sig` sidecar resource containing a valid ed25519 signature over `obtained_content` bytes under the corresponding private key, when fetch-verification is invoked (with or without a matching pin), then the admit decision is true.

**AC6 — Configured pubkey + missing or invalid signature rejects content:**
Sub-case A (missing): Given `trust_entry.pubkey` set, but the `<url>.sig` resource cannot be obtained, when fetch-verification is invoked, then the admit decision is false and a stderr warning is emitted.
Sub-case B (invalid sig): Given `trust_entry.pubkey` set and a `<url>.sig` resource present but containing a signature that fails verification (wrong key, wrong content, corrupted), when fetch-verification is invoked, then the admit decision is false and a stderr warning is emitted.

**AC7 — No configured pubkey means no signature check:**
Given `trust_entry.pubkey` absent, when fetch-verification is invoked, then no sidecar fetch is attempted and no signature check is performed. The admit decision depends only on the pin check (if pin is present) or is true (if pin is also absent).

**AC8 — Legacy trust file parses and works:**
Given a `trusted_extends.toml` file in the legacy form (`trusted = ["https://example.com/policy.md"]`), when the `TrustFile` is deserialised, then it produces a collection containing one `TrustEntry` with `url = "https://example.com/policy.md"` and `pubkey` absent. No error is signalled. The resulting entry drives the same behaviour as today (no signature check, no sidecar fetch).

**AC13 — `arai trust --add --pubkey` records key; listing surfaces keyed status:**
Sub-case A: Given `arai trust --add https://example.com/policy.md --pubkey <valid-64-hex>`, when the command is run, then the trust file contains an entry for that URL with the supplied pubkey value, and subsequent runs of fetch-verification for that URL perform a signature check.
Sub-case B: Given a trust file containing a URL with a pubkey, when the trust listing command is run, then the output identifies that URL as having a configured key (and distinguishes it from URLs without a key).

**AC14 — Full gate:**
The full local gate passes: `cargo fmt --all --check` reports no formatting issues, `cargo clippy --all-targets` reports no new warnings, and `cargo test` passes all tests including the existing extends suite and any new unit/integration tests for this module.
