---
name: design-doc
description: Use this skill before writing any non-trivial code, especially when starting a new feature, service, or codebase. It produces a structural design document that commits to module boundaries, contracts, and data shapes BEFORE implementation begins. Trigger this whenever the user describes building something new, asks to "build", "create", or "implement" a system or feature larger than a single function, or whenever the conversation is about to move from discussion into code. Also trigger if the user mentions architecture, system design, planning, or wanting to "think through" a build before coding. Especially important for AI-generated codebases where unconstrained generation tends to sprawl into accidental complexity.
---

# Design Doc

Produce a structural design document that commits to the shape of a system before any code is written. The document is the seed artefact for everything downstream — contract derivation, leaf implementation, and composition all read from it.

## Why this skill exists

When code generation begins without an explicit structural commitment, each step optimises locally. A class is added here, a wrapper there, a "just in case" abstraction somewhere else. There is no global view to push back against any individual decision, so the codebase accretes complexity that nobody asked for and nobody can later justify.

A design document forces the global decisions to happen first, in one place, where they can be evaluated against each other. Once it exists, every subsequent generation step has a constraint to honour rather than a blank canvas to sprawl across. The discipline is the value; the document is the artefact that carries it.

This is not a UML diagram and it is not a README. It is the smallest artefact that can fully answer: what modules exist, what is each one responsible for, what does each one expose, and how does data flow between them.

## When to produce a design doc

Produce one whenever the work is larger than a single function or single file change. The cost of the document is small; the cost of an unconstrained generation that has to be unwound later is large.

Skip it for: bug fixes scoped to one module, single-file scripts, experimental throwaway code the user has explicitly framed as throwaway, or modifications that fit entirely within an existing module's contract.

## The single most important rule

**If the prompt under-specifies a behaviour, ask. Do not decide.**

The most expensive failure mode of this skill is silently filling in plausible-looking detail that the user never asked for. A prompt like "build me a URL shortener" does not specify retry counts, consistency policies, or the exact shape of given external interfaces — and those are not decisions for you to make. They are decisions for the user. Picking a "reasonable" value for any of them buries a tradeoff in the design that the user never agreed to and that downstream agents will faithfully implement.

Your job is not to produce a complete design from any prompt. Your job is to produce a complete design from a *complete enough* prompt — and to identify gaps when the prompt is not. The audit step below is mandatory and runs before any structural work.

## Language neutrality

The design document must be implementable in any reasonable language. Language choice is the user's, captured in `project_context.yaml` separately. Your input may include a small amount of project context (typically just `{language.name, repo_style.kind}`) so you can phrase scope correctly — for instance, knowing the project is a single binary versus a multi-package monorepo affects what you put in "out of scope". But the modules and contracts you describe must use language-neutral terminology.

Concretely:
- Don't name modules in language-specific styles (`UserController`, `IUserRepository`, `userMod`). Use kebab-case capability names that any language can map onto its own naming convention (`user-management`, `link-resolution`).
- Don't describe behaviours using language-specific concurrency or error idioms (`goroutines`, `promises`, `exceptions`, `Result types`). Describe semantics: "concurrent invocations are safe", "signals an error condition", "the call is dispatched without awaiting its result".
- Don't assume a runtime model (event loop, garbage collection, threading model). The contract-derivation and leaf-implementation skills will translate semantic guarantees into language-appropriate mechanisms; your job is to state the semantics correctly.

If you find yourself unable to describe a system without committing to a language, that is a sign that the system's domain is genuinely language-bound (rare — usually only for things like "build a kernel module" or "build a browser extension"). In that case, the project context must say so first; otherwise re-raise.

## Process

Work through these in order. Do not jump ahead — earlier sections constrain later ones, and skipping creates the same local-optimisation problem the skill exists to prevent.

### 0. Audit the prompt for under-specification

Before any structural work, walk through the prompt and identify everything it does *not* specify that a downstream implementor would need a definite answer for. The categories below are predictable; check each one explicitly.

- **Error and failure policies.** What happens on every plausible failure of every external interaction? Retries (how many, with what backoff, on what conditions)? Fallbacks? Hard failures? Silent degradation?
- **Concurrency and consistency tradeoffs.** What happens on partial failures of multi-step operations? Are operations atomic, eventually consistent, or fire-and-forget? What ordering guarantees are needed?
- **Exact shapes of "given" external interfaces.** When the prompt says "assume X is given", what operations does X actually expose? What are their semantics? Picking a plausible interface is *not* what was asked.
- **Capacity and bounds.** Maximum sizes, rates, quantities? What happens at the limits?
- **Behavioural guarantees that aren't explicit.** Idempotency? Order preservation? Latency expectations? Concurrency safety?
- **Disambiguations between reasonable interpretations.** Wherever the prompt admits two materially different readings, pick neither — surface the question.
- **Magic numbers.** Any quantity the prompt didn't name (retry counts, timeouts, code lengths, buffer sizes, page sizes). Defaults are decisions; decisions belong to the user.

