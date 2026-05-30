# Manifest: tier-provenance

## Responsibility

Carries an admitted upstream block's declared tier and upstream-source label through the inline step into rule provenance, and enforces tier semantics when rules are extracted, stored, and ranked: strict blocks local shadowing of same-subject upstream rules, advisory causes deprioritisation by the ranker, and override causes implicit drop of upstream rules whose subject-predicate-object exactly matches a local rule.

## Not responsible for

Deciding whether an upstream block is admitted (that is fetch-verification). Classifying the tier token from the directive text (that is directive-tokenisation). This module acts only on content that has already been admitted and on a tier value that has already been validated.

---

## Settled decisions (BINDING — do not reopen or hedge)

**AC11_drop_syntax:** tier=override drop syntax is triple-equality (implicit). A local rule whose subject-predicate-object exactly matches an upstream rule implicitly drops that upstream rule when tier=override is in effect. No new directive keyword or annotation syntax. The comparison is exact equality of all three SPO fields (subject, predicate, object). This decision closes Open question 1 in the design document. No alternative drop syntax exists. [AC11]

**AC12_duplicate_token:** Duplicate directive tokens = fail-closed. This constraint is satisfied upstream by directive-tokenisation before any tier value reaches this module.

---

## Inputs

- **admitted_upstream_content** (`byte sequence or string`, required): The verified upstream markdown for one upstream block, as admitted by fetch-verification. This module operates on this content to extract rules.
- **tier** (`Tier`, required): The declared tier for this upstream block. Always one of the four valid `Tier` variants (Strict, Advisory, Override, Peer). The tokeniser has already fail-closed on unknown values; a tier value arriving here is guaranteed to be valid.
- **source_label** (`string`, required): An identifier for the upstream origin — the URL of the upstream block. Stored as provenance on every rule extracted from this block. Used by the ranker and guardrail matcher to identify upstream rules.
- **local_content** (`byte sequence or string`, required): The local instruction content that is inlined after the upstream block. Required for the override tier (to identify local rules for triple-equality comparison against upstream rules). Also required for strict tier (to detect same-subject local rules that must not shadow upstream rules).

## Outputs

- **Extracted rules with provenance** (written to the rule store, not returned as a direct value): Each rule triple extracted from the admitted upstream content has its `RuleProvenance` record extended with `tier` and `source_label`. Downstream, the guardrail matcher reads these provenance fields.
- **Ranking/shadowing behaviour**: At match time in the guardrail matcher, the tier provenance controls whether an upstream rule is suppressed, deprioritised, or retained relative to local rules.

There is no direct return value to the caller (resolve-composition) beyond triggering the store writes and establishing the provenance. The observable effect is in the rule store and in subsequent guardrail match output.

## Side effects

- **Rule-store writes** (`store.rs`): When upstream rules are extracted from `admitted_upstream_content`, their stored records include the new `tier` and `source_label` provenance fields. This is a write to the existing SQLite rule store. New columns or nullable fields are added additively — existing stored rows without these fields remain valid and are read as `tier = Peer` and `source_label` absent.
- **Shadowing suppression** (`parser.rs`, `store.rs`): For `tier=strict`, when a local rule shares the same subject as an upstream rule, the local rule is suppressed from being stored or matched as an override of the upstream rule. The exact mechanism (prevented write vs. filtered read) is the implementor's choice, but the observable result is that the upstream strict rule is not shadowed.
- **Deprioritisation** (`guardrails.rs`): For `tier=advisory`, advisory-tier rules are ranked lower than peer/strict rules at match time. The ranking mechanism (a confidence/severity adjustment or a sort key) is the implementor's choice, but the observable result is that advisory rules appear deprioritised in guardrail output relative to same-subject peer/strict rules.
- **Implicit drop** (`guardrails.rs`): For `tier=override`, upstream rules whose SPO triple exactly matches a local rule's SPO triple are dropped and not surfaced by the guardrail matcher. Rules that do not match any local triple are retained.

## Error semantics

- **Peer tier (default) — unchanged path:** An admitted block whose tier is `Peer` produces rules with ranking and shadowing behaviour identical to today. No new branch is entered, no new suppression or deprioritisation occurs. The provenance fields `tier = Peer` and `source_label` are recorded for observability but produce no behavioural change relative to the current un-annotated path. [AC1]
- **Reference to non-existent upstream rule in override context:** When `tier=override` is in effect and a local rule's SPO triple does not match any upstream rule's SPO triple, this is a no-op drop: no error is signalled, no warning is emitted, the block is not skipped. Dropping something absent has no effect. [AC11, BINDING: AC11_drop_syntax]
- **Invalid tier value arriving at this module:** This cannot happen if directive-tokenisation is correctly implemented (it fail-closes on unknown values). If a value outside the four valid variants does reach this module, the implementor must treat it as Peer (safest degradation) and emit a stderr warning. This is a defensive guard, not a primary contract.
- **Tiering never rejects a block:** Tier values only affect rule ranking, shadowing, and drop. A block that reaches this module has already been admitted by fetch-verification; tier-provenance cannot cause the block to be rejected.

## Behavioural guarantees

