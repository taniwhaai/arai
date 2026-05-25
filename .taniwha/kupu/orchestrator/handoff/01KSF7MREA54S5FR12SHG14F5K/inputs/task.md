# Task — design doc v2 for issue #113 (prompt-text collector)

Produce a short, structural design doc for the Arai prompt-text collector.

## Inputs (under `inputs/`)

- `brief.md` — Brief v4. Authoritative. Source of truth for scope, ACs,
  in/out-of-scope, and the 4 open questions.
- `project_context.yaml` — Rust 2021 / cargo / single_package / `src/{module}.rs`
  / `#[cfg(test)] mod tests` convention.

## Required output

Write to `outputs/design_doc.md`. A single Markdown file beginning with this
front-matter:

```yaml
---
version: 2
parent_brief_version: 4
tier: single_module
---
```

## Hard constraints

- **Structural tier:** `single_module`. The brief says so. Do NOT propose
  multi-module or a composition tree.
- **No Rust code.** No struct definitions, no fn signatures, no closures.
  Language-aware but implementation-neutral. Contract-derivation owns
  Rust specifics.
- **Resolve all four open questions in the brief** — record your answer
  as a *decision* in the design body, not as a continuing open question.
  Brief's recommended answers are typically the right call; deviate only
  with explicit justification.
- **Preserve all of the brief's "Out of scope" prohibitions verbatim** in
  the design's own Out-of-scope section. Don't soften or omit them.
- **Test surface section must cover all 9 ACs from the brief.** Especially
  AC5 (no hook mutation) and AC6 (no network egress) — these need
  structural verification approaches in the test surface.

## Required structure

1. Front-matter (above).
2. `## Structural tier` — single_module + justification.
3. `## Purpose` — one paragraph.
4. `## External boundaries` — what crosses the module wall and in which
   direction (e.g. stdin: inbound prompt text via UserPromptSubmit;
   filesystem: outbound receipts via existing audit pipeline).
5. `## Modules` — exactly one entry: `prompt_collector`. Within it:
   responsibilities, NOT responsibilities, inputs, outputs, side effects,
   error semantics, behavioural guarantees, dependencies.
6. `## Caller-site change in src/hooks.rs (UserPromptSubmit branch)` —
   describe the call insertion point and the contract that the call site
   MUST NOT mutate the existing hook response.
7. `## Data shapes` — `PromptRule`, `PromptMatchReceipt`. Describe fields
   in plain English (name + type-category + constraint). NO Rust types.
8. `## Decisions made for v1` — your resolution of the brief's 4 open
   questions, one paragraph each. Format: "**<question name>**: <chosen
   answer>. <one-sentence justification>."
9. `## Out of scope` — preserve the brief's prohibitions verbatim plus
   any additional ones implied by the chosen answers.
10. `## Test surface` — coverage required for each AC1..AC9 from the brief.
    For each AC, name the test class (unit/integration/structural-grep) and
    what it asserts. Add tests for behaviour the brief implies but doesn't
    enumerate (e.g. determinism of `prompt_hash`).

## Boundaries

- Do NOT amend brief v4. If you find the brief genuinely ambiguous, write
  a re-raise `outputs/re_raise.yaml` per the re-raise protocol and produce
  no design doc. Use this for genuine ambiguity, NOT for things you could
  defensibly decide.
- Do NOT reference design v1 (it covers a different module, in a different
  cycle).
- Do NOT include speculation about v2/Kete features. The design is for v1
  Arai only.
- Do NOT add new acceptance criteria. The 9 ACs in the brief are the
  scope; tests beyond them are allowed but cannot graduate to ACs.

Follow your skill. Read inputs/brief.md and inputs/project_context.yaml
carefully. Produce a short, dense design doc.
