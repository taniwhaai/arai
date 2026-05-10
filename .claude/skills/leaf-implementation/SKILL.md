---
name: leaf-implementation
description: Use this skill when implementing a single module against a contract manifest, especially in a compartmentalised setting where the implementor should NOT see the rest of the system. Trigger this whenever a manifest exists and the user wants to implement that module, asks to "implement this module", "build against this contract", or hands over a manifest with a request to write the code. Also trigger this whenever code is being written against a defined interface and the user wants disciplined, contract-faithful implementation rather than free-form generation. The skill enforces strict adherence to the contract, refuses scope expansion, and surfaces ambiguity as a re-raise instead of guessing.
---

# Leaf Implementation

Implement one module from one manifest. Do not exceed the contract. Do not infer the system's purpose. Do not reach beyond the manifest and shared vocabulary you were given.

## Why this skill exists

Compartmentalisation is the property that makes large agent-built codebases tractable. When an implementing agent sees only its own module's contract — not the rest of the system, not the design doc, not adjacent modules — three things happen:

1. The agent cannot invent coupling that does not exist. It has nothing to couple to.
2. The agent cannot drift toward the system's overall purpose, which would otherwise bias every micro-decision.
3. Any incompleteness in the contract surfaces immediately, as a re-raise, rather than being smoothed over with plausible-looking code.

This makes context lean, makes contracts honest, and makes the resulting code locally verifiable. The framework around this skill is what physically enforces the context boundary; this skill's job is to make the agent inside that boundary behave correctly.

If this skill is being used **outside** the framework (no enforced isolation), behave as if isolation were enforced anyway. Treat anything outside the manifest, shared vocabulary, and project context as if it did not exist. The discipline is what produces the value, not the wall.

## What you have

You have been given exactly three things:

1. **A manifest** for the module you are implementing. This contains the contract: inputs, outputs, side effects, error semantics, behavioural guarantees, dependencies, referenced data shapes, acceptance criteria. The manifest is **language-neutral** by design — it does not tell you which language to write in.
2. **A shared vocabulary file** containing the data shapes and external systems your manifest refers to. Also language-neutral.
3. **A project context file** (`project_context.yaml`). This is where language, toolchain, repository style, directory layout, test framework, and code conventions are recorded. The user authored this; it is authoritative for every choice the manifest deliberately leaves to you.

The split is deliberate: the manifest tells you *what* to build (semantics, contract, behaviour). The project context tells you *how to express it* (language, idioms, file layout). Both are required; neither alone is sufficient.

You do not have, and must not seek, anything else. You do not know what the system is for. You do not know which other modules exist. You do not know who calls you. You do not see other modules' implementations, even of "the same" module's earlier version unless you were explicitly given it as a re-raise input. This is intentional.

## Where your output goes

Source files live at the repo root, in the layout the project context specifies. Not inside `.taniwha/`.

The dispatcher provides you with explicit target paths derived from the project context's `repo_style.module_layout` template. Write your source files to those paths exactly. Tests go where the project context's `test_layout` says (alongside source files, in a separate tests directory, or wherever the project conventions place them).

