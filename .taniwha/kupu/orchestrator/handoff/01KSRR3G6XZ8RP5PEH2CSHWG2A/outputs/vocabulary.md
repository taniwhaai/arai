---
schema_version: 1
slice: extends-pinning-signing-tiering
tier: small_multi_module
---

# Shared Vocabulary — extends-pinning-signing-tiering

Every data shape defined here is referenced by name from the per-module contracts. Implementors receive this file alongside their module's contract. Shapes are defined once; no module contract redefines them inline.

---

## Data shapes

```yaml
data_shapes:

  - name: ParsedDirective
    sharing: shared
    referenced_by: [directive-tokenisation, fetch-verification, tier-provenance, resolve-composition]
    description: |
      The structured result of successfully classifying one arai:extends directive line.
      Carries three fields:
        url   — the URL token extracted from the directive. Required. The raw string
                exactly as it appears in the directive, including any in-URL @ userinfo
                characters that are NOT preceded by surrounding whitespace. Not validated
                beyond being a non-empty string at this layer.
        pin   — Optional. When present, a 64-character lowercase-normalised hex string
                representing the expected sha256 content hash for the upstream resource.
                The leading @ sigil is stripped; what remains after normalisation is
                exactly 64 hex characters [0-9a-f]. Absent when the directive carries no
                @<pin> token.
        tier  — Optional. The declared Tier for this upstream block. Absent means the
                Peer tier applies (the default). Never carries an unknown value — the
                tokeniser fail-closes on unrecognised values before producing this shape.
      A directive that cannot be classified produces a MalformedDirective outcome
      instead of a ParsedDirective (see below). ParsedDirective is only produced on
      the success path.
    fields:
      - name: url
        type: non-empty string
      - name: pin
        type: optional 64-character lowercase hex string
      - name: tier
        type: optional Tier (see Tier shape; absent = Peer)

  - name: MalformedDirective
    sharing: shared
    referenced_by: [directive-tokenisation, resolve-composition]
    description: |
      The outcome produced by directive-tokenisation when a directive line cannot be
      classified. Carries:
        offending_token — the whitespace-separated token that caused the failure,
                          as a string, for inclusion in the stderr warning. Required.
        reason          — a short human-readable description of why classification
                          failed (e.g. "duplicate @pin token", "unknown tier value",
                          "malformed pin: not 64-char hex"), for the warning message.
                          Required.
      The caller (resolve-composition) uses this to emit a stderr warning and skip
      the directive; local content is preserved. This shape carries no state used
      downstream beyond the warning.
    fields:
      - name: offending_token
        type: string
      - name: reason
        type: string

  - name: Tier
    sharing: shared
    referenced_by: [directive-tokenisation, fetch-verification, tier-provenance, resolve-composition]
    description: |
      An enumeration of four variants representing how strongly an upstream block's
      rules bind relative to local rules.

      Variants:
        Strict   — An upstream rule whose subject matches a local rule's subject takes
                   precedence; the local rule does not shadow it. Spelled "strict" in a
                   directive.
        Advisory — An upstream rule is deprioritised by the ranker relative to
                   peer/strict rules. Spelled "advisory" in a directive.
        Override — The local instruction file may implicitly drop upstream rules by
                   triple-equality (see AC11 settled decision). Spelled "override" in a
                   directive.
        Peer     — No shadowing change, no deprioritisation, no implicit drop. This is
                   the default when a directive omits tier=. "peer" is NOT a writable
                   directive token; it is the absence value, applied implicitly.

      Only "strict", "advisory", and "override" are valid written values in a directive.
      Any other written value is a malformed directive.
    variants:
      - Strict
      - Advisory
      - Override
      - Peer

  - name: TrustEntry
    sharing: shared
    referenced_by: [fetch-verification, resolve-composition]
    description: |
      The per-URL trust record read from or written to the trust file.
      Carries:
        url    — the trusted URL string. Required.
        pubkey — Optional. A hex-encoded ed25519 public key string for this URL.
                 When absent, no signature check is performed for this URL.
                 When present, must be a valid hex encoding of a 32-byte ed25519
                 public key; a value that fails this validity check causes a
                 fail-closed reject (not a silent downgrade to no-check).
    fields:
      - name: url
        type: non-empty string
      - name: pubkey
        type: optional hex-encoded string (64 hex characters encoding 32 bytes)

  - name: TrustFile
    sharing: shared
    referenced_by: [fetch-verification, resolve-composition]
    description: |
      The in-memory representation of the trusted_extends.toml file. Wraps a
      collection of TrustEntry values.

      The deserialiser accepts two on-disk forms without requiring a migration step:
        Legacy form:  a TOML key "trusted" whose value is a list of URL strings.
                      Each string maps to a TrustEntry with url=<that string> and
                      pubkey=absent. Byte-for-byte equivalent behaviour to today.
        New form:     a TOML key "trusted" whose value is a list of TOML inline
                      tables, each carrying at minimum a "url" field and optionally
                      a "pubkey" field.
      Both forms deserialise into the same in-memory collection of TrustEntry values.
      The serialiser (used by arai trust --add) writes the new form. A file already
      in the legacy form is read without being rewritten unless an entry is added or
      updated via trust --add.

      This dual-form deserialiser is the trust-file half of the backward-compatibility
      invariant.
    fields:
      - name: entries
        type: ordered collection of TrustEntry

  - name: RuleProvenance
    sharing: shared
    referenced_by: [tier-provenance, resolve-composition]
    description: |
      The provenance record carried alongside each extracted rule triple. This is an
      EXTENSION of the existing provenance structure (source line, layer, severity).
      Two new fields are added additively:
        tier         — the Tier of the upstream block this rule came from. Absent for
                       purely local rules, which read as Peer tier. When reading older
                       stored rules that pre-date this field, the absence must be
                       interpreted as Peer to preserve current ranking behaviour.
        source_label — the URL of the upstream block this rule came from, as a string.
                       Absent for purely local rules. When absent, the rule is treated
                       as having no upstream origin.
      These two fields are set once at parse/extraction time and are never re-derived
      downstream. The rule store persists them; the guardrail matcher reads them.
      Older stored rules without these fields are valid and read as Peer/no-label.
    fields:
      - name: tier
        type: optional Tier (absent = Peer)
      - name: source_label
        type: optional string (the upstream URL; absent for local rules)
```

