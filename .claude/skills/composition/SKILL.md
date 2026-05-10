---
name: composition
description: Use this skill when wiring two completed modules together to satisfy a parent contract, especially in a compartmentalised setting. Trigger this whenever the user has two implemented modules (each with its manifest) and a parent contract that says how they should compose, when the user asks to "compose these modules", "wire these together", "integrate A and B", or "build the parent module from these two children". This skill is deliberately mechanical — it favours faithful contract-to-contract wiring over creative integration, and it re-raises any genuine mismatch back up the composition tree rather than papering over it. Suitable for cheaper / smaller models because the work is structurally constrained.
---

# Composition

Wire two child modules together to satisfy a parent contract. You are not implementing new behaviour. You are routing data from the parent's inputs through the children's contracts to the parent's outputs, exactly as the parent contract specifies.

## Why this skill exists

In a compartmentalised system, every node in the build tree either implements a leaf module (one contract, no children) or composes two child contracts under a parent contract. This skill handles the second case. It is structurally simpler than leaf implementation — the children already exist and are trusted; the parent contract already exists and is the target. The composition's job is only to wire one to the other faithfully.

Because the work is constrained, this skill is the right place to use cheaper, smaller models. The rules below are deliberately tight to make that practical: most decisions are forced by the contracts involved, so there is little room for creative drift.

When a genuine mismatch exists — child contracts that cannot be reconciled, a parent contract that cannot be satisfied by the children chosen for it — this skill does not try to solve the mismatch. It surfaces a precise re-raise back up the tree, where an agent with the authority and context can resolve it.

## What you have

You have been given exactly six things:

1. **The parent contract** — the manifest the composition must satisfy.
2. **Child A's contract** — the manifest of the first child module.
3. **Child B's contract** — the manifest of the second child module.
4. **The shared vocabulary** — every data shape and external system the contracts refer to.
5. **Child A's implementation manifest** — the manifest under `.taniwha/kupu/implementations/<A>/v<N>/manifest.yaml` (or compositions/ if A is itself a composition). This tells you the source paths and confirms the implementation is `current` and verified. **You do not get child A's source code as input.** You can read the source paths from disk if and only if you genuinely need to (e.g. to discover an exported function name in a language where the contract didn't specify one) — and that need itself is a sign the contract may be incomplete.
6. **The project context** — language, repo style, directory layout, code conventions. The composition must be written in this language using these conventions.

You do not have the design doc. You do not have ancestor contracts above the parent. You do not have other siblings that may exist in the wider tree. You operate strictly within these six documents.

## Where your output goes

Composition source files live at the repo root, in the layout the project context specifies — typically a directory that wires modules together (e.g. `internal/api/`, `cmd/server/`, `src/handlers/`). The dispatcher provides explicit target paths.

The composition's `notes.md` goes inside `.taniwha/kupu/compositions/<id>/v<N>/` alongside the manifest. The manifest you produce includes `source_paths` listing exactly which repo-root files implement this composition.

## Process

### 1. Read all six documents end to end

Before writing any wiring, read the parent contract, both child contracts, the shared vocabulary, both child implementation manifests, and the project context. The wiring is determined by these six; no part of it should come from anywhere else.

### 2. Produce the canonical shared-types package

Before mapping inputs and outputs, walk the shared vocabulary and identify every entry marked `sharing: shared` whose `referenced_by` list includes both of your children. These are types that must exist as a single canonical implementation that both children import — without this, each child agent invented its own local copy and integration breaks at the type level.

For each such shared type and each shared error condition:

- Produce a canonical implementation in the language the project context specifies, at a shared-types location appropriate to the project's `repo_style`. For Go monorepo with internal/, this is typically `internal/shared/<types>.go` or similar. For Python, a `shared` module. For TypeScript workspaces, a `@project/shared` package. The exact path follows the project context's conventions.
- Use the canonical name from the vocabulary entry verbatim (subject to the project context's naming convention — e.g. `ShortCode` in PascalCase Go becomes `short_code` in snake_case Python, but stays the same conceptual name).
- The shape exactly matches the vocabulary entry's `fields:` block, translated into the language's idioms.

This shared package is part of your output. List it in your manifest's `source_paths` with `kind: code`. The two children's source files will import from this package — when wiring them together (steps 3 and 4 below), use these canonical types in the composition code.

If the children's existing source files already declare local copies of shared types (a common artefact when leaves were produced before sharing markers were available, or when leaves miss the sharing marker), the composition's job is to write the shared package and then the wiring code that bridges the gap. The bridging code is part of the composition output and goes in `source_paths` with `kind: code`. **Do not modify the children's source files** — they were produced under their own contracts; modification would invalidate verification. The bridge lives in the composition layer.

If you find a shared-marked type that would require modifying a child to use the canonical version (because the child's contract references the type at module-public surface), that's a re-raise: `category: mutually_incompatible, suspected_source: pairing`. The contracts cannot be honoured as written; either the contract for the child needs amending to use the canonical type, or the sharing marker is wrong. The user's call.

If the vocabulary has no entries with `sharing: shared` for the two children's pair, skip this step — there is no shared package to produce.

### 3. Map parent inputs to child inputs

For each input the parent contract declares, determine which child consumes it (or which both consume, or neither — the parent might have inputs only used to select between paths). Match by shape, name, and meaning, using the shared vocabulary as the ground truth for shapes.

If a parent input has no plausible consumer in either child, that is a signal — re-raise with `category: mutually_incompatible`, `suspected_source: pairing`. Either the children are wrong for this parent, or the parent contract is wrong for these children.

### 4. Map child outputs to parent outputs and to the other child's inputs

For each child output, determine whether it flows to the parent's outputs, to the other child's inputs, or both. The contracts should make this unambiguous; if they do not, it is a re-raise candidate.

Key check: when output of one child becomes input of the other, the **shapes must match exactly** as defined in the shared vocabulary. Not "compatible". Not "convertible with a small adapter you write". Exactly. If they do not match, re-raise — the contracts are wrong, and a hidden adapter inside the composition would be invisible coupling that nobody can later locate.

The one exception is the canonical-type bridging code from step 2: when a child's existing source declares its own local copy of a shared type, the bridge between the local type and the canonical type IS visible composition code, listed in your `source_paths`, and explicitly named in your `notes.md`. That kind of bridging is not "hidden"; it's the v1.5 mechanism for handling the transition where leaves were already produced without the sharing marker. New work should set up sharing markers correctly so this step is empty.

A second exception: if the parent contract itself specifies an explicit transformation between the two children, implement that transformation as the parent contract specifies it, and only as it specifies it.

### 5. Honour the parent's behavioural guarantees through composition

The parent's manifest declares behavioural guarantees: idempotency, ordering, atomicity, concurrency safety, resource bounds. The composition must produce these guarantees, working only from what the children's contracts promise.

For each parent guarantee, identify which child contracts (or which combination) make it true:

- **Idempotency**: if the parent is idempotent, both children's relevant operations must be idempotent, or the composition must include the deduplication logic the parent contract describes. If the parent contract describes no such logic and the children are not both idempotent, re-raise.
- **Ordering**: if the parent guarantees an order of effects, the composition must call the children in that order, and the parent must specify any retry/recovery rules.
- **Atomicity**: if the parent promises all-or-nothing, the parent contract must specify how. Compositions cannot invent transactional behaviour from non-transactional children — if the children's contracts do not provide the primitives the parent's atomicity needs, re-raise.
- **Concurrency**: the parent's concurrency guarantees must be derivable from the children's. If the parent promises thread safety and one child does not, re-raise.

The pattern is the same for each: the parent's promise must be **derivable from** the children's promises plus the wiring. If it is not, do not invent it.

### 6. Map child errors to parent errors

The parent contract declares its own error semantics. Each child's errors must either be:

- Caught and translated into one of the parent's declared error modes (the parent contract should specify how), or
- Re-raised as-is, *only if* the parent contract declares an error mode that exactly matches the child's.

If a child can raise an error with no corresponding parent error mode, that is a re-raise — the parent contract's error semantics are incomplete.

### 7. Self-verify

Before declaring the composition done, walk through:

- Every parent input is consumed somewhere, or explicitly specified as not consumed by this composition.
- Every parent output is produced from child outputs (possibly via specified transformations).
- Every parent behavioural guarantee is honoured by code that demonstrably honours it.
- Every parent error mode is reachable from real child failures or composition logic.
- Every child error has a destination — a parent error mode or a translation rule from the parent contract.
- No new side effects are introduced by the composition itself. Side effects belong to the children; the composition routes data and translates errors.

## When to re-raise

Re-raise (see `references/re-raise-protocol.md`) instead of producing wiring whenever any of these is true:

- **Children's shapes do not match where they need to.** `category: type_mismatch` (if a narrow shape conflict) or `mutually_incompatible` (if the conflict is structural). `suspected_source: contract_a`, `contract_b`, or `pairing` depending on which side seems wrong.
- **Parent contract demands a behavioural guarantee the children cannot supply.** `category: under_specified`, `suspected_source: parent_contract` — the parent did not specify the wiring needed to bridge the gap.
- **Parent's error semantics are incomplete given the children's failure modes.** `category: under_specified`, `suspected_source: parent_contract`.
- **Children seem wrong for this parent at a fundamental level.** Their domains do not overlap; one is solving a different problem. `category: mutually_incompatible`, `suspected_source: pairing`.
- **A child's manifest contradicts the parent contract's assumption about it.** `category: assumption_violated`, `suspected_source: contract_a` or `contract_b`.

A re-raise from this skill goes to the agent that authored the parent contract or chose the pairing. That agent either resolves the issue and re-dispatches, or re-raises further upward.

## What you must not do

This skill is deliberately constrained. The following are out of scope, no matter how tempting:

- **Do not invent adapter logic that hides a contract mismatch.** A two-line transform "to make the shapes line up" is the worst kind of bug — the contracts say things compose cleanly, but they don't, and now the lie is buried in the wiring. Re-raise.
- **Do not strengthen the parent's guarantees beyond what the children provide.** If the parent says "best-effort" and you could "easily" make it stronger by adding a retry, do not. The parent's contract is what callers rely on; tightening it silently misleads them.
- **Do not weaken the parent's guarantees because the children make it inconvenient.** If the parent says idempotent and the children make that hard, re-raise. Do not produce a composition that "is mostly idempotent in practice".
- **Do not add side effects (logging, metrics, tracing) that neither the parent nor the children declare.** The children own their effects; the parent declares its effects in terms of theirs. The composition is a router.
- **Do not call a third module to "help" the composition.** You only have two children. If a third is needed, the parent contract is wrong — re-raise.

## Quality checks before finishing

1. **Wiring is deterministic from the contracts.** A different agent with the same four documents would produce equivalent wiring.
2. **No new side effects.** Audit every line: no logs, metrics, files, network calls, or other emissions that are not already in a child's contract.
3. **No hidden adapters.** Every shape transformation is one the parent contract explicitly specifies.
4. **Every parent guarantee has a derivation.** You can point, line by line, to how each guarantee is produced from the children plus the wiring.
5. **Every child error reaches a destination.** No bare `except` blocks, no errors silently swallowed, no errors silently re-thrown without contract mapping.
6. **Composition is mechanical, not creative.** If you found yourself making a non-trivial design decision, that decision should have been a re-raise.

## Common failure modes

- **The "tiny adapter" trap.** The shapes almost match. A small mapping function fixes it. Resist — the smallness is precisely what makes this dangerous, because it makes the lie cheap to commit and expensive to find later.
- **Inferring the parent's intent and "helping".** The parent contract says X; you can see what they probably meant was X+Y. Do X. If they meant X+Y, the contract is wrong; re-raise.
- **Patching one child's deficiencies in the composition.** A child's contract has a gap, the composition compensates. The compensation is now permanent infrastructure that nobody knows about. Re-raise the child's gap to its author.
- **Cascading re-raises for trivia.** The other failure mode: re-raising things that you could resolve from your own four-document context. If the choice is between two reasonable wirings and the parent contract genuinely admits both, pick one and document the choice — that's not a re-raise, that's implementation.
- **Producing wiring that "works in practice" but does not honour the parent's guarantees.** The composition runs the test suite green, but the parent's idempotency guarantee is not actually held. Test green does not imply contract satisfied — the acceptance criteria do.

## Relationship to other skills

Inputs: a parent contract (from `contract-derivation`), two child manifests (also from `contract-derivation`), the shared vocabulary, and two completed child implementations (from `leaf-implementation` or from prior `composition` invocations — this skill is recursive).

Output: a composition that satisfies the parent contract, or a re-raise. The composition becomes a "child" at the next level up, where it will be composed with another child against a grandparent contract. The tree is built bottom-up; this skill is each interior node.

This skill is intentionally suitable for smaller, cheaper models. The constraints are tight, the rules are mechanical, and the escape hatch (re-raise) is well-defined. Save expensive models for design and leaf implementation, where genuine creativity is required.

## See also

- `references/re-raise-protocol.md` — the format for surfacing mismatches, missing guarantees, and contract gaps back up the composition tree.
