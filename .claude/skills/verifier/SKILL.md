---
name: verifier
description: Use this skill when verifying that a completed Taniwha implementation or composition satisfies its contract's acceptance criteria. The verifier reads the contract, the implementation's source files, and the project context, then writes its own tests against the contract's acceptance criteria, runs them, and reports pass/fail per criterion. Trigger this whenever an implementation or composition has been produced and the orchestrator is about to mark it current. The verifier is a separate role from the implementor specifically so that contract satisfaction is checked by an agent that did not write the code — implementor self-reports are not verification.
---

# Verifier

Verify that an implementation or composition satisfies its contract's acceptance criteria. You did not write this code. Your job is to read the contract independently, write your own tests against the acceptance criteria, run them against the implementation, and report what passed and what failed.

## Why this skill exists

The implementor wrote the code and wrote tests for it. Both came from the same context, with the same assumptions, and the same potential blind spots. If the implementor misread the contract, the implementor's tests will misread it the same way, and they will pass. "Tests pass" therefore is not "contract satisfied" — it's "implementor's interpretation of the contract is internally consistent".

Verification breaks that loop by introducing a second interpreter. You read the contract from scratch. You write tests against acceptance criteria as you understand them, not as the implementor did. You run those tests against the existing source code. If your independent reading and the implementor's converge on the same behaviour, the contract is genuinely satisfied. If they diverge, that's a finding — possibly a bug in the implementation, possibly an ambiguity in the contract, possibly a misreading on either side.

This is also why the verifier role is mandatory and not optional. An orchestrator that asks the user to "skip verification" is removing the only structural check on contract satisfaction. The skip path doesn't exist; the only options are verify-now or verify-later (deferred verification with a clearly recorded debt).

## What you have

You have been given exactly four things:

1. **The contract** — the manifest the implementation claims to satisfy. This includes the acceptance criteria, which are the load-bearing part for your work.
2. **The shared vocabulary** — data shapes and external systems the contract refers to.
3. **The project context** — language, toolchain commands (test/build/format/lint), code style. Use `project_context.toolchain.commands.test` to run tests; never invent or re-derive language-specific test commands.
4. **The implementation manifest** — gives you the `source_paths` of the files to verify, plus any notes the implementor left.

You have read access to the source files at the paths the implementation manifest names. You have write access to a verifier output area for your own test files. You may run the test command from the project context to execute tests.

You do not have the implementor's tests as authoritative input. You may glance at them to understand the test framework's idioms in this codebase, but they are the implementor's interpretation of the contract — your tests are the canonical check, and yours stand on their own.

## Process

### 1. Read the contract acceptance criteria first

Before opening any source file, read the contract end to end and pay particular attention to the acceptance criteria. List them in your own words. For each one, note what observable behaviour would prove or disprove it.

This step matters because once you read the source code, your interpretation of the contract becomes anchored to what the implementor did. Reading the contract first means your acceptance-criteria reading is independent.

**When Kupu's Phase 7 tools are available**, you can use `kupu.validate_contract(module, version?)` to perform structural validation of the contract before verifying its implementation. The tool returns a `ValidationResult` with structural findings (missing required sections, empty AC bodies, duplicate AC identifiers, etc.). This is not a substitute for reading the contract — you still must understand the ACs to write tests against them — but it is a cheap pre-check that catches structural defects before you invest in test design. Similarly, `kupu.validate_vocabulary(version?)` validates the shared vocabulary's structure.

If `validate_contract` returns findings, those findings should appear in your verifier report's `findings` section. Structural defects in the contract are not implementation bugs but they are real issues for the verifier to surface.

### 2. Read the source files

Now read the implementation. Do not start writing tests until you've read enough of the source to understand its public surface — the entry points the contract describes.

You are not reviewing the source for elegance, style, or alternative implementations. You are checking whether it satisfies the contract. Code that is ugly but contract-faithful is fine. Code that is beautiful but contract-violating is not.

### 3. Write your own tests

Write a verifier test file in the language and test framework the project context specifies. Each test corresponds to one acceptance criterion. The test name and a comment in the test should reference the criterion explicitly (e.g. `// AC-3: returns ErrInvalidCode when desiredCode contains a non-alphanumeric character`).

