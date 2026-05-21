# Canonical rules-file spec (v1)

**Status:** draft for review.
**Scope:** resolves [#75](https://github.com/taniwhaai/arai/issues/75) (format spike), [#76](https://github.com/taniwhaai/arai/issues/76) (schema design), and [#79](https://github.com/taniwhaai/arai/issues/79) (rule-pack accommodation).
**Out of scope:** `arai migrate` (#78), `arai sync` (#77), rule-pack publication.

---

## Why a canonical file

Today, Arai reads rules from whatever instruction file each tool happens to use — `CLAUDE.md`, `.cursorrules`, `.windsurfrules`, `.github/copilot-instructions.md`. Each format has its own conventions, and the rules drift: a `should not` added to `CLAUDE.md` to fix a Claude misbehaviour never makes it to Cursor, because the maintainer forgot the second file existed.

A canonical `arai.toml` (the *one* file you commit, version-controlled, schema-checked) becomes the source of truth. `arai sync` (#77) writes the per-tool files. `arai migrate` (#78) bootstraps the canonical file from whatever rules Arai's parser can already extract.

This document fixes the file's format and schema so those two pieces of work can begin.

---

## Part 1 — Format spike (resolves #75)

The brief recommended TOML; the spike confirms it. Below are the same four rules authored in both formats so a reader can judge.

### Sample rule set: alembic + git push + secrets + test-before-commit

**TOML:**

```toml
[meta]
schema_version = 1

[[rule]]
id = "alembic-no-handwrite"
description = "Never hand-write Alembic migrations — let `alembic revision --autogenerate` produce them"
when.tool = ["Write"]
when.path = ["**/migrations/versions/*.py", "**/alembic/versions/*.py"]
then.action = "block"
then.message = "Hand-writing migrations bypasses schema-drift detection. Use `alembic revision --autogenerate -m <slug>` instead."
severity = "block"

[[rule]]
id = "git-force-push-main"
description = "Refuse force-pushes targeting main or master"
when.tool = ["Bash"]
when.command_pattern = "^git push.*--force.*\\b(main|master)\\b"
then.action = "block"
then.message = "Force-pushing to a protected branch overwrites peer history. If you really mean it, run the command yourself outside the agent."
severity = "block"

[[rule]]
id = "no-committed-secrets"
description = "Block any Write that looks like it contains an AWS access key"
when.tool = ["Write", "Edit"]
when.content_pattern = "AKIA[0-9A-Z]{16}"
then.action = "block"
then.message = "That looks like an AWS access key. Move it to environment variables or a secret manager."
severity = "block"

[[rule]]
id = "test-before-push"
description = "Encourage running tests before pushing"
when.tool = ["Bash"]
when.command_pattern = "^git push"
when.session_lacks = ["cargo test", "pytest", "npm test"]
then.action = "warn"
then.message = "No test invocation recorded in this session. Consider running tests before pushing."
severity = "inform"
```

**YAML:**

```yaml
meta:
  schema_version: 1

rules:
  - id: alembic-no-handwrite
    description: Never hand-write Alembic migrations — let `alembic revision --autogenerate` produce them
    when:
      tool: [Write]
      path:
        - "**/migrations/versions/*.py"
        - "**/alembic/versions/*.py"
    then:
      action: block
      message: >
        Hand-writing migrations bypasses schema-drift detection.
        Use `alembic revision --autogenerate -m <slug>` instead.
    severity: block

  - id: git-force-push-main
    description: Refuse force-pushes targeting main or master
    when:
      tool: [Bash]
      command_pattern: "^git push.*--force.*\\b(main|master)\\b"
    then:
      action: block
      message: >
        Force-pushing to a protected branch overwrites peer history.
        If you really mean it, run the command yourself outside the agent.
    severity: block

  - id: no-committed-secrets
    description: Block any Write that looks like it contains an AWS access key
    when:
      tool: [Write, Edit]
      content_pattern: "AKIA[0-9A-Z]{16}"
    then:
      action: block
      message: That looks like an AWS access key. Move it to environment variables or a secret manager.
    severity: block

  - id: test-before-push
    description: Encourage running tests before pushing
    when:
      tool: [Bash]
      command_pattern: "^git push"
      session_lacks: [cargo test, pytest, npm test]
    then:
      action: warn
      message: No test invocation recorded in this session. Consider running tests before pushing.
    severity: inform
```

### What the spike found

**TOML wins.** Three reasons that matter in practice:

1. **Indentation isn't load-bearing.** The YAML samples above need careful attention to where `when:` and `then:` indent — paste into a different editor and reflow can silently change meaning. Non-technical contributors hit this. TOML doesn't have that surface.
2. **No quoting ambiguity on patterns.** `command_pattern: "^git push.*--force.*\\b(main|master)\\b"` in YAML needs both outer quotes *and* the right escaping discipline; YAML's "is this a string or a multi-doc anchor or a tag" parsing is fiddly when the value contains `&`, `*`, `:`, `<`, `>`, `|`, `!`, or unquoted leading `-`. TOML strings are just strings.
3. **Comments are first-class in both, but TOML's `#` matches the existing CLAUDE.md / `.cursorrules` convention** users are already used to. The cognitive switching cost is lower.

YAML's only meaningful win is multi-line strings (`>` folded scalar). TOML handles this with `"""..."""` blocks; that's enough. There is no scenario where YAML is materially easier for a hand-authoring contributor.

**Anchors and includes:** YAML's `&anchor` / `<<: *anchor` looks useful for rule packs (factor a shared `when.path` set and reference it), but the moment a contributor needs to read someone else's anchored YAML they have to learn the syntax. The TOML equivalent is just naming a `[fragments.alembic_paths]` table and referencing it from a code-side composition step (`extends = ["packs/alembic"]`, expanded by `arai sync`). The expansion is explicit, which is the right tradeoff for a policy file.

**Decision:** `arai.toml`. The schema below assumes TOML throughout.

---

## Part 2 — Schema (resolves #76)

The v1 schema fixes the minimum needed to make `arai sync` (#77) and `arai migrate` (#78) implementable. Anything not listed is deliberately out of scope for v1.

### Top-level layout

```toml
[meta]
schema_version = 1               # required; bump on incompatible change
project = "arai"                 # optional; pure documentation

# Optional — pack references (see Part 3). Each entry is a URL or
# shorthand recognised by Arai's trust list (`arai trust --add`).
extends = [
    "taniwhaai/rules-fastapi-alembic@v1.0.0",
]

# Rules are an array of inline tables.  Order doesn't matter for matching;
# `arai sync` writes per-tool files in id order.
[[rule]]
id = "..."
...
```

### Per-rule fields

| Field | Type | Required | Notes |
|---|---|---|---|
| `id` | string | yes | Stable, kebab-case, project-unique. Survives renames; appears in audit log. |
| `description` | string | yes | Free-form prose. The human-readable "why this rule exists". Not shown to the agent. |
| `when.tool` | `[string]` | yes | Arai tool names: `Bash`, `Write`, `Edit`, `NotebookEdit`, `Read`, `Glob`, `Grep`, ... or `"*"`. |
| `when.path` | `[string]` | no | Glob patterns matched against the tool's target path (Write/Edit/Read). Multiple = OR. |
| `when.command_pattern` | string (regex) | no | Regex over `Bash` command strings. Mutually exclusive with `when.path`. |
| `when.content_pattern` | string (regex) | no | Regex over file content being written (Write) or the patch being applied (Edit). |
| `when.session_lacks` | `[string]` | no | Strings that must NOT have appeared in any Bash invocation in this session. Used for prerequisite rules (e.g. "tests before push"). |
| `then.action` | `"block" \| "warn" \| "allow"` | yes | What Arai does on match. `block` denies the tool call; `warn` injects context; `allow` is an explicit positive (overrides a more general rule, see "exceptions" below). |
| `then.message` | string | yes | Shown to the agent as `additionalContext` (warn) or `deny.reason` (block). Plain text; multi-line OK. |
| `severity` | `"block" \| "warn" \| "inform"` | yes | Routing for the existing severity-aware enforcement in `intent.rs`. Usually matches `then.action`; `severity = "inform"` with `then.action = "warn"` is a soft nudge. |
| `expires` | string (`YYYY-MM-DD`) | no | Mirrors the existing `(expires YYYY-MM-DD)` annotation; rule is silently dropped after the date. |

### Exceptions and overrides

A single `[[rule]]` block expresses one positive or negative rule. To say "Edit allowed but Write blocked for a path", write two rules and let action selection resolve them:

```toml
[[rule]]
id = "alembic-no-handwrite"
when.tool = ["Write"]
when.path = ["**/migrations/versions/*.py"]
then.action = "block"
severity = "block"
then.message = "..."

[[rule]]
id = "alembic-edits-fine"
when.tool = ["Edit"]
when.path = ["**/migrations/versions/*.py"]
then.action = "allow"
severity = "inform"
then.message = "(no-op — editing existing migrations is fine)"
```

Conflict resolution: most-specific wins; ties resolved by `action` priority `block > warn > allow`. This matches Arai's existing severity ordering and avoids inventing a new precedence model.

### Worked example: alembic end-to-end

A complete `arai.toml` that produces the same enforcement as today's prose CLAUDE.md `Never hand-write Alembic migrations` rule, including the "editing existing migrations is fine" exception, looks like:

```toml
[meta]
schema_version = 1

[[rule]]
id = "alembic-no-handwrite"
description = "Force migrations through `alembic revision --autogenerate`"
when.tool = ["Write"]
when.path = ["**/migrations/versions/*.py", "**/alembic/versions/*.py"]
then.action = "block"
then.message = """\
Hand-writing migrations bypasses schema-drift detection.
Use `alembic revision --autogenerate -m <slug>` instead — \
it inspects the SQLAlchemy models and emits the diff for you."""
severity = "block"

[[rule]]
id = "alembic-edits-fine"
description = "Editing an already-generated migration is fine — only creation is restricted"
when.tool = ["Edit"]
when.path = ["**/migrations/versions/*.py", "**/alembic/versions/*.py"]
then.action = "allow"
then.message = "(no-op — explicit override of alembic-no-handwrite for Edit)"
severity = "inform"
```

`arai sync` will fan this out to `CLAUDE.md`, `.cursorrules`, etc., using each tool's preferred conventions. The `id` is preserved in audit-log entries so post-hoc analysis (`arai audit --rule alembic-no-handwrite`) joins cleanly across tools.

---

## Part 3 — Rule-pack accommodation (resolves #79)

Rule packs (FastAPI + alembic + pytest; Next.js + Prisma; etc.) are explicitly out of scope for v1. But the schema must not need a breaking change when packs ship. Three points to verify:

1. **`extends` field is reserved at the top level.** Already shown above. The array contains entries that are either URLs (HTTPS only, matching `arai:extends` precedent in `src/extends.rs`) or shorthand identifiers resolved against a future registry. v1 parser MUST recognise this field and either inline-expand it (when packs ship) or warn-and-ignore (now). The latter means a user can write `extends = []` today without breakage.

2. **Pinned-in-canonical vs inherited-from-pack** is distinguished by the `id` namespace. Pack-imported rules use prefixed ids (`fastapi-alembic.no-handwrite`); locally-pinned overrides drop the prefix (`my-no-handwrite`) and `then.action`/`severity` from the local entry wins over the pack version with the same suffix. No new field needed — convention only.

3. **Trust model for packs** reuses the existing `arai trust --add` list from #29 / `src/extends.rs`. URL packs are subject to the same HTTPS + 24h-cache + 512 KB cap. Shorthand packs (`taniwhaai/rules-fastapi-alembic`) resolve via a future registry; v1 schema doesn't commit to one, and the parser MUST surface a "shorthand not yet supported" error if it sees one. No silent failure.

**Conclusion for #79:** the v1 schema accommodates packs without a breaking change. The only thing v1 has to *do* is reserve the `extends` top-level field name and define id-prefix conventions. Both are in this spec.

---

## What ships in v1

1. The `arai.toml` file format (TOML, per Part 1).
2. The schema defined in Part 2: per-rule `when` / `then` / `severity` / `expires`.
3. A reserved `extends` field at the top level and a documented id-prefix convention (Part 3).
4. Parser warn-and-ignore for any field not in the schema (forwards compatibility for v2 additions).

What does *not* ship in v1: rule-pack publication, registry resolution, semver version pinning, signature verification on packs. These are tracked in #29 and the pack-publication discussion that grows out of #79.

## Open questions for review

- **`then.action = "allow"` semantics.** This spec treats it as an explicit positive override (Part 2). Alternative: drop it entirely and rely on absence-of-rule. Keeping it lets the alembic-edits-fine pattern stay explicit, which reads better in code review. Recommended: keep.
- **Glob vs regex on `when.path`.** Spec says glob. Regex is more powerful but easier to write wrong. Rust crates exist for both (`globset`, `regex`). Recommended: glob; users who need regex use `when.command_pattern` for `Bash` or escalate to v2.
- **`severity = "inform"` with `then.action = "block"`.** Currently the spec says these usually agree but doesn't forbid the mismatch. Should the schema validate it? Recommended: no — the mismatch is sometimes meaningful (e.g. a block whose audit-log routing should be quiet).
