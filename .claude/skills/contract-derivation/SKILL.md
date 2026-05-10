---
name: contract-derivation
description: Use this skill after a design document exists and before any module is implemented. It takes a design doc and derives per-module contracts (manifests) that are complete enough for an implementor working in isolation to build each module correctly without seeing the rest of the system. Trigger this whenever the user has a design doc and is about to start implementation, when the user asks to "derive contracts", "produce manifests", "split this design", or "prepare modules for implementation". Also trigger when an existing system needs its modules' contracts reverse-engineered before further compartmentalised work. This skill is the bridge between system-level design and isolated implementation.
---

# Contract Derivation

Take a design document and produce a set of per-module contracts (manifests) that are complete in isolation. Each manifest must let an implementor build its module correctly without seeing any other part of the system.

## Why this skill exists

A design document captures the system's structure, but its module contracts often lean on shared context — adjacent modules, the system's purpose, implicit assumptions a human reader would fill in. Compartmentalised implementation cannot rely on any of that. The implementor of one module sees only the manifest for that module, and nothing else. Anything the manifest fails to state will either be guessed (badly) or surfaced as a re-raise.

This skill's job is to take each module's contract from the design doc and harden it: make every implicit assumption explicit, name every behavioural guarantee, define every error mode, and lock down every data shape it references. The resulting manifests are the durable source of truth for the rest of the build.

## Process

### 1. Read the design doc end to end first, and read the structural tier

Before deriving any single manifest, read the entire design doc. Each manifest will be produced in isolation, but you cannot produce them in isolation — you need the whole picture to know which assumptions are shared, which data shapes flow where, and which modules depend on which.

The design doc's "Structural tier" section shapes everything you do:

- **`single_module`**: Produce one manifest. **No vocabulary file is produced by default** — the single module's contract names its own types inline. If you do produce a vocabulary file (only when the brief specifically names types as part of an external interface), every entry MUST be `sharing: local`; no `sharing: shared` entries are valid for single-module projects. No composition tree. See step 3 for the strict rule.
- **`small_multi_module`**: Produce 2–4 manifests plus one vocabulary file. Step 3 (catalogue shared vocabulary) runs as written. The composition layer above will be a single composition; sharing markers matter for the types that flow across the module boundary.
- **`full_decomposition`**: Produce 5+ manifests plus a full vocabulary. All steps run as written. Sharing markers are critical because the composition tree is deep.

If the tier is `single_module`, the rest of this skill simplifies considerably. The "isolation check" still applies — the manifest must be complete enough for an implementor to build the module — but cross-manifest concerns (shared vocabulary, sharing markers, language-neutrality across multiple modules) reduce to "ensure the single manifest is internally complete and language-neutral".

### 2. Audit the design doc for silent decisions

Before deriving any contract, walk the design doc looking for behaviours, magic numbers, or policies that the user did not specify in the original prompt and that the design-doc skill should have surfaced as questions but didn't. Common shapes:

- A specific retry count, timeout, or other magic number with no justification tied to a stated requirement.
- An error-handling policy ("this failure is logged and ignored"; "this retries up to N times") whose value is not derivable from the prompt.
- An assumed shape of an external interface that the prompt said was "given" without specifying.
- A consistency or ordering tradeoff resolved one way without acknowledgement.

If the design doc has an "Open questions" section, those questions count too — anything an implementor would need to act on must be answered before contracts can be derived.

If you find any of these, **emit a re-raise** before producing any manifests. Use category `under_specified`, suspected_source `parent_contract`, and list every silent decision in `attempted_resolutions` with what would be needed to resolve them. Do not derive contracts that bake in silent decisions, even if the design doc states them explicitly. Once they are in the manifest, the implementor will faithfully build them, and the user never gets asked.

This is defence in depth — the design-doc skill is supposed to catch these first, but if it didn't, you must.

### 3. Catalogue the shared vocabulary (skip or minimise for `single_module`)

**For `single_module` tier, sharing analysis is skipped entirely.** A `sharing: shared` marker means "more than one module references this type". For a single-module project, every type is referenced by exactly one module by definition — there is no second module. Marking anything `sharing: shared` is a category error and produces a real downstream failure: the leaf-implementation skill enforces "must import shared types from the canonical path", which doesn't exist in a single-module project, causing the leaf to re-raise. The amendment loop is preventable: don't generate the shared markers in the first place.

Concrete rule for `single_module`:

- **If you produce a vocabulary file at all, every entry MUST be `sharing: local`.** No `sharing: shared` entries are valid in a single-module project.
- **In most cases, you should skip producing a vocabulary file.** The single module's contract names its own types inline. A vocabulary file is only useful when types are exposed to callers outside the module (the user, the system that wires this module up later) and need a stable reference point. Even then, those entries are `sharing: local` from Taniwha's perspective — they are local to the only module that exists. The "sharing" they do is with external code, which is outside Taniwha's scope.
- **Skip the vocabulary file by default.** Produce one only if a downstream agent (verifier, future composition) would genuinely need it. If unsure, skip — the absence of a vocabulary file for a single-module project is correct; the presence of one with `sharing: shared` markers is wrong.

For multi-module tiers, identify every named data shape, every external system referenced, and every cross-cutting concern (authentication, logging, transaction boundaries) that appears across modules. These will be referenced by name from individual manifests but must be defined once, in a shared section that travels with every manifest. Sharing markers (`sharing: shared` or `sharing: local`) apply per the v1.5 rules — `shared` when more than one module references the entry, `local` when only one does.

### 4. For each module, produce a manifest

Each manifest has a fixed structure (see "Manifest format" below). Work through the modules one at a time. For each, do not just transcribe the design doc — interrogate it.

For every contract field, ask:

- **Is this stated, or am I inferring it from elsewhere in the design doc?** If inferring, write it down explicitly in the manifest. The implementor will not have the rest of the design doc.
- **Could a reasonable implementor read this and produce two materially different implementations?** If yes, the contract is ambiguous. Tighten it, or flag it for the design author to clarify.
- **Are error semantics complete?** For each error mode the contract acknowledges, is the error type, the signalling mechanism, and the caller's obligation stated? "Errors are propagated" is not complete.
- **Are behavioural guarantees complete?** Idempotency, ordering, atomicity, concurrency safety, latency bounds — none of these can be inferred from a type signature. State them explicitly, including stating "no guarantee" where that is the intent.
- **Are dependencies expressed by contract, not by module?** A manifest's dependencies should reference the *contracts* of dependent modules, not assume implementation details of those modules. The implementor will not see those modules; only their contracts.

### 5. Surface gaps as re-raises, do not fill them in

If a manifest cannot be completed because the design doc is genuinely under-specified, do not guess. Emit a re-raise (see `references/re-raise-protocol.md`) with `category: under_specified` and `suspected_source: parent_contract`, naming the specific clause and what is missing. The design doc author resolves it, then derivation continues.

This is the single most important rule of this skill. Filling in plausible-looking detail to "make the manifest complete" produces manifests that lie. Implementors will trust them. The result is worse than an honest re-raise.

### 6. Validate each manifest in isolation

Before declaring a manifest done, run the isolation check: imagine you are an implementor who has been handed only this manifest, the shared vocabulary, and the re-raise protocol. Nothing else. Could you build the module correctly? If you have any question whose answer is not in those documents, the manifest is not done.

## Manifest format

Each module gets a single Markdown file. Use this structure exactly:

```markdown
# Manifest: [module-name]

## Responsibility
[One sentence. Mechanism, not purpose.]

## Not responsible for
[One sentence, if relevant. Bounds the scope.]

## Inputs
- **[name]** (`[type]`, [required|optional]): [meaning, constraints, validation rules]

## Outputs
- **[name]** (`[type]`): [meaning, constraints]

## Side effects
- [Each effect on the outside world. Be specific: which database, which table, which external service, which log channel. State "none" explicitly if there are none.]

## Error semantics
- **[error condition]**: signalled as [mechanism]; caller contracted to [obligation].

## Behavioural guarantees
- **Idempotency**: [yes/no/conditional, with conditions]
- **Ordering**: [guarantees made about order of effects or outputs]
- **Atomicity**: [what happens on partial failure]
- **Concurrency**: [thread/process safety properties]
- **Resource bounds**: [memory, time, external calls — where relevant]
- [Any other guarantee specific to this module]

## Dependencies
- **[contract-name]**: [which operations of that contract are used]
- (Or: "none")

## Referenced data shapes
- [List of data shape names this manifest refers to. Definitions live in shared vocabulary.]

## Acceptance criteria
- [List of testable conditions that, if all hold, mean the module satisfies its contract. These should be objective and verifiable without seeing the implementation.]
```

## Shared vocabulary file

Alongside the per-module manifests, produce one shared vocabulary file. This contains:

- **Data shapes**: every named data structure referenced by any manifest, defined exactly once.
- **External systems**: every external service, database, or interface referenced by any manifest.
- **Cross-cutting concerns**: any policy that applies across modules (authentication model, error wrapping conventions, logging requirements).
- **Sharing markers**: for each data shape AND each named error condition, indicate whether it is `shared` (multiple modules reference the same instance) or `local` (only one module names it).

Every manifest implicitly travels with this file. An implementor receives all three of: their module's manifest, the shared vocabulary, and the project context.

### Sharing markers

This is the load-bearing detail in vocabulary design. When two modules both reference (say) `ShortCode`, each leaf agent reads its own contract and produces a local type — `linkcreation.ShortCode`, `httpapi.ShortCode`, etc. — that has the right shape but is not the same Go type as the others. The composition layer then has to do extensive type translation, or worse, the composition layer is skipped and the modules don't compose.

To prevent this, every entry in the vocabulary names whether it is shared:

```yaml
data_shapes:
  - name: ShortCode
    sharing: shared
    referenced_by: [link-creation, http-api, stats-query]
    description: |
      The short code identifier returned from link creation. <semantic description>.
    fields:
      - name: code
        type: <language-neutral type description>

  - name: GenerationAttemptCount
    sharing: local
    referenced_by: [code-generation]
    description: |
      Internal counter used only by code-generation. Not exposed to other modules.
    fields:
      - name: attempts
        type: integer

errors:
  - name: NotFound
    sharing: shared
    referenced_by: [link-resolution, stats-query, http-api]
    semantics: "Signalled when a code does not exist in the store."

  - name: GenerationExhausted
    sharing: local
    referenced_by: [code-generation]
    semantics: "Internal to code-generation; surfaced through the parent contract as GenerationFailed."
```

A shape or error is `shared` when more than one module references it. A shape or error is `local` when exactly one module references it. This is a structural fact that follows from the contract set — the deriving agent does not invent the marker, just observes which references exist.

### Consequences of sharing markers

`sharing: shared` is a contract on the **composition layer**: when composition runs above two children that both reference a shared type, the composition is responsible for producing the canonical implementation of that shared type at the level appropriate to the composition. The shared type lives in a package or module the children both import; the composition writes that package as part of its work. The leaves do not invent local copies — they reference the canonical name and the composition wires the canonical type into place.

`sharing: local` is a contract on the **leaf**: the type is the leaf's internal concern, can be named whatever the leaf wants, and is not exposed at the module boundary.

The leaf-implementation skill knows to read the sharing markers and, for shared types, defer to the canonical name and structure rather than declaring its own. The composition skill knows to produce the canonical implementations of shared types it sees in its children's contracts. Vocabulary without sharing markers is incomplete and gets re-raised.

## Quality checks before finishing

1. **Isolation check passes for every manifest.** Reading only the manifest and shared vocabulary, an implementor has no unanswered questions.
2. **No manifest references another module's implementation.** Dependencies are stated as contracts, not as modules-with-known-internals.
3. **Every error mode has a signalling mechanism and a caller obligation.** No bare "errors propagate".
4. **Every behavioural guarantee section is non-empty.** "No guarantee" is a valid entry; missing entries are not.
5. **Every data shape used is defined in the shared vocabulary.** No inline shape definitions inside a manifest.
6. **Acceptance criteria are objective.** A reviewer can check them against an implementation without judgement calls.
7. **Re-raises were emitted, not papered over.** If anything could not be derived from the design doc, it surfaced as a re-raise rather than being silently filled in.
8. **Manifests are language-neutral.** No language-specific terms appear anywhere in any manifest. See "Language neutrality" below.
9. **Every vocabulary entry has a sharing marker.** Every data shape and every named error has `sharing: shared` or `sharing: local`. The `referenced_by` list reflects which modules' contracts mention it. An entry without a sharing marker is incomplete.

## Language neutrality

**Manifests must be implementable in any language the project context could specify.** Contracts describe semantics — what the module promises about behaviour — not how those semantics are realised in any specific language or runtime. A contract that mentions "goroutines" is unusable as a Python contract. A contract that mentions "promises" is misleading as a Go contract. The implementor reads `project_context.yaml` to know which idioms are appropriate; the contract must not pre-empt that choice.

This is a real failure mode, not a theoretical concern. If a contract uses the word "goroutines" while describing concurrency semantics and the design doc and brief never mentioned a language, the leaf-implementation agent will take the contract's word choice as authoritative and write the entire codebase in Go without the user being asked. Contract-derivation is the layer where that drift starts; this skill is responsible for preventing it.

### Forbidden term lists

