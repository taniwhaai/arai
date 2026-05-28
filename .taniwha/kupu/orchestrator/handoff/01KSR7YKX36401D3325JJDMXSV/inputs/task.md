# Task — design-doc (Arai design v3)

Produce design doc v3 for the Arai project — slice: "Grok TUI native deny via
exit code 2" (GitHub issue #122, exit-code sub-task).

The brief is at `inputs/brief.md` (canonical: kupu/brief/v5.md). Read it
carefully. The scope is a single source file: `src/hooks.rs`. No new modules, no
new crate dependencies, no changes to parser or store.

The design doc must:
1. Declare tier: single_module (the brief is explicit about this).
2. Name the single module (e.g. `hooks-grok-exit`) — a name that precisely
   captures the hooks.rs exit-code-2 change.
3. Define the module's external boundaries, inputs, outputs, and side effects.
   (Side effects of note: it writes to stdout and sets the process exit code;
   it reads host via env vars. It must NOT change the match pipeline.)
4. Enumerate acceptance criteria AC1–AC7 from the brief as the module's ACs.
5. State what is NOT in scope (no changes outside src/hooks.rs, no new
   abstractions, no changes to match pipeline/parser/store).

The project context is at `inputs/project_context.yaml`. It establishes: Rust
(edition 2021), single Cargo crate, modules are flat .rs files under src/, tests
are `#[cfg(test)]` blocks or integration tests under tests/. Fallible library
functions use `Result<T, String>`.

Output: a design doc markdown file `design_doc.md` following the Taniwha
design-doc schema — YAML frontmatter with version, parent_brief_version, tier;
then body sections: Structural tier, Purpose, External boundaries, Modules,
Acceptance criteria, Out of scope. Emit `re_raise.yaml` instead ONLY if the
brief is genuinely ambiguous.
