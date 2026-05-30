# Manifest: resolve-composition

## Responsibility

Wires directive-tokenisation, fetch-verification, and tier-provenance in sequence within the existing `resolve()` function so that a directive line flows through tokenisation, then fetch and verification, then provenance tagging — and enforces that the two backward-compatibility invariants hold end-to-end for any input path through the pipeline.

## Not responsible for

Implementing the logic of any of the three composed modules. This composition node owns only the wiring order, the data handoffs between modules, and the end-to-end backward-compatibility guarantee. Error handling (skip + warn on each reject path) is specified here as the caller's obligation for each module's failure outcome; the underlying error is produced by the respective module.

---

## Settled decisions (BINDING — do not reopen or hedge)

**AC11_drop_syntax:** tier=override drop syntax is triple-equality (implicit). A local rule whose SPO triple exactly matches an upstream rule's SPO triple implicitly drops that upstream rule when tier=override is in effect. No new directive keyword. [AC11]

**AC12_duplicate_token:** Duplicate directive tokens (two @<pin> or two tier=) = fail-closed (skip + warn). Handled by directive-tokenisation before any value reaches fetch-verification or tier-provenance. [AC12]

---

## Inputs

- **instruction file content** (`string`, required): The full text of a discovered instruction file, which may contain one or more `arai:extends` directive lines (in `#` or `<!-- -->` form) at the top. Existing input to `resolve()`; unchanged.
- **trust file (`trusted_extends.toml`)** (`TrustFile`, required): Deserialised at the start of resolution. Used to look up a `TrustEntry` for each directive URL. Deserialisaton must use the dual-form deserialiser (legacy + new form) as specified in the fetch-verification and vocabulary contracts.
- **on-disk cache** (ambient, existing): The existing 24-hour on-disk content cache. Used by the fetch path on both the fresh-remote and stale-cache fallback paths. Not a direct input to `resolve()` but used by the fetch helper it calls.

## Outputs

- **resolved instruction content** (`string`): The final merged instruction text — verified upstream block(s) inlined ahead of local content, with each admitted upstream block's content having its rules tagged with tier and source provenance. This is the unchanged output type of `resolve()`; the content is now provenance-tagged for downstream processing.
- **rule provenance** (implicit, in store): Rule triples extracted from admitted upstream blocks carry `tier` and `source_label` fields in their `RuleProvenance` records, persisted to the rule store. This is not a separate return value; it is a side effect of the tier-provenance module invoked during composition.

## Side effects

- All side effects are those of the composed modules: stderr warnings on skip paths, sidecar fetches when a pubkey is configured, trust-file reads and writes (trust --add path), rule-store writes with tier provenance. No additional side effects are introduced by the composition itself.

## Wiring sequence (per directive line)

The following ordered steps describe how `resolve()` processes each `arai:extends` directive line in the instruction file:

**Step 1 — Tokenise:**
`resolve()` calls directive-tokenisation on the raw directive line (with comment delimiters stripped if present).
- If the result is `MalformedDirective`: `resolve()` emits a stderr warning naming the offending token and the reason, skips this directive, and preserves local content. Processing continues with the next directive line.
- If the result is `ParsedDirective`: proceed to step 2.

**Step 2 — Resolve trust entry and obtain content:**
`resolve()` looks up the `TrustEntry` for `ParsedDirective.url` in the `TrustFile`. If the URL is not trusted (no entry), the existing not-trusted behaviour applies unchanged. If trusted, `resolve()` invokes the existing fetch/cache path to obtain content bytes.

**Step 3 — Verify:**
`resolve()` invokes fetch-verification with the obtained content, `ParsedDirective.pin`, and the resolved `TrustEntry`.
- If the result is reject: `resolve()` emits a stderr warning, skips this upstream block, and preserves local content. Processing continues with the next directive line.
- If the result is admit: proceed to step 4.

**Step 4 — Tag provenance and inline:**
`resolve()` passes the admitted content, `ParsedDirective.tier` (defaulting to Peer when absent), and `ParsedDirective.url` as source_label to tier-provenance. Tier-provenance extracts rules with the correct `RuleProvenance`, writes them to the store, and returns (or causes) the upstream block inlined ahead of local content.

**Step 5 — Downstream pipeline:**
The inlined + provenance-tagged content passes to `parser.rs` rule extraction and then to `store.rs` persistence and `guardrails.rs` matching, which honour the tier provenance as specified in the tier-provenance contract.

## Error semantics