The following terms MUST NOT appear in any manifest. They are language- or runtime-specific and pre-empt implementation choices:

- **Concurrency idioms:** `goroutine`, `goroutines`, `green thread`, `green threads`, `coroutine`, `coroutines`, `async/await`, `Promise`, `promise`, `Future` (capitalised, when referring to a specific language type), `Task`, `Deferred`, `actor` (when referring to a runtime model rather than a domain concept), `fiber`, `fibers`.
- **Memory model idioms:** `pointer`, `reference` (when referring to a specific language's concept), `borrow`, `borrowed`, `move semantics`, `garbage collected`.
- **Error idioms:** `exception` (when referring to a language-specific exception type), `panic`, `panics`, `Result type` (specific to Rust/OCaml), `Either`, `Maybe`/`Option` (specific types).
- **Function-call idioms:** `callback`, `callback hell`, `monad`, `lambda` (when describing a runtime concept rather than the math), `closure` (in language-mechanic terms; "stateful function" is fine).
- **Build/runtime idioms:** `JIT`, `bytecode`, `bundle`, `transpile`, `module bundler`.

If you find yourself wanting to use one of these terms because it precisely captures the semantics, reach for a language-neutral substitute instead. A non-exhaustive substitution table:

| Forbidden | Use instead |
|-----------|-------------|
| "safe to call from multiple goroutines" | "safe under concurrent invocation" |
| "fire-and-forget via goroutine/promise" | "dispatched without awaiting its result" |
| "callers should await this" | "the call is synchronous from the caller's perspective" |
| "throws an exception" | "signals an error condition" / "fails with error type X" |
| "returns a Result" | "returns either the success value or an error of kind X" |
| "passed by reference" | "the caller and module share a single instance" |
| "passed by value" | "the module receives a copy and cannot affect the caller's instance" |
| "callback function" | "a function the caller supplies, invoked when X happens" |
| "garbage collected" | "memory management is the runtime's responsibility" |

If a semantics genuinely cannot be expressed without a specific runtime concept (extremely rare), that is a re-raise: the project context may need to be amended to fix the language, OR the design needs to acknowledge that this module imposes runtime requirements. Do not bake the runtime assumption into the contract silently.

### What the contract CAN say about runtime

Contracts may name properties that are runtime-relevant when expressed neutrally:

- "Concurrent invocations are safe; the module holds no shared mutable state."
- "Atomic with respect to observation by other concurrent operations on the same key."
- "Idempotent under retries within a 24-hour window."
- "Allocates memory bounded by O(input size); does not allocate per-call beyond input."
- "Failure of this call must not block the caller's response path; how this is achieved is the implementor's choice given the project context."

That last form — "how this is achieved is the implementor's choice given the project context" — is the canonical way to defer a runtime question that a contract cannot pre-decide.

## Common failure modes

- **Smoothing over ambiguity.** The design doc is unclear; the deriving agent picks an interpretation and writes it as if it were stated. The implementor builds to that interpretation. The composer discovers the design author intended something else. This is the single most expensive failure pattern in compartmentalised systems — re-raise instead.
- **Type-signature-only contracts.** The manifest specifies inputs and outputs but leaves error semantics and behavioural guarantees implicit. This produces implementations that compile and look right but fail at composition.
- **Implementation leaking into contract.** The manifest specifies *how* the module works, not just *what* it promises. This pre-empts the implementor's choices and will become wrong when the implementation needs to change.
- **Language-specific terms slipping into contracts.** The most insidious form of "implementation leaking into contract" — the deriving agent reaches for a precise term ("goroutine", "promise") and contaminates the manifest with a language assumption. See "Language neutrality" above; this is a non-negotiable rule, not a stylistic preference.
- **Cross-manifest coupling by accident.** Two manifests reference the same concept under different names, or define the same shape twice with subtle differences. The shared vocabulary fixes this — use it.
- **Acceptance criteria that require seeing the implementation.** "The function uses a hash map" is not an acceptance criterion. "Lookup is O(1) amortised on input size" is.

## Relationship to other skills

Input: the design document produced by the `design-doc` skill.

Output: a set of manifests plus a shared vocabulary file. Each manifest is the input to one invocation of the `leaf-implementation` skill. The shared vocabulary travels with every manifest.

If a manifest's contract can be further decomposed into smaller modules — i.e. the module is itself non-trivially composite — re-run this skill on that module's contract to produce sub-manifests. The composition tree is recursive; this skill is the recursion step.

## See also

- `references/re-raise-protocol.md` — the format for surfacing under-specification back to the design author.
