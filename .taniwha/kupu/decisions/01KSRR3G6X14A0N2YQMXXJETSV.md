---
id: 01KSRR3G6X14A0N2YQMXXJETSV
kind: re_raise_resolution
triggered_by: user_input_received:design_doc_approval (event 01KSRR1SHGKN6KGTKFXMX2EM6P)
affects:
  - kind: design_doc
    id: v4
    from_version: 4
    to_version: 4
  - kind: contract
    id: tier-provenance
    from_version: null
    to_version: 1
  - kind: contract
    id: directive-tokenisation
    from_version: null
    to_version: 1
recorded_at: "2026-05-29T02:13:12.000Z"
---

# Design open questions resolved: AC11 drop syntax + duplicate directive token handling

Design v4 carried two open questions that required user decisions before contract
derivation could proceed. Both were presented to the user at design approval time
(event 01KSRR1SHGKN6KGTKFXMX2EM6P) and are now settled.

## Decision 1 — AC11 tier=override drop syntax

**Question:** For `tier=override`, how should a local instruction file identify
which upstream rules to drop?

**Options presented:**
- Explicit `arai:drop` directive keyword
- Triple-equality (implicit): SPO match
- Annotation syntax on a local rule

**Chosen:** Triple-equality (implicit)

**Meaning:** A local rule whose subject-predicate-object exactly matches an
upstream rule implicitly drops that upstream rule when `tier=override` is in
effect for the upstream block. No new directive keyword is introduced. The
mechanism relies on exact SPO triple match at rule-extraction time.

**Impact on tier-provenance contract:** The override-drop behaviour in the
tier-provenance module is specified as: when processing local content after an
admitted `tier=override` upstream block, if a local rule's extracted SPO triple
is identical to any upstream rule's SPO triple from that block, the upstream rule
is dropped (suppressed from the rule store / ranking). A reference to an SPO that
does not match any upstream rule is a no-op (not an error). This shape avoids
introducing any new on-disk directive syntax and limits the feature to the
tier-provenance module only.

## Decision 2 — Duplicate directive token handling (AC12 family)

**Question:** If a directive carries two `@<pin>` tokens or two `tier=` tokens,
what should happen?

**Options presented:**
- Fail-closed (skip + warn)
- First-wins (use the first occurrence)
- Last-wins (use the last occurrence)

**Chosen:** Fail-closed (skip + warn)

**Meaning:** A directive carrying more than one `@<pin>` token OR more than one
`tier=` token is malformed. The directive is skipped (local content is preserved)
and a stderr warning is emitted naming the duplicate token. This is consistent
with the brief's fail-closed posture and with directive-tokenisation's existing
error semantics for unknown tokens.

**Impact on directive-tokenisation contract:** The token-classification loop must
track whether a `@`-prefixed token or a `tier=` token has already been seen in
the current directive. On encountering a second such token, the module returns the
malformed signal rather than overwriting the first value. This rule joins the
existing fail-closed paths at AC12.

## Rationale

Both decisions are self-consistent with the design's stated fail-closed posture
and with the existing module decomposition (directive-tokenisation and
tier-provenance remain clearly bounded; no new module is needed). The triple-
equality drop syntax is the simplest mechanism that satisfies the brief without
adding a new directive keyword, and the user's note confirmed this interpretation.

These decisions are binding on the contract-derivation subagent for this slice.
