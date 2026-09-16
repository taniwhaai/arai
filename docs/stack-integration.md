# Arai in the Taniwha stack

Reviewed 2026-09-16 against Atlas `9ca6cc0` and Kete `0a98b8d`.
This document records existing boundaries and the public embedding contract;
it does not announce a completed Kete enrollment or policy-activation flow.

## Authority and responsibilities

| Component | Responsibility | Relationship to Arai |
| --- | --- | --- |
| Arai | Local rule extraction, deterministic matching and local evidence | Standalone open library and CLI; no required hosted service |
| Kete agent | Bound device identity, verified policy acceptance, operating leases, protected local state and evidence transport | Embeds Arai's matching code; owns the bound product's trust and availability policy |
| Kete control plane | Organization governance, policy distribution and evidence aggregation | Distributes policy ahead of evaluation; does not decide each local tool call |
| Codeworld | Versioned repository identity, history and provenance | Supplies derived ground through the wider stack; no required Arai service connection |
| Atlas | Versioned architecture decisions and repository topology | Design authority, not a runtime dependency |

Atlas [0005](https://github.com/taniwhaai/atlas/blob/9ca6cc0defdeea92b05ae0d617cc6ac2acc6f10d/decisions/0005-deployment-stack.md)
requires one enforcement codebase: Kete wraps Arai instead of maintaining a
second matcher. Accepted decisions
[0007](https://github.com/taniwhaai/atlas/blob/9ca6cc0defdeea92b05ae0d617cc6ac2acc6f10d/decisions/0007-local-trust-boundary.md)
and [0009](https://github.com/taniwhaai/atlas/blob/9ca6cc0defdeea92b05ae0d617cc6ac2acc6f10d/decisions/0009-offline-enforcement.md)
put synchronous enforcement inside the local, offline-capable trust boundary.
Kete outages must not change an accepted local policy decision. Expired or
invalid bound policy follows an explicit administrator-selected mode; signed
revocation, silence and ordinary network failure are distinct states.

Accepted [0008](https://github.com/taniwhaai/atlas/blob/9ca6cc0defdeea92b05ae0d617cc6ac2acc6f10d/decisions/0008-derived-ground-portability.md)
makes a derived substrate's versioned contract and byte-identical conformance
corpus authoritative. It does not require a Codeworld port, universal graph
schema or network lookup in Arai. Missing ground must be reported as missing
or stale, never fabricated.

## Public library composition

Kete's [composition spike](https://github.com/taniwhaai/kete/blob/0a98b8ddfdafae28d7f148a04076bb34e3a02380/agent/src/compose.rs)
exercises this pipeline through Arai's public library:

1. Parse caller-supplied text with `parser::extract_rules` or
   `extract_rules_with_provenance`.
2. Persist it with `Store::open` and `Store::upsert_file`. For new integrations,
   classify through `Store::classify_all_guardrails` before activation.
3. Match a parsed hook through `hooks::match_hook`.
4. Use `hooks::highest_severity` for the shared severity calculation.
5. Explicitly record the outcome with `audit::record_firing`.
6. Enumerate evidence with `audit::list_buckets`; copy `AuditBucket::jsonl_bytes`
   without reserializing it, and verify the chain with `audit::verify_chain`.

`Config::load_from(project)` resolves a project without changing process CWD.
It still reads the normal Arai environment and configuration. An embedder that
needs separate state custody should construct/configure `Config` explicitly
before opening the store or recording audit events. `Config::load_from` alone
does not isolate its audit output from the user's normal Arai directory.

`match_hook` reads local rules/session state and returns `HookMatch`; it does
not print a host response, append audit events, refresh policy or contact a
service. The caller owns host response formatting, timing and the operational
decision (including bound-product failure handling). `highest_severity` honors
classified severity and overrides, and retains predicate fallback for older
unclassified stores. Do not duplicate that calculation in a wrapper.

The [surface audit](https://github.com/taniwhaai/kete/blob/0a98b8ddfdafae28d7f148a04076bb34e3a02380/docs/design/ARAI-SURFACE-AUDIT.md)
was written against Arai `35189a5`, still the exact revision in Kete's Cargo
manifest at this review. Its CWD mutation and copied severity calculation can
be removed when Kete deliberately updates that pin. Exposing an API in Arai
does not automatically upgrade the downstream build. The compatibility test
in `tests/embedding_compat.rs` exercises both the original unclassified spike
and the shared APIs without requiring Kete's optional compose build.
The audit lock uses the established cross-platform
[`fs2::FileExt`](https://docs.rs/fs2/0.4.3/fs2/trait.FileExt.html) API rather than
the newer standard-library lock API, avoiding a Rust 1.89 requirement for this
change. The full current dependency graph has not been validated on Kete's
older 1.88 toolchain; downstream compiler compatibility still needs its own run.

## Policy acceptance and discovery

Kete's [SECURE-PIPE L5/L8](https://github.com/taniwhaai/kete/blob/0a98b8ddfdafae28d7f148a04076bb34e3a02380/docs/design/SECURE-PIPE.md)
owns signature, body hash, binding scope, version floor, expiry, trusted time
and atomic activation. It requires semantic validation of the exact verified
body through Arai before committing the active-policy pointer and version
floor. Activation must use those stored bytes, never re-fetch a policy URL.

Arai's parser accepts Markdown and returns extracted triples. That is a
parsing seam, not a signed-policy validator or a guarantee that arbitrary
Markdown encodes the intended policy. `upsert_file` commits one source's
replacement; it is not a transaction with Kete's keystore, version floors or
active-policy pointer. The bound agent must define semantic acceptance and
crash-safe activation around this seam before claiming the complete flow.

Use separate source identities and retain provenance for externally accepted
rules. Local instruction discovery/refresh must not delete wrapper-owned
policy simply because its source identity is not a file in the working tree.
Parsing a caller-supplied body must not resolve its `arai:extends` directive.
Provenance labels describe origin; they do not prove signature validity or
grant authority. The standalone `arai:extends` trust/hash mechanism is not
Kete enrollment, signed bundle verification or TLS certificate pinning.

## Evidence and transport

Arai's JSONL remains the canonical local event source. Kete's prepared upload
batches, credentials, retry cursors and protected security state belong in
Kete-owned storage. SECURE-PIPE requires persisting the exact batch bytes and
signed header before the first network attempt, with idempotent retries.
Audit and telemetry remain separate channels with separate consent scopes.

The existing audit writer is best-effort: `record_firing` returns no success
receipt. Empty chain-verification results alone do not prove that an expected
event was written. Bucket enumeration and byte reads are not an atomic snapshot
of an actively written log; a shipper must establish a complete consistent
prefix and persist the prepared batch before sending it. A hash chain detects
changes relative to retained evidence; it does not authenticate the author
against another process with the same local privileges. Device attribution
and server acceptance are additional Kete responsibilities.

Writers use a per-day OS lock and recover the predecessor from the complete
JSONL tail, rather than trusting the cached `.head` sidecar. Lock markers stay
in Arai's audit directory and are not transport checkpoints. A partial,
malformed or unchained legacy tail is preserved and is not extended: Arai
does not erase evidence or silently start a new chain inside the same bucket.
`verify_chain` reports incomplete records, including a missing final newline.
An upgrade encountering an unchained legacy day needs an explicit migration
or rotation policy if same-day recording is required, retaining the original
bytes and acknowledging the coverage boundary. The next UTC day naturally
starts a new bucket; it does not repair or authenticate prior evidence.

Kete's spec distinguishes implemented cryptographic operations, agent-side
protocol invariants and distributed guarantees. Its current agent primitives
must not be described as a completed, live end-to-end deployment.

## Historical HTTP-hook draft

`docs/design-http-hooks-kete-integration.md` is an unimplemented draft. Its
parallel per-call HTTP evaluator, bearer-token configuration and network
fail-open default do not define the current bound integration. They predate
the accepted local/offline enforcement boundary and the device-bound pipe.
Do not implement that draft as an implicit dependency of `match_hook`, or
transmit raw hook contents as a side effect of ordinary matching. A future
proposal that changes these boundaries needs its own architecture decision.

## Compatibility checks

Run `cargo test --test embedding_compat` with default features and
`cargo test --no-default-features --test embedding_compat` for the lean build.
The tests cover the real Kete spike, shared severity/override behavior,
explicit project roots, caller-supplied policy provenance and raw audit
transport. They use temporary local state and no service connection. Changes
to public structs and function signatures need downstream compatibility review;
changes to policy/evidence formats additionally need protocol conformance tests.