Write down every gap you find. Be specific — "what happens on collision" is too vague; "when an auto-generated code collides with an existing one, should the system retry (how many times?), return an error, or something else?" is actionable.

If the list is empty, proceed to step 1. If the list is non-empty, you have two options, and you must pick one explicitly:

**Option A: Re-raise.** If any of the gaps is genuinely load-bearing — i.e. different reasonable answers would produce materially different module structures, contracts, or behavioural guarantees — emit a re-raise (see `references/re-raise-protocol.md`), category `under_specified`, suspected_source `parent_contract`. Include all of the gaps in the re-raise so the user can answer them in one round. Do not produce a design doc; the re-raise is your output.

**Option B: Defer to the user explicitly within the design doc.** If the gaps exist but none of them affect the high-level structure (they only affect, say, the value of a single configuration knob), include an "Open questions" section at the top of the design doc. Each open question is stated as a question to the user, not as a decision. Downstream agents reading the design doc will surface a re-raise if they encounter an open question they need to act on.

Re-raise is the default. Use Option B only when the questions are genuinely peripheral to structure.

The point of this step is that downstream agents (contract-derivation, leaf-implementation, composition) all carry their own discipline against silent decision-making — but they only fire if you didn't pre-empt them by burying the decision in the design. You are the first line of defence. Be thorough.

### 0a. Distinguish user-delegated decisions from agent-silent decisions

When you write the design doc, every choice that fills a gap from the audit must be one of two clearly-marked things:

- **User-delegated decision.** The user explicitly gave you authority over this dimension. Examples: "build it however you think best", "use sensible defaults", or domain-specific equivalents like "pick a reasonable retry count". For these, write the decision into the design with a `delegated:` annotation that quotes the user's exact words granting the delegation. Downstream agents and the user reviewing the design can see "this was the user's call to delegate to me".
- **Open question.** No delegation has been granted. The decision belongs to the user but you have not yet asked. Write it under "Open questions" at the top of the design with a `requires_user_decision: true` annotation. Downstream agents must not act on these without resolution.

A common silent-decision failure mode looks like this: the design says "stats-retrieval returns 0 for nonexistent codes" with no annotation. The agent silently picked an interpretation (return 0 vs raise error), the user never knew, and downstream contracts faithfully implement the wrong behaviour. The fix is structural: every decision in the design must be either explicitly delegated or explicitly open. There is no third "I just decided" category.

If you find yourself writing a clause and you cannot honestly say "the user delegated this" or "this is an open question", that's the failure signal — stop and ask. The audit step's whole point is to make this distinction visible; carrying the distinction through into the design doc preserves it.

### 0b. Decide the project's structural tier

Before any structural work, decide how much architectural structure this brief actually needs. This is the single decision that has the largest effect on downstream cost. Get it wrong toward over-decomposition and you spend orders of magnitude more time and tokens producing a sprawling codebase for a small problem; get it wrong toward under-decomposition and a genuinely large project ends up as one unmaintainable file.

The agent's natural bias is toward decomposition — that's the failure mode this step exists to counter. **Default to the smallest tier the brief admits.**

Three tiers:

**`single_module`** — one contract, one implementation, one verifier. No vocabulary file (or a minimal one), no composition. The whole project lives at the repo root in whatever single-package layout the project context names. Choose this when:
- The brief describes one capability (even if that capability has multiple operations)
- All operations share state or are tightly coupled
- No part of the system would be independently useful or independently swappable
- Total scope estimable as "a few hundred lines"

**`small_multi_module`** — 2 to 4 modules with one composition layer above them. No nested compositions. Choose this when:
- The brief describes 2–4 distinct concerns with clear boundaries
- The concerns have meaningfully different failure semantics, scaling profiles, or dependencies
- Each concern would benefit from independent contracts and verification
- Total scope estimable as "single thousands of lines"

**`full_decomposition`** — 5 or more modules with nested composition tree. Choose this when:
- The brief describes a system with multiple subsystems, each itself complex
- Substantial independent functionality at multiple levels
- Real boundaries that benefit from composition agents
- Total scope estimable as "many thousands of lines"

### Justification rules (non-negotiable)

For any tier above `single_module`, you must justify the choice with **specific brief content** that makes the smaller tier insufficient. "More structure is better engineering" is not a justification; it's the bias this rule exists to counter. "The brief mentions X, Y, and Z which have meaningfully different failure semantics" is a justification.

For any tier with N modules, **each module beyond the first** must have its own justification tied to specific brief content. Walk through:
- Module A: what brief content makes this a separate module?
- Module B: what brief content makes this a separate module from A?
- ...

