# Contract-derivation task — prompt-collector

Derive a contract for the single module described in design doc v2:
the `prompt-collector` module (`src/prompt_collector.rs`).

The design doc fully specifies:
- Module boundary, purpose, and external boundaries (stdin, audit log, rule source)
- Data shapes: `PromptRule` (pattern, label) and `PromptMatchReceipt` (6 fields)
- Behavioural guarantees (deterministic, concurrent-safe, no side effects, skip-invalid-regex)
- Error semantics (invalid regex → skip silently, return skip count)
- Caller-site wiring in `src/hooks.rs` (UserPromptSubmit branch, post-response, no mutation)
- Acceptance criteria AC1–AC9 and additional implied tests

The contract must be language-neutral. Derive it directly from the design's
module section and acceptance criteria. Do not include Rust syntax.

Module: `prompt-collector`
Design version: 2
Tier: `single_module` (no vocabulary, no sibling contracts needed)

The prior `base-directory-resolution` v1 contract is stable and must NOT
be amended. Only the `prompt-collector` contract is needed.

Output a single contract Markdown file. Begin with a YAML front-matter block:

```
---
module: prompt-collector
version: 1
parent_design_version: 2
---
```

Then the contract body structured as: purpose, inputs, outputs, behavioural
guarantees, error semantics, acceptance criteria, and out-of-scope.
