# Implementation notes — tier-provenance

## AC satisfaction

### AC9 — strict tier: upstream rule not shadowed by same-subject local rule

**Mechanism**: In `match_guardrails`, after the score-based low-relevance
filter, strict-tier upstream rules are retained regardless of score.  The
filter `matched.retain(|(g, s)| ...)` exempts any rule where
`matches!(g.tier, Some(Tier::Strict))`.

**Test**: `ac9_strict_upstream_not_shadowed_by_local` and
`ac9_strict_not_suppressed_by_score_filter` — local rule scores 2+,
strict upstream scores lower (phrase mismatch or low overlap), yet the
strict upstream survives.

### AC10 — advisory tier: upstream rule deprioritised

**Mechanism**: Advisory rules have their effective score halved before
sorting (`score / 2`).  Additionally, the low-relevance filter protects
advisory rules from being dropped (they are marked as retain regardless of
score), so they always appear in the output but rank after peer/strict
matches of equal or higher score.

**Test**: `ac10_advisory_upstream_deprioritised_vs_local_peer` — local
scores 2, advisory base-scores 2 → halved to 1; local appears at
position 0, advisory at position 1 (after).

### AC11 — override tier: triple-equality implicit drop

**Mechanism**: Before the scoring filter, a set of all local (Peer)
rule SPO triples is built.  Any upstream override-tier rule whose
`(subject, predicate, object)` exactly matches a local SPO is removed
before matching.

- Sub-case A (`ac11a_*`): exact match → upstream dropped, local surfaced.
- Sub-case B (`ac11b_*`): no match → upstream retained normally.
- Sub-case C (`ac11c_*`): no upstream rules at all → local unaffected,
  no error.

No new directive keyword or annotation syntax is introduced (BINDING:
AC11_drop_syntax).

### AC1 (tier-provenance half) — Peer tier produces no behavioural change

**Mechanism**: When `tier` is `None` or `Some(Tier::Peer)`, none of the
three new code paths (override pre-filter, advisory halving, strict
retain-exemption) fire.  The pre-slice behaviour is preserved byte-for-byte.

**Test**: `ac1_peer_tier_absent_produces_no_behaviour_change` — all-None
tier guardrails match the same rules they would have before this slice.

### AC14 — full gate

- `cargo fmt --all --check`: clean (0 diff)
- `cargo clippy --all-targets`: 9 warnings (matches pre-slice baseline)
- `cargo test`: 386 tests pass (379 pre-slice + 7 new)

## Design notes

### Provenance embedding in resolve()

`resolve()` embeds `<!-- arai:extends-block url="..." tier="..." -->`
markers in the inlined output string.  The new `extract_rules_from_resolved()`
function in `parser.rs` parses these markers to apply per-block provenance.
This approach keeps `discovery.rs` callers unchanged (outside scope).

### Store migration

Migration v4 is additive (uses `add_column_if_missing`): existing rows
without `tier`/`source_label` columns read as NULL = Peer/no-label.  The
`guardrail_from_row` mapper treats NULL tier as `None` (Peer) so no
behaviour change on pre-v4 databases.

### Guardrail struct

Two new `Option`-typed fields added to `Guardrail` with
`#[serde(default, skip_serializing_if = "Option::is_none")]` so existing
serialised guardrails (from hooks, audit, scenarios) round-trip without
change.
