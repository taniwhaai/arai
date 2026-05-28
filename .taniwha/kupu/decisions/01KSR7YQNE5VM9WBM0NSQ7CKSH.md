---
schema_version: 1
id: 01KSR7YQNE5VM9WBM0NSQ7CKSH
decided_at:
  iso: 2026-05-28T21:31:11.000Z
  filename: 20260528T213111000Z
kind: scope_change
summary: "Begin new slice — issue #122 Grok TUI native deny via exit code 2; dispatch design-doc for v3"
affects:
  - kind: design_doc
    id: design
    from_version: 2
    to_version: null
triggered_by: 01KSR7WM5VKW07B3GRNZHT323G
---

# Decision: Begin new slice — issue #122 Grok TUI native deny via exit code 2

## Context

Brief v5 records GitHub issue #122 scoped to the exit-code sub-task. The prior slice
(brief v4 / issue #113 — prompt-collector) is complete: PR #119 merged, build_completed
event 01KSGCVH3T2PD92X98ACPZR3K6 recorded. Design doc v2 is scoped to prompt-collector
and does not cover this slice.

The build_started event 01KSR7WM5VKW07B3GRNZHT323G marks the start of v5.

## Resolution

Dispatch a design-doc subagent (handoff 01KSR7YKX36401D3325JJDMXSV) to produce design
doc v3 for the Grok exit-code deny slice.

## Rationale

The brief is explicit about scope and tier (single_module, one file: src/hooks.rs).
No new project context questions are needed — language, toolchain, and repo conventions
are already captured in project_context v1. The design-doc agent confirms the
single_module tier and produces the module definition covering the hooks.rs exit-code-2
behaviour: handle_stdin_impl returns i32 exit code, handle_stdin calls process::exit(2)
on Grok+Block deny path.

## Consequences

After design-doc returns and user approves v3:
  1. Dispatch contract-derivation → contract for hooks-grok-exit module
  2. Dispatch leaf-implementation → modify src/hooks.rs
  3. Dispatch verifier → cargo test --test hooks_safety + cargo test (AC1–AC7)