- **MalformedDirective from tokenisation**: `resolve()` emits a stderr warning containing `offending_token` and `reason`, skips the directive, preserves local content. No error is propagated to the caller of `resolve()` — the overall resolution succeeds with reduced upstream content.
- **Reject from fetch-verification**: `resolve()` emits a stderr warning naming the URL and the verification failure, skips the upstream block, preserves local content. No error is propagated to the caller of `resolve()`.
- **Error from trust-file deserialisaton**: This is a fatal error for the resolution; `resolve()` returns an error to its caller (signalled as an error value per the project's existing fallible-function convention). The trust file being unreadable is not silently ignored.
- **Error from sidecar fetch (within fetch-verification)**: Handled by fetch-verification as a reject; `resolve()` sees a reject decision, not a raw error.
- All per-directive failures are non-fatal to the overall `resolve()` call. A file with N directives where K fail produces a result with (N-K) admitted upstream blocks, all local content preserved, and K stderr warnings.

## Behavioural guarantees

### Backward-compatibility invariant (HARD — both halves required)

A bare `arai:extends <url>` directive combined with a legacy list-of-strings `trusted_extends.toml` MUST produce an output byte-identical to the pre-slice `resolve()` output for the same inputs. Two mechanisms guarantee this end-to-end:

**Tokeniser half:** directive-tokenisation, given a line with no trailing tokens, produces a `ParsedDirective` with `pin` absent and `tier` absent. This is the AC1 guarantee from the directive-tokenisation contract. With no pin, fetch-verification performs no hash comparison beyond the existing cache sidecar check; with tier absent (Peer), tier-provenance adds no new behaviour.

**Trust-file half:** the `TrustFile` dual-form deserialiser maps legacy string entries to `TrustEntry` with `pubkey` absent. With no pubkey, fetch-verification performs no signature check and fetches no sidecar. The combined result is that neither the new pin-comparison branch nor the new signature-check branch is entered, and the resolution output is byte-identical to today.

Neither half of this invariant is optional. Both must hold for the invariant to be satisfied. The cross-module integration test (see Acceptance criteria) must verify the end-to-end invariant, not only each half in isolation.

- **Ordering:** Step 1 (tokenise) always precedes Step 3 (verify). Step 3 always precedes Step 4 (provenance). Content is never inlined before verification completes.
- **Atomicity per directive:** Each directive is processed atomically from `resolve()`'s perspective — it is either fully admitted (Steps 1–4 all succeed) or fully skipped (any failure causes skip). There is no partial inline of a directive's content.
- **Idempotency:** Given the same instruction file and trust file, `resolve()` produces the same output. (Subject to network content changing between calls, which is outside the scope of this contract.)
- **Concurrent invocation safety:** `resolve()` is safe under concurrent invocation on the read/verify path. Store writes follow the existing store concurrency model.

## Dependencies

- **directive-tokenisation contract**: `resolve()` calls the tokenisation function specified in that contract. The calling convention is: pass the raw directive text; receive either `ParsedDirective` or `MalformedDirective`.
- **fetch-verification contract**: `resolve()` calls the verification function specified in that contract. The calling convention is: pass url, pin, obtained_content, trust_entry; receive admit/reject.
- **tier-provenance contract**: `resolve()` calls the provenance-tagging function specified in that contract. The calling convention is: pass admitted_upstream_content, tier, source_label, local_content.
- **TrustFile dual-form deserialiser** (part of fetch-verification): Called by `resolve()` at the start of resolution to load the trust file.

## Referenced data shapes

All defined in the shared vocabulary file:
- `ParsedDirective`
- `MalformedDirective`
- `Tier`
- `TrustEntry`
- `TrustFile`
- `RuleProvenance`

## Acceptance criteria

**Integration test requirement (cross-module, no real network):**
There must be a cross-module integration test in the `tests/` directory at the crate root that:
- Seeds the on-disk content cache directly (bypassing real network calls) with known content for a test URL.
- Exercises the full pipeline through `resolve()` for at least the following scenarios:
  1. Pin present and matching — admit path, upstream content inlined.
  2. Pin present and mismatching — reject path, local content preserved, stderr warning emitted.
  3. Configured pubkey + valid signature sidecar (seeded in the cache) — admit path.
  4. Configured pubkey + invalid/missing sidecar — reject path, warn.
  5. `tier=strict` — upstream rule not shadowed by same-subject local rule.
  6. `tier=advisory` — upstream rule present but deprioritised.
  7. `tier=override` with matching local SPO triple — upstream rule dropped.
  8. `tier=override` with no matching local SPO triple — upstream rule retained.
  9. Bare directive + legacy trust file — output byte-identical to pre-slice behaviour.
- Passes without any real network calls.

**End-to-end backward-compatibility (hard invariant):**
Given a bare `arai:extends <url>` directive and a legacy `trusted_extends.toml` in list-of-strings form, when `resolve()` is called, then:
- The output text is identical to what the pre-slice `resolve()` produces for the same inputs.
- No stderr warning is emitted.
- No sidecar fetch is attempted.
- No pin comparison is performed.

**Wiring order enforced:**
Given a directive that will ultimately be admitted, the integration test can observe (via tracing, test doubles, or structured output) that tokenisation precedes verification, and verification precedes provenance tagging, with no step skipped.

**Per-directive failure isolation:**
Given an instruction file containing two directives where the first has a pin mismatch (reject) and the second is a bare directive (admit), when `resolve()` is called, then the output includes the second directive's upstream content (inlined) and does not include the first directive's upstream content, and exactly one stderr warning is emitted (for the first directive).

**AC14 — Full gate:**
The full local gate passes: `cargo fmt --all --check` reports no formatting issues, `cargo clippy --all-targets` reports no new warnings, and `cargo test` passes all tests including the existing extends suite and the new cross-module integration test described above.
