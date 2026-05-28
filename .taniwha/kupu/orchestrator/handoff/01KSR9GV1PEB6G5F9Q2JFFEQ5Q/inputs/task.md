# Task — contract-derivation (module: hooks-grok-exit)

Derive a single module contract/manifest for the module **hooks-grok-exit** from
the approved design doc v3 (`inputs/design_doc_v3.md`).

The contract must be usable by an isolated implementor who has never read the
design doc — it must stand alone. Carry **AC1–AC7 verbatim** as the acceptance
criteria section, numbered consistently with the design doc. Language-neutral
where the schema requires, but grounded in Rust/cargo conventions
(`inputs/project_context.yaml`): single-file module (`src/hooks.rs`), tests in
`#[cfg(test)] mod tests` blocks or under `tests/`, no new crate dependencies,
`Result<T, String>` for fallible functions.

This is a single_module tier design — no shared vocabulary, no composition
contract. Produce exactly one contract document.

Key constraints from the design doc:
- Side-effecting entry point is `handle_stdin`; `process::exit` is called only
  from there (the inner exit-code computation returns a value, never calls
  `process::exit` directly).
- Reuse three existing functions in `src/hooks.rs` as-is: `detect_host`,
  `emit_grok_decision`, `emit_claude_decision`.
- `desired_exit_code` is a pure function: `2` when `host==Grok AND
  event_type==PreToolUse AND deny_outcome==true`; `0` otherwise.
- Error path (malformed/oversize stdin) on Grok host: emit Grok-shaped deny JSON
  and exit 2. On Claude host: unchanged (Claude-shaped deny, exit 0).
- stdout must always be flushed before process exit.

Output: a single contract markdown `contract_manifest.md` (or per your skill's
naming) written to the handoff `outputs/` directory. Emit a re_raise.yaml instead
ONLY if the design is genuinely ambiguous.
