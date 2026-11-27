# Canonical rules-file spec (v1)

**Status:** v1 projection format shipped. `arai canonicalize` writes this
format from existing instruction files; `arai sync` renders managed prose
into per-tool files that already exist in the project. Native enforcement
matches scanned Markdown policy, not the stored v1 selectors or actions.
**Scope:** resolves [#75](https://github.com/taniwhaai/arai/issues/75) (format spike), [#76](https://github.com/taniwhaai/arai/issues/76) (schema design), and [#79](https://github.com/taniwhaai/arai/issues/79) (rule-pack accommodation).
**Still out of scope:** published rule-packs as a distribution channel.

`arai migrate` is **not** this bootstrap. That command moves the on-disk
layout from `~/.arai` to `~/.taniwha/arai`. Bootstrapping `arai.toml` from
instruction files is `arai canonicalize` (the work originally tracked as #78).

---

## Why a canonical file

Today, Arai reads rules from whatever instruction file each tool happens to use — `CLAUDE.md`, `.cursorrules`, `.windsurfrules`, `.github/copilot-instructions.md`. Each format has its own conventions, and the rules drift: a `should not` added to `CLAUDE.md` to fix a Claude misbehaviour never makes it to Cursor, because the maintainer forgot the second file existed.

A canonical `arai.toml` is a version-controlled source for generated prose. `arai canonicalize` extracts whatever the parser already recognises; `arai sync` writes managed blocks in existing per-tool files. Extraction is lossy. Review the generated prose, scan it, then test actual calls; editing a v1 selector does not directly activate native selector matching.

This document describes projection data consumed by those commands. The live matcher is unchanged: instruction files remain valid policy even if you never canonicalize. Authored canonical IDs are headings in generated prose, not stable authored identity in native audit records. No v2 selector engine is activated.

---

## Part 1 — Format spike (resolves #75)

The brief recommended TOML; the spike confirms it. Below are the same four rules authored in both formats so a reader can judge.

### Projection-data samples: alembic + git push + secrets + test-before-commit

The TOML/YAML examples below illustrate stored data. Regexes, paths,
session selectors and actions in them are not executable native policy;
the implemented canonical loader uses TOML. YAML is a format comparison.

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

**Anchors and includes:** YAML's `&anchor` / `<<: *anchor` could factor shared data, but adds authoring complexity. v1 has no fragment composition or canonical pack expansion in `arai sync`; reserved metadata does not provide that behavior.

**Decision:** `arai.toml`. The schema below assumes TOML throughout.

---

## Part 2 — Schema (resolves #76)

The v1 schema is what `arai canonicalize` emits and `arai sync` consumes. Anything not listed is deliberately out of scope for v1 (including published rule-pack distribution).

### Top-level layout

```toml
[meta]
schema_version = 1               # required; bump on incompatible change
project = "arai"                 # optional; pure documentation

# Reserved metadata; v1 does not load packs from these references.
extends = []

# Rules are an array of tables. `arai sync` groups prose by severity,
# preserving input order within a group.
[[rule]]
id = "..."
...
```

### Per-rule fields

| Field | Type | Required | Notes |
|---|---|---|---|
| `id` | string | yes | Author-facing identifier rendered as a heading. No stable authored audit-ID guarantee. |
| `description` | string | yes | Free-form prose rendered as a blockquote. |
| `when.tool` | `[string]` | yes | Tool labels rendered in a trigger summary, not directly matched selectors. |
| `when.path` | `[string]` | no | Stored path patterns rendered as trigger text; no native glob execution. |
| `when.command_pattern` | string | no | Stored command pattern rendered as text; no native regex execution. |
| `when.content_pattern` | string | no | Stored content pattern rendered as text; no native regex execution. |
| `when.session_lacks` | `[string]` | no | Stored metadata; not rendered or executed by v1 sync. Native prose prerequisites are a separate matching feature. |
| `then.action` | string | yes | Stored action label; sync does not execute it or implement allow precedence. |
| `then.message` | string | yes | Rendered prose. Subsequent scanning/matching determines its native effect. |
| `severity` | string | yes | Sync groups entries labelled `block`, `warn`, `inform`; this is not a direct native severity override. |
| `expires` | string | no | Stored date metadata; sync does not render or execute it. Native prose expiry annotations remain separate. |

### Exceptions and overrides

The following is projection data expressing author intent for different
tools. It does not implement an Edit exception or direct path selection:

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

v1 has no most-specific selector resolution or `allow` override precedence.
Native matching uses the projected, scanned prose and its effective intent,
source scope and severity. Test that resulting policy rather than assuming
the two records create an executable exception.

### Worked example: alembic end-to-end

This complete `arai.toml` illustrates projection data for Alembic. It is
not equivalent native enforcement of the fresh Alembic fixture, and the
`allow` entry does not establish an executable exception:

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

`arai sync` renders a managed Markdown block into existing instruction
files. It preserves text outside the managed markers. Run `arai scan`
after sync, inspect the extracted rules and use `arai test` plus native
handler replay to establish their behavior. Canonical IDs appear as prose
headings; native audit identity remains the existing scanned-rule identity.

---

## Part 3 — Rule-pack accommodation (resolves #79)

Rule packs (FastAPI + alembic + pytest; Next.js + Prisma; etc.) are explicitly out of scope for v1. But the schema must not need a breaking change when packs ship. Three points to verify:

1. **`extends` is reserved under `[meta]`.** v1 stores the string list but does not resolve or expand canonical pack references.

2. **ID namespaces are authoring conventions.** They do not establish inherited/local override precedence in v1.

3. **Pack trust/distribution remains future work.** Existing instruction-file URL extension behavior does not make canonical pack metadata executable.

The metadata reserves room for pack references without claiming a shipped pack loader.

---

## What ships in v1

1. The `arai.toml` file format (TOML, per Part 1).
2. The schema defined in Part 2: per-rule `when` / `then` / `severity` / `expires`.
3. Reserved `[meta].extends` metadata (Part 3).
4. TOML deserialization of the stored fields; unknown fields are ignored. This is not an executable selector validator.

What does *not* ship in v1: rule-pack publication, registry resolution, semver version pinning, signature verification on packs. These are tracked in #29 and the pack-publication discussion that grows out of #79.

## Enforcement limits

- Stored selectors/actions are projection data; v1 does not directly execute them.
- `allow` does not override another native rule. A matched effective block denies when denial is enabled.
- Stored severity and expiry do not substitute for native scanned intent, overrides or annotations.
- Authored canonical IDs do not provide stable authored identity in native audits or scenario assertions.