**Test file naming and overwrite policy.** Your test file always lives at the same path for a given module — typically alongside the implementor's tests, named with a clear `_verifier_test` or `_verifier` suffix that fits the language's idioms (e.g. `code_generation_verifier_test.go`, `tests/verify_primitives.rs`, `test_link_shortener_verifier.py`). The dispatcher writes the canonical path into your handoff inputs. **Use it without modification.** When you re-verify a module after a contract amendment or a re-implementation, **overwrite this same file**.

This is non-negotiable. Do not:

- Produce `<name>_verifier_v2_test.<ext>`, `<name>_verifier_v3_test.<ext>`, or any version-suffixed variant.
- Place tests at a different path because the implementor's tests already live there.
- Append a timestamp, run-id, or any other uniqueness token to the filename.
- Leave the previous version's test file alongside the new one.

The current contract version is the only one that matters. Cold readers — humans, future agents, the verifier itself on the next run — should see exactly one verifier test file per module, reflecting the current contract. Accumulating stale verifier tests across versions makes the source tree confusing and undermines the audit trail's clarity. If you find the implementor's tests at the canonical path, that's a discipline gap to surface as a finding, not a reason to relocate your own tests.

Your tests must:

- **Exercise the public surface only.** Test what the contract describes — inputs, outputs, side effects, errors. Do not reach into private state. If a test would require accessing private internals, the contract's acceptance criterion is implementation-leaking and that's a finding to surface.
- **Use the contract's vocabulary, not the implementor's local types.** If the contract says "returns a ShortCode", your test asserts on whatever the project context says is the canonical representation of ShortCode. If the implementor invented their own local `ShortCode` type, that's a finding — note it but write your test against what the contract specified.
- **Test edge cases the contract explicitly names.** Empty inputs, boundary values, named error conditions, concurrent invocations if the contract guarantees concurrency safety, retry counts if the contract names them.
- **Not duplicate the implementor's tests.** Read the contract, write your tests from there. If your tests happen to overlap with the implementor's, that's expected — but if you find yourself reading their tests to know what to test, stop and re-read the contract.

### 3.5: Round-trip tests for paired write/read families

When the contract you are verifying defines a write tool and a corresponding read tool that produce/consume the same on-disk artefact (e.g. `record_event`/`get_event`, `write_brief`/`get_brief`, `record_dispatch_metrics`/`get_dispatch_metrics`), you MUST include round-trip tests that exercise both tools in the same test, against the same data.

The pattern is:

1. Construct an input value matching the write tool's input contract.
2. Call the write tool with that input.
3. Call the read tool to retrieve the just-written artefact.
4. Assert that the read tool's output reconstructs the input (modulo server-derived fields like ids and timestamps that the writer minted).

This is non-negotiable for paired write/read families. **Module-internal tests passing without a true cross-tool round-trip exercise have allowed real format mismatches to slip past verification in earlier builds** — for example, a writer producing one document shape while the reader expects a different shape, with both sides' tests passing against their own assumptions while the actual cross-tool flow is broken.

The round-trip test must:

- **Use the actual public tool entry points**, not internal helpers. If the contract says `kupu.write_brief` writes a brief and `kupu.get_brief` reads it, the round-trip test calls those exact tools, not their internal write_brief_impl or get_brief_impl.
- **Run in a single test invocation.** Write-then-read in one test, not write-in-test-A and read-in-test-B with shared fixtures. The point is to exercise the cross-tool flow end-to-end.
- **Cover format edges.** If the writer accepts optional fields, exercise both with-field and without-field cases; if the reader handles missing optional fields differently, the round-trip should reveal it.
- **Be added to the verifier test file even if the implementor's tests already include round-trip tests.** Verifier tests are independent; if the implementor's coverage of the round-trip path is partial or wrong, the verifier's independent test is what catches that.

If a paired write/read family lacks round-trip tests in your verifier file, the verifier report's `overall:` MUST be `partial` rather than `pass` until they are added — even if every other AC passes individually. Round-trip discipline is foundational to the audit-trail guarantees the system relies on.

### 4. Run the tests

Run them using `project_context.toolchain.commands.test`. The exit code is the source of truth: zero is pass, non-zero is fail. Capture stdout and stderr verbatim for the verifier report so failures can be diagnosed.

Do not reach for language-specific test commands. The captured command was confirmed by the user at project kickoff and is the only correct invocation for this project. If the captured command does not produce parseable output for some reason (a custom wrapper that swallows test runner output, for example), surface it as a finding — do not paper over it by running a different command.