If you cannot construct a brief-content-tied justification for a module, that module is the agent's bias talking, not the brief's requirement. Drop it. The work it would have done belongs in another module.

If the brief is genuinely ambiguous about scope, the right action is the same as for any other ambiguity: re-raise. Do not pick the larger tier "to be safe" — over-decomposition has real costs. Smallest tier the brief admits, with specific justification for any expansion.

### Tier output

Record the tier decision at the top of the design doc, before the purpose paragraph:

```
## Structural tier

Selected: single_module | small_multi_module | full_decomposition

Justification: <one paragraph tying the tier choice to specific brief content>

Modules: <count, with each module's brief-tied justification if more than one>
```

The user reviews and approves the tier alongside the design itself. If the user rejects the tier, that's a re-raise back to design-doc to redo at a different tier.

### What changes downstream

The tier decision shapes how every later step runs:

- **`single_module`**: skip step 6 (composition) entirely; produce one contract; vocabulary is omitted or minimal; design doc has one Modules entry.
- **`small_multi_module`**: produce one composition layer; vocabulary lists shared types if any; design doc has 2–4 Modules entries plus one Composition entry.
- **`full_decomposition`**: produce nested compositions; full vocabulary with sharing markers; design doc has 5+ Modules entries plus a Composition tree.

Steps 1 through 7 below describe the full-decomposition shape. For smaller tiers, omit the parts that don't apply.

### 1. Capture the system's purpose in one paragraph

Before any structure, write a single short paragraph stating what the system does and for whom. This paragraph is the only place the system's purpose is allowed to appear. Every later section refers to mechanism, not purpose. This separation is deliberate — it lets downstream agents work on individual modules without the system's purpose biasing their decisions.

### 2. Identify external boundaries

List every interface the system has with the outside world: HTTP endpoints exposed, external services called, databases touched, message queues consumed or produced, filesystems read or written, user interfaces presented. For each, name it and state the direction of data flow. Do not yet describe internal modules.

### 3. Identify internal modules by capability (or skip if `single_module`)

If you chose `single_module` in step 0b, this step has one entry — the whole system as one module — and you skip ahead to step 4. The "name" is just the project name; the responsibility is the whole brief; "not responsible for" is the out-of-scope list.

If you chose a multi-module tier, decompose the system into modules organised by **capability** (the business thing the module does), not by **layer** (controller / service / repository). Capability slices localise change: a future modification to "user invitations" should touch one folder, not seven.

For each module beyond the first, the brief-content justification from step 0b must hold. If you find yourself wanting to add a module that has no specific justification, do not add it.

For each module, write:
- A short name (kebab-case, suitable for a folder).
- A one-sentence statement of what it is responsible for.
- A one-sentence statement of what it is explicitly NOT responsible for, if there is a plausible neighbouring concern.

Aim for modules that fit in a junior engineer's head. If a module's responsibility statement needs an "and", consider whether the "and" is genuine separate scope (split it) or a single coherent capability stated awkwardly (rewrite).

### 4. Specify each module's contract

For each module, specify its public surface. A contract must be complete enough that an implementor working in isolation, with no other context, could build the module correctly. Under-specified contracts are the single biggest failure mode — be deliberate.

Each contract includes:

- **Inputs** — every input the module accepts, with type shape and meaning. Distinguish required from optional.
- **Outputs** — every output the module produces, with type shape and meaning.
- **Side effects** — every effect on the world outside the module's return value: database writes, external calls, log emissions, file writes, message publications. "None" is a valid and useful answer.
- **Error semantics** — what failure modes exist, how they are signalled (exception, error value, status code), and what the caller is contracted to do about each.
- **Behavioural guarantees** — properties the module promises that are not visible in the type signature: idempotency, ordering, atomicity, latency bounds, concurrency safety, bounds on resource use. Many bugs in agent-generated code come from these going unstated.
- **Dependencies** — every other module in the system this one calls. If it calls none, say so explicitly.

### 5. Specify data shapes that cross boundaries

Any data structure that flows between modules, or between the system and the outside world, gets named and defined once. Modules refer to these shapes by name in their contracts. This prevents the same shape being redefined slightly differently in three places, which is a frequent source of agent-introduced bugs.

### 6. State the composition (or skip if `single_module`)

If `single_module`, skip this step. The single module IS the system; there is no composition.

If multi-module, describe how modules combine to deliver each external boundary's behaviour. A simple list per external boundary, naming the modules involved and the order they participate, is sufficient. If composition is non-trivial (fan-out, retry, conditional branching), state it explicitly.

### 7. State what is explicitly out of scope

End with a short section listing concerns that might reasonably be expected but are deliberately not addressed. This prevents downstream agents from "helpfully" adding scope that was not asked for. Examples: "no caching layer in this iteration", "authentication is assumed to be handled upstream", "no internationalisation".