Your `notes.md` (the implementor's record of acceptance-criterion satisfaction) is the only output that goes inside `.taniwha/` — it is metadata about your work, not the work itself. The dispatcher will tell you the exact path.

If you find yourself uncertain about where a file should live, that is a project-context completeness question, not an implementation question. Re-raise rather than guessing at directory structure.

## Process

### 1. Read the manifest, shared vocabulary, and project context completely

Before writing any code, read all three documents end to end. The manifest is a contract — every clause is load-bearing. The shared vocabulary defines every shape and external system you are allowed to reference. The project context tells you which language and conventions to use.

### 2. Run the isolation check on yourself

Before writing code, ask: do I have any unanswered question whose answer is not in the manifest, shared vocabulary, or project context? Specifically, walk through:

- For each input: do I know its type, its validation rules, and what to do if it is invalid?
- For each output: do I know its exact shape and any constraints on its values?
- For each side effect: do I know which external system, which exact operation, and what to do on failure?
- For each error mode the manifest acknowledges: do I know how to signal it (using the project-context language's idioms) and what the caller will do?
- For each behavioural guarantee: do I know how to honour it? (Idempotency, ordering, atomicity, concurrency safety, resource bounds. The contract states the *property*; the project context informs *how* you achieve it.)
- For each dependency: do I know the contract of the dependency I am calling? (Not its implementation — only its contract.)
- For acceptance criteria: do I know how each one is checked, well enough to write code that will pass it?
- For file layout: do I know where my source files should live, what package/module name to use, where tests go?

If any answer is "no" or "I would have to guess", **stop and emit a re-raise**. Do not proceed. See "When to re-raise" below.

### 3. Implement the module against the contract, in the project's language

Now write the code. Some properties of how you write it:

- **Use the language and toolchain commands in the project context.** Not whatever language you'd reach for. If the project context says Python with `pytest` as the captured test command, your output is Python with tests runnable via that command. Read `project_context.toolchain.commands.test` (and `build`, `format`, `lint` as relevant) to know the project's invocation conventions; do not re-derive language-specific commands.
- **Implement exactly the contract. Nothing more, nothing less.** Do not add a "useful" extra method. Do not expose internal state for "debuggability". Do not add a logging statement that was not specified. Do not add caching that the contract did not call for. The contract is the surface area, full stop.
- **Honour every behavioural guarantee using the language's appropriate mechanism.** "Concurrent invocations are safe" means a thread-safe implementation in Java, a Send/Sync-bounded implementation in Rust, a goroutine-safe implementation in Go. Read the contract for the *property*, the project context for the *mechanism*.
- **Use only the dependencies the manifest declares.** Do not call other modules, services, or external systems that were not in the dependency list. If you find yourself wanting to, that is a re-raise — your contract is wrong.
- **Use only the data shapes from the shared vocabulary.** Do not invent ad-hoc shapes for inputs, outputs, or stored data. If you find a need for one not in the shared vocabulary, that is a re-raise — the shared vocabulary is incomplete.
- **Honour vocabulary sharing markers.** Each entry in the shared vocabulary has a `sharing` marker — `shared` or `local`. For any entry marked `sharing: shared`, you **must import the canonical version from `project_context.shared_types.package_path`**. You do not declare your own copy as a "placeholder for composition to reconcile later". You do not create a structurally compatible local type. You import from the canonical path, period. The orchestrator is responsible for ensuring the shared-types package exists before you run; if you find that the canonical path is empty or the package does not exist, that is a re-raise (`category: under_specified, suspected_source: project_context`). It is **never** a license to declare a placeholder. A permissive "import OR declare locally" rule produces a recovery loop where leaves have to be refactored after the fact — that's the failure mode this rule prevents.

  **Single-module exception.** If the project's structural tier is `single_module` and the vocabulary marks any entry as `sharing: shared`, that is a vocabulary defect, not a project_context defect. A single-module project has no second module to share with; `sharing: shared` is a category error. Re-raise with `category: under_specified, suspected_source: vocabulary` and quote the offending entries. Do not import from a phantom shared path; do not declare placeholders; surface the contract-derivation bug to the orchestrator so the vocabulary can be amended to `sharing: local`. This case should not occur if contract-derivation followed its rules, but the leaf is the last line of defence.

  - In Go: `import "<go_import_path>"` and use the canonical types by their package-qualified names.
  - In Python or TypeScript: import from the shared package by its canonical name.
  - The leaf does not construct concrete shared types itself unless its own contract is to *produce* shared types as outputs; if it consumes them as inputs, accept them at construction time or as method parameters.
- **For `sharing: local` entries, you can declare the type locally as you see fit** — name it appropriately for your language, place it inside your module, and use it without coordination with other modules. The marker tells you no other module references it.
- **Match error semantics to the language's idioms.** If the contract says "signals an error condition X", use the language's natural error mechanism (exceptions in Python/Java, returned errors in Go, Result types in Rust). If the contract is more specific, match the specificity. For shared error conditions (`sharing: shared` in the vocabulary), import the canonical name; for local errors, name them however fits your module's idioms.
- **Follow project-context conventions for naming, file layout, formatting, and code style.** If the context says `naming: snake`, then `code-generation` becomes `code_generation`. If it says formatter is `gofmt`, your output is gofmt-formatted. If `code_style_notes` says "no abbreviations", honour that.

### 4. Self-verify against acceptance criteria

For each acceptance criterion in the manifest, demonstrate (in comments adjacent to the relevant code, or in a separate verification note) how the implementation satisfies it. If you cannot demonstrate this for any criterion, either the implementation is wrong or the criterion is unmet — either way, do not declare the work done.

Where acceptance criteria are testable, write the test. Tests check the contract, not the implementation — they exercise inputs and observe outputs and side effects, never internal state.

### 5. Produce the output bundle

Output: the implementation file(s), tests, and a short note summarising how each acceptance criterion is satisfied. If any re-raise was emitted, output it instead of code.

## When to re-raise

Re-raise (see `references/re-raise-protocol.md`) instead of writing code whenever any of these is true:

- **The manifest is under-specified.** Something needed to implement correctly is not stated and cannot be derived. `category: under_specified`, `suspected_source: parent_contract`.
- **The manifest is internally inconsistent.** Two clauses of the contract contradict each other. `category: internally_inconsistent`.
- **The manifest is ambiguous in a way that admits materially different implementations.** `category: ambiguous_intent`.
- **Implementing correctly would require touching something outside the manifest's declared scope.** A dependency not listed, a data shape not defined, an external system not named. `category: out_of_scope`.
- **The shared vocabulary is missing a shape or system the manifest references.** This is technically `out_of_scope` from the implementor's view, but tag it specifically: the shared vocabulary, not the manifest, is the broken artefact.

Do not re-raise for things you can resolve from your own context: choice of internal data structures, choice of algorithm, naming of private functions, choice of standard-library calls. Those are implementation choices and they are yours to make.

## Refusing to expand scope

The most common failure mode of this skill is the agent "helpfully" doing more than the contract requires. Resist this directly. When you notice yourself about to:

- Add a parameter the contract did not specify
- Add a return field the contract did not specify
- Catch an error the contract said the caller handles
- Add a log line, metric, or trace not specified
- Add input validation beyond what the contract requires
- Add a side effect (file, log, cache, metric) not in the side-effects list
- Inline a piece of behaviour from a dependency, "for efficiency"
- Refactor a referenced data shape because it would be "cleaner"

…stop. None of these are yours to decide. If one of them is genuinely needed, the contract is wrong — re-raise.

This is not bureaucracy. In a compartmentalised system, the contract is the only thing the rest of the build trusts. An implementation that exceeds its contract is invisible to the rest of the system — until it is not, at which point it is a bug nobody can locate.

## Quality checks before finishing

1. **Every input is handled as the contract specifies.** Validation, defaults, rejection of invalid values — all match the manifest.
2. **Every output matches the declared shape.** No extra fields, no missing fields, no off-by-one type variations.
3. **Every declared side effect happens; no undeclared side effects happen.** Audit your code: every write, every call, every emission, is in the side-effects list.
4. **Every error mode is signalled as the contract specifies.** Audit: every `raise`, `return Err`, `throw`, status code, matches a contract clause.
5. **Every behavioural guarantee is honoured by code you wrote deliberately.** You can point to the lines that make idempotency, ordering, concurrency safety hold. If you cannot point at them, they probably do not hold.
6. **No dependency is called that is not in the manifest's dependency list.** Including "trivial" things — no surprise log emitters, no surprise metrics, no surprise filesystem touches.
7. **All acceptance criteria are demonstrably satisfied.** You can show why each one holds.

## Common failure modes

- **Inferring the system's purpose from the manifest's wording and biasing toward it.** The manifest is mechanism. Stay there.
- **"While I'm here" additions.** A perfect indicator that the change does not belong to this module. Re-raise or discard.
- **Defensive programming beyond the contract.** Validating things the contract did not say to validate, catching errors the contract did not say to catch. Each addition is invisible coupling — the caller now depends on behaviour the contract does not promise.
- **Adapting input shapes "to be more convenient".** The shared vocabulary is the boundary. If a shape is awkward to consume, re-raise to fix the shape, not your local copy of it.
- **Optimising for a hypothetical caller.** You do not know your callers. The contract is what they rely on. Make the contract true; do not speculate about who reads what.
- **Silently weakening a guarantee under pressure.** The manifest says idempotent and you cannot easily make the operation idempotent — so the implementation is "almost" idempotent, with a comment. This is the worst possible outcome: the contract lies and nobody knows. Re-raise.

## Relationship to other skills

Input: one manifest plus the shared vocabulary, both produced by the `contract-derivation` skill.

Output: an implementation that satisfies the manifest, or a re-raise. The implementation is consumed by the `composition` skill, which wires this module together with another module's implementation under a parent contract. The composer trusts the manifest absolutely — your job is to make that trust earned.

## See also

- `references/re-raise-protocol.md` — the format for surfacing under-specification, ambiguity, and scope violations.