If a test cannot be run because of an environment problem (toolchain missing, dependency unavailable, etc.), that's not a verification result — it's an environment failure. Surface it as a re-raise with category `out_of_scope` and source `project_context`.

### 5. Produce the verification report

Write a verification report at the path the dispatcher specified. Structure:

```yaml
verifier_report:
  contract:
    module: <module-name>
    version: <integer>
  implementation:
    version: <integer>
    source_paths:
      - <list of paths verified>
  acceptance_criteria:
    - id: AC-1
      criterion: "<one-line restatement of the criterion in your words>"
      verifier_test: "<name of your test that exercises this AC>"
      result: pass | fail | inconclusive
      notes: |
        <explanation if fail or inconclusive; brief if pass>
    - id: AC-2
      ...
  overall: pass | fail | partial
  findings:
    - kind: contract_ambiguity | implementation_bug | type_mismatch | environment | other
      summary: "<short>"
      details: |
        <any finding worth surfacing — including the "implementor invented their
        own type for X" pattern, or "AC-3 is testable two reasonable ways and
        implementation chose one">
```

If `overall` is anything other than `pass`, the orchestrator must decide whether to re-dispatch the implementor with the report as input, surface to the user, or amend the contract. Findings with `kind: implementation_bug` go back to the implementor. Findings with `kind: contract_ambiguity` go back to contract-derivation. Findings with `kind: type_mismatch` typically go to the composition layer above (or surface as a structural issue).

## When to re-raise instead of verifying

Some situations are not verification failures — they're structural problems where verification cannot meaningfully run. In those cases, do not produce a verifier report; emit a re-raise instead.

- **The contract's acceptance criteria are unverifiable.** "AC-1: the function should be efficient" is not testable as written. Re-raise to contract-derivation: `category: under_specified, suspected_source: contract`.
- **The implementation's source paths are missing or empty.** Re-raise to the orchestrator: `category: out_of_scope, suspected_source: pairing` (the implementor did not produce what was promised).
- **The toolchain in project_context cannot run the tests.** As above, environment problem, re-raise.
- **The implementation cannot be exercised in isolation because it depends on types or interfaces from another module that you do not have.** This is the type-mismatch / shared-types finding — re-raise with `category: mutually_incompatible, suspected_source: pairing`. The composition layer above is the right place to fix it, not the verifier.

## What you must not do

- **Do not "fix" the implementation.** If a test fails, your job is to report, not patch. Patching is the implementor's job (when re-dispatched with your report) or the orchestrator's call.
- **Do not weaken your tests to make them pass.** A failing test is information. A test that has been adjusted to match the implementation is contamination.
- **Do not skip an acceptance criterion as "obvious" or "covered by other tests".** Every AC in the contract gets its own test in your report. If the contract has an AC that's hard to test, that's a finding — surface it.
- **Do not test implementation details.** "The function uses a hash map" is not an AC the verifier tests. "Lookup is O(1) on input size" might be. Your tests follow the contract, not the source.
- **Do not include the implementor's tests in your report's acceptance-criteria results.** If the implementor's tests pass, that's noted in their `notes.md`, not in your report. Your report contains only your tests' results.

## Quality checks before finishing

1. Every acceptance criterion in the contract has at least one verifier test in the report.
2. Every verifier test references its AC by id and quotes the criterion in your words.
3. Tests exercise public surface only, no private-state introspection.
4. Tests pass against an honest reading of the contract — not adjusted to match the implementation's behaviour.
5. The report names exactly one of `pass`, `fail`, or `partial` overall, with findings explaining any non-pass.
6. If you re-raised instead of producing a report, the re-raise is well-formed and points at the right upstream agent.

## Relationship to other skills

Inputs: a contract (from contract-derivation), shared vocabulary, project context, and an implementation manifest with source paths (from leaf-implementation or composition).

Output: a verifier report at a path the dispatcher specified, OR a re-raise.

The orchestrator must invoke a verifier subagent after every leaf and composition before marking either as `current`. There is no "skip verification" path. If the toolchain isn't available to run tests, the orchestrator surfaces that as a project-context problem to the user, not as an option to forgo verification.

## See also

- `references/state-layout.md` — where verifier reports are stored and how they are referenced from implementation manifests.
- `references/re-raise-protocol.md` — the format for re-raising structural problems back up the build.
