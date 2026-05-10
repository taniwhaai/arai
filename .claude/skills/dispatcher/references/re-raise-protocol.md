# Re-Raise Protocol

A shared structured-error format used across the Taniwha compartmentalised-coding skills. When an agent encounters a problem it cannot solve from within its bounded context, it does not guess and it does not silently work around the issue — it emits a re-raise. Re-raises flow upward through the composition tree until they reach an agent with the authority and context to resolve them.

## Why this exists

In a compartmentalised system, agents operate with deliberately limited context. A leaf implementor sees only its own contract. A composer sees only the two contracts it is wiring together. This is a feature: it keeps context lean, prevents hallucinated coupling, and forces contracts to be complete. But it also means that when something is genuinely wrong — a contract is under-specified, two modules cannot compose, an upstream assumption was violated — the agent encountering the problem usually does not have the authority to fix it. The re-raise is how that agent surfaces the problem precisely enough that the right upstream agent can act on it without having to re-derive what went wrong.

The principle: **errors flow toward authority, not toward whoever happens to encounter them.**

## Format

A re-raise is a structured object with these fields:

```yaml
re_raise:
  origin: <module-or-composition-id>
  category: <see categories below>
  suspected_source: contract_a | contract_b | pairing | parent_contract | self
  failed_clause:
    contract: <which contract>
    location: <field, type, behaviour, or section reference>
    expectation: <what the contract specifies or implies>
    observation: <what was actually encountered>
  context_excerpt: <minimal quote from the contract showing the problem>
  attempted_resolutions: <list of approaches considered and why each was rejected>
  proposed_fix: <optional — only if the origin agent has a concrete suggestion>
  blocking: <true|false — whether work can continue without resolution>
```

Every field except `proposed_fix` is required. Agents should not invent a `proposed_fix` if none is obvious; an empty field is more useful than a guess.

## Categories

Pick the most specific category that fits. If a problem genuinely spans categories, pick the one that points most clearly at the responsible upstream agent.

- **`under_specified`** — the contract does not say enough to implement or compose unambiguously. Common with error semantics, ordering guarantees, idempotency, and edge-case behaviour.
- **`internally_inconsistent`** — the contract contradicts itself (e.g. claims a function is pure but also describes side effects).
- **`mutually_incompatible`** — two contracts being composed cannot be reconciled (e.g. A produces a stream, B requires a complete value).
- **`type_mismatch`** — narrower than `mutually_incompatible`; specifically a type-shape conflict that no reasonable adapter resolves.
- **`assumption_violated`** — an implementation produced something its own contract said it would not, discovered at composition time.
- **`out_of_scope`** — the work requested would require touching something beyond the agent's bounded context. The agent must not expand scope unilaterally; it re-raises instead.
- **`ambiguous_intent`** — the contract is internally consistent and complete on the surface, but admits multiple reasonable implementations that would behave differently in practice. The agent re-raises rather than picking one silently.

## Suspected source

This field tells the upstream agent where to look first. It is a hypothesis, not an accusation — the receiving agent may determine the real source is elsewhere.

- **`contract_a`** / **`contract_b`** — one of the two child contracts the current agent is working with.
- **`pairing`** — both child contracts are individually fine, but they were paired incorrectly by the parent.
- **`parent_contract`** — the contract this agent was asked to satisfy is itself the problem.
- **`self`** — the agent has identified an error in its own previous output (used when re-entering work after another agent's re-raise).

## Routing

Re-raises do not travel arbitrarily. They go to the agent one level up — the agent that authored the contract or chose the pairing the re-raise is challenging. That agent then either:

1. **Resolves locally** — amends a contract clause, re-pairs modules, or clarifies intent — and re-dispatches the affected child work.
2. **Re-raises further** — if the problem's true source is above the current level, the agent emits its own re-raise upward, referencing the original.

A re-raise must never be silently swallowed. If an agent cannot resolve and cannot re-raise (e.g. it is the root), the re-raise becomes a human-facing question.

## Anti-patterns

- **Working around the problem.** If a leaf agent cannot satisfy its contract as written, it must re-raise. Silently producing something that *almost* matches the contract poisons the rest of the tree.
- **Over-broad re-raises.** "The contract is bad" is not actionable. Name the clause, quote the language, state what was expected and what was observed.
- **Re-raise cascades for trivia.** If the agent can resolve the question from its own context without expanding scope, it should. Re-raises are for genuine boundary problems.
- **Speculative `proposed_fix`.** If the origin agent does not actually know how to fix the issue, leaving the field empty gives the upstream agent a cleaner slate.

## Example

```yaml
re_raise:
  origin: leaf:user-invitation-sender
  category: under_specified
  suspected_source: parent_contract
  failed_clause:
    contract: user-invitation-sender
    location: behaviour.on_duplicate_invitation
    expectation: contract states "invitations are idempotent"
    observation: contract does not specify whether a second call within the deduplication window returns the original invitation token, a fresh token, or an error
  context_excerpt: "Invitations are idempotent. Repeated calls with the same email within 24 hours should not produce duplicate side effects."
  attempted_resolutions:
    - "Returning the original token: matches 'no duplicate side effects' but the contract does not promise token stability to the caller, which would be a hidden API guarantee."
    - "Returning a fresh token: arguably violates idempotency from the caller's perspective."
    - "Returning an error: contradicts 'idempotent'."
  proposed_fix: ""
  blocking: true
```