---

## External systems

```yaml
external_systems:

  - name: trusted_extends.toml (trust file)
    description: |
      On-disk TOML file recording which upstream URLs are trusted for policy
      extension and which carry a configured ed25519 public key. Read at resolve
      time; written by arai trust --add. The path follows the existing project
      configuration path convention.
    accessed_by: [fetch-verification]
    access_mode: read and write

  - name: upstream policy content (HTTPS resource)
    description: |
      The markdown body fetched over HTTPS at the URL named in an arai:extends
      directive. Fetched by the existing fetch/cache path (HTTPS-only, 512 KB cap,
      24h on-disk cache). Unchanged by this slice.
    accessed_by: [fetch-verification]
    access_mode: read-only

  - name: signature sidecar (HTTPS resource)
    description: |
      The detached ed25519 signature for an upstream policy resource. Located at
      <url>.sig. Fetched using the same fetch posture (HTTPS-only, size cap). Only
      fetched when a public key is configured for the URL in the trust file.
    accessed_by: [fetch-verification]
    access_mode: read-only

  - name: on-disk content cache
    description: |
      The existing 24-hour on-disk content cache and its .sha256 integrity sidecar.
      Reused unchanged. Both the fresh-remote and stale-cache paths pass through
      fetch-verification's pin check.
    accessed_by: [fetch-verification]
    access_mode: read and write (existing path; no new write behaviour)

  - name: stderr
    description: |
      Standard error stream. Receives human-readable warnings on every fail-closed
      reject path (malformed directive, pin mismatch, missing or invalid signature,
      malformed configured key). No structured output; text only.
    accessed_by: [directive-tokenisation (via caller), fetch-verification, resolve-composition]
    access_mode: write-only

  - name: rule store (SQLite)
    description: |
      The existing SQLite database (store.rs) that persists rule triples. The
      tier-provenance module extends stored rule records with tier and source_label
      fields. Older records without these fields remain valid.
    accessed_by: [tier-provenance]
    access_mode: read and write
```