- **strict — no-shadow guarantee [AC9]:** When `tier=strict`, an upstream rule retains its match-time authority for any subject it covers. A local rule with the same subject does not shadow, override, or deprioritise the upstream strict rule. Both rules may be stored, but at match time the upstream strict rule is not suppressed by the local rule.
- **advisory — deprioritisation guarantee [AC10]:** When `tier=advisory`, an advisory-tier upstream rule is deprioritised by the ranker relative to peer and strict rules on the same subject. The rule is not dropped — it is still available — but it appears at lower priority in the guardrail match output.
- **override — triple-equality implicit drop [AC11, BINDING: AC11_drop_syntax]:** When `tier=override`, for each upstream rule, if there exists a local rule (extracted from `local_content`) whose subject, predicate, and object all exactly match the upstream rule's subject, predicate, and object (case-sensitive string equality on all three fields), then the upstream rule is dropped and not surfaced by the guardrail matcher. Upstream rules with no matching local triple are retained and surfaced normally. No new directive keyword or annotation syntax is used.
- **peer — no behavioural change [AC1]:** When `tier=Peer` (including the case where `tier` was absent and Peer was applied as the default), behaviour is identical to the pre-slice path: no shadowing suppression, no deprioritisation, no drop.
- **Provenance flows one-way:** The `tier` and `source_label` fields in `RuleProvenance` are set once at extraction time (here, or in the parser called from here) and are never re-derived downstream. The four affected source files (extends.rs, parser.rs, store.rs, guardrails.rs) share a single representation of these fields and a single read path.
- **Additive provenance fields:** The `tier` and `source_label` additions to `RuleProvenance` are additive. Existing stored rules without these fields are read as `tier = Peer` and `source_label` absent. The store schema migration (adding nullable columns) must not break reads of existing rows.
- **Idempotency of provenance writes:** Extracting and storing provenance for the same upstream block twice (e.g. on re-scan) produces the same stored state as doing it once.
- **Concurrent invocation safety (match path):** The guardrail match path reads provenance fields from the store; reads are safe under concurrent invocation. Store writes (extraction/re-scan) follow the existing store concurrency model, which is unchanged.
- **No new failure modes for the peer-tier path:** When tier is Peer and source_label is absent (i.e. a fully local rule), this module's logic adds zero new failure modes or observable behaviour changes relative to today.

## Dependencies

- **fetch-verification contract**: This module receives the admitted upstream content and the resolved `Tier` value that originated in a verified `ParsedDirective`. It does not re-verify content; it trusts that content reaching it has already passed fetch-verification.
- **directive-tokenisation contract**: The `Tier` value this module acts on was produced by directive-tokenisation. This module does not re-parse the directive; it receives an already-validated `Tier`.
- **Existing in-project modules (used, not changed in contract)**: `parser.rs` (rule extraction function, extended to accept and record provenance fields), `store.rs` (rule storage, extended to persist and read tier/source_label), `guardrails.rs` (guardrail matcher, extended to apply strict/advisory/override at match time).

## Referenced data shapes

All defined in the shared vocabulary file:
- `Tier` — the enumeration of tier variants
- `RuleProvenance` — the existing provenance record extended with `tier` and `source_label`

## Acceptance criteria

**AC9 — strict tier: upstream rule not shadowed by same-subject local rule:**
Given an upstream block with `tier=strict` containing a rule with subject S, and a local instruction file containing a rule with the same subject S (but different predicate or object), when rules are extracted and the guardrail matcher runs, then the upstream strict rule is surfaced and is not suppressed or overridden by the local rule. Both rules may appear; the upstream rule is not hidden.

**AC10 — advisory tier: upstream rule deprioritised:**
Given an upstream block with `tier=advisory` containing a rule with subject S, and a local instruction file containing a peer-tier rule with the same subject S, when the guardrail matcher runs and both rules match an input, then the local peer rule appears at higher priority than the advisory upstream rule in the match output. (The advisory rule is not absent — it is present but ranked lower.)

**AC11 — override tier: triple-equality implicit drop [BINDING: AC11_drop_syntax]:**
Sub-case A (match → drop): Given an upstream block with `tier=override` containing a rule with SPO triple (S, P, O), and a local instruction file containing a rule with exactly the same SPO triple (S, P, O), when the guardrail matcher runs, then the upstream rule is not surfaced. The local rule is surfaced.
Sub-case B (no match → retain): Given an upstream block with `tier=override` containing a rule with SPO triple (S, P, O), and a local instruction file containing NO rule with that exact SPO triple, when the guardrail matcher runs, then the upstream rule IS surfaced normally.
Sub-case C (no-op on absent): Given a local instruction file with a `tier=override` directive containing a local rule (S, P, O) where no upstream rule has that SPO triple, when extraction and matching run, then no error is signalled, no warning is emitted, and no upstream rule is dropped spuriously.

**AC1 (tier-provenance half) — Peer tier produces no behavioural change:**
Given an upstream block with `tier` absent (Peer), when rules are extracted and the guardrail matcher runs, then the match output is identical to what the pre-slice code would produce for the same upstream content with no tier annotation.

**AC14 — Full gate:**
The full local gate passes: `cargo fmt --all --check` reports no formatting issues, `cargo clippy --all-targets` reports no new warnings, and `cargo test` passes all tests including the existing extends suite and any new tests for tier-provenance behaviour in parser.rs, store.rs, and guardrails.rs.