## Output format

If step 0 produced a re-raise, your output is the re-raise YAML, not a design doc. Stop here.

Otherwise, produce a single Markdown document. Use this exact structure:

```markdown
# [System name]

## Structural tier

**Selected:** single_module | small_multi_module | full_decomposition

**Justification:** [one paragraph tying tier choice to specific brief content]

**Module count:** [N, with brief-tied justification per module if N > 1]

## Open questions
[Only present if step 0 surfaced peripheral questions that should be deferred to the user. Each question stated as a question, not a decision. If no questions, omit this section entirely — do NOT include it as "None".]

## Purpose
[One paragraph. The only place purpose is stated.]

## External boundaries
- **[name]**: [direction, type, brief description]
- ...

## Modules
### [module-name]
**Responsible for:** [one sentence]
**Not responsible for:** [one sentence, if relevant]

**Inputs:**
- ...

**Outputs:**
- ...

**Side effects:**
- ...

**Error semantics:**
- ...

**Behavioural guarantees:**
- ...

**Dependencies:**
- ...

### [next module]
...

## Data shapes
### [ShapeName]
[Definition]

## Composition
- **[external boundary]**: [modules involved, order]

## Out of scope
- ...
```

## Quality checks before finishing

Run through these before declaring the document complete. Each one is a failure mode this skill exists to prevent.

1. **No silent decisions.** Walk every clause of the design doc and ask: did the user actually specify this, or did I pick a value? If you picked a value (a retry count, a timeout, a consistency policy, an interface shape, an error-handling rule, a magic number), the value should either be stated by the user, justified by an explicit, narrowly-scoped engineering reason that any reader would agree with (e.g. "code length is 7 because the prompt's stated capacity of 1B distinct codes requires at least 6 alphanumeric characters"), or moved to the Open questions section. Plausibility is not justification.
2. **No purpose leakage.** Every section after "Purpose" describes mechanism. If a module's responsibility statement explains *why* the system needs it, rewrite it to describe *what it does*.
3. **Capability slicing, not layer slicing.** No module is named `controllers`, `services`, `repositories`, `utils`, or `helpers`. If you see one, the decomposition is wrong.
4. **No "and" in responsibility statements.** Each module does one thing. Compound responsibilities mean the module should be split.
5. **No under-specified contracts.** For each module, ask: could an implementor who has never seen this system build this correctly from this contract alone? In particular, are error semantics and behavioural guarantees stated, or only implied?
6. **No unstated dependencies.** Every module's dependency list either names other modules or says "none". An empty field is a bug.
7. **Data shapes defined once.** No shape is redefined inline in a module contract — it is named, defined under "Data shapes", and referenced.
8. **Out-of-scope is non-empty.** If you cannot think of anything reasonable to exclude, you have not thought hard enough about scope.

## Common failure modes

- **Silently filling in behavioural details the prompt didn't specify.** The single most expensive failure of this skill. Picking a "reasonable" retry count, consistency policy, or interface shape buries a tradeoff in the design that the user never agreed to and that downstream agents will faithfully implement. The discipline against this is step 0 plus quality check 1. If you find yourself reaching for a "default", that is the signal — re-raise or open-question it.
- **Designing implementation, not interface.** The document describes algorithms or data structures internal to a module. Strip those out — they belong to the implementor, not the design.
- **Anticipating modules that "might be needed later".** Speculative modules accrete cost immediately and are rarely the right shape when the future arrives. Design for what is asked, with explicit out-of-scope notes for the rest.
- **Over-fragmenting modules under "no compound responsibilities".** The rule against compound responsibilities is not a rule against any module exposing more than one operation. A module exposing four operations against the same data with the same semantics is one capability, not four. Apply the rule to *responsibilities* (what the module is for), not to *operations* (what it can do). If splitting would produce four "modules" that all depend on the same underlying state and exist to serve the same external boundary, they are one module with four operations.
- **Pattern-matching to familiar architectures.** Layered architectures, hexagonal architectures, clean architecture and friends are tools, not goals. If the system's natural decomposition is three capability modules, three capability modules is the answer; a layered structure on top of that is ceremony.
- **Hand-waving error semantics.** "Errors are returned" is not a contract. State which errors, in what form, and what the caller is contracted to do.

## Relationship to downstream skills

The design doc is the input to the `contract-derivation` skill, which splits each module's contract into a form suitable for compartmentalised implementation. If the design doc is incomplete or under-specified, contract derivation will surface the gaps as re-raises rather than papering over them. This is intended: better to discover under-specification at design time than at integration time.

If working alone (without the rest of the Taniwha framework), the design doc is still directly useful — it is a structural commitment that any subsequent code generation, agent or human, can be measured against.