---

## Cross-cutting concerns

```yaml
cross_cutting:

  - name: fail-closed posture
    description: |
      Any ambiguity or error in the new token-classification, verification, or
      provenance path resolves by skipping (not admitting) the upstream block and
      preserving local content. No new path silently degrades to a less secure mode.
      A configured check that cannot be completed (missing sidecar, malformed key,
      unknown token) is always treated as a failure, never as "no configured check".

  - name: error convention
    description: |
      All fallible functions in this project return either a success value or an
      error string. No custom error type is introduced by this slice. Error strings
      are human-readable warning text suitable for display on stderr. Implementors
      should follow the existing project convention for fallible functions as
      documented in project_context.yaml.

  - name: backward-compatibility invariant (hard)
    description: |
      A bare arai:extends <url> directive combined with a legacy list-of-strings
      trusted_extends.toml MUST produce behaviour byte-identical to today. Two
      mechanisms guarantee this (one per data path):
        Tokeniser path: a directive with no trailing tokens produces a ParsedDirective
          with pin=absent and tier=absent (Peer). No new branch is entered. [AC1]
        Trust-file path: the TrustFile dual-form deserialiser maps legacy string entries
          to TrustEntry with pubkey=absent. No pubkey means no signature check and no
          sidecar fetch. [AC7, AC8]
      Both halves are load-bearing. Neither may be weakened.

  - name: hex encoding convention
    description: |
      All binary values (content hashes, public keys, signatures) are encoded as
      lowercase hex strings. The pin comparison is case-insensitive (both sides
      normalised to lowercase before comparison). This matches the existing sha256
      hex convention in the project.

  - name: new external dependency
    description: |
      ed25519-dalek is the only new crate dependency introduced by this slice.
      It is owned by the fetch-verification module. No other new crate dependency
      is permitted. The leaf implementor selects the exact version.

  - name: full gate requirement (AC14)
    description: |
      Every leaf implementor and the verifier MUST pass the full local gate before
      declaring work complete: cargo fmt --all --check (formatting clean), then
      cargo clippy --all-targets (no new warnings), then cargo test (all tests pass,
      including existing extends suite and new integration tests). Running only
      cargo test is insufficient. This requirement appears as AC14 in every contract.
```

---

## Named error conditions

```yaml
errors:

  - name: MalformedDirectiveError
    sharing: shared
    referenced_by: [directive-tokenisation, resolve-composition]
    semantics: |
      Signalled when a directive line cannot be classified (unknown trailing token,
      unknown tier= value, malformed pin syntax, duplicate @pin token, or duplicate
      tier= token). The caller receives a MalformedDirective value (not this as an
      error type) so it can emit a warning and skip the directive.

  - name: PinMismatchError
    sharing: local
    referenced_by: [fetch-verification]
    semantics: |
      Internal to fetch-verification. The computed sha256 hex of obtained content
      does not equal the pin value from the ParsedDirective (case-insensitive
      comparison). Results in reject (skip + warn).

  - name: SignatureFailureError
    sharing: local
    referenced_by: [fetch-verification]
    semantics: |
      Internal to fetch-verification. Either the signature sidecar could not be
      obtained, or the sidecar content fails ed25519 verification against the
      configured public key. Results in reject (skip + warn).

  - name: MalformedKeyError
    sharing: local
    referenced_by: [fetch-verification]
    semantics: |
      Internal to fetch-verification. A configured pubkey value in a TrustEntry
      is not valid hex or does not decode to a valid ed25519 public key. Results
      in reject (skip + warn); never silently downgrades to no-check.
```
