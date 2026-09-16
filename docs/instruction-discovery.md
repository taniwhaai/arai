# Instruction discovery and ownership

`arai init` and `arai scan` discover root and nested `CLAUDE.md`,
`CLAUDE.local.md`, and `AGENTS.md` variants, `.claude/rules/**/*.md`,
`.cursor/rules/**/*.md` and `*.mdc`, and the existing global, memory and
explicit extra sources. Global Claude rules under `~/.claude/rules/` are
included. Case aliases on Windows and duplicate extra references are deduplicated.
Generated dependency/build directories, VCS internals and nested Git repositories
are excluded. Symlinked files are read, but symlinked directories are not traversed.
Instruction files may be intentionally gitignored, so Git ignore patterns do not
exclude them. An unreadable source or invalid scope aborts the scan.

## Activation

Nested instruction files apply within their directory. `.claude/rules` `paths`
and Cursor `globs` restrict file actions relative to the associated project or
nested rule directory's project. `alwaysApply: true` makes a Cursor rule apply
throughout that directory regardless of `globs`. Manual or agent-selected Cursor
rules have no deterministic activation signal in Arai: they are stored, reported
as inactive, and do not silently become global rules. Legacy plain `.md` Cursor
rules without activation metadata retain their previous unconditional behavior.

The supported frontmatter subset accepts top-level `paths`, `globs` and
`alwaysApply`, quoted glob strings, scalar/inline/multiline lists and brace globs.
Invalid scope syntax fails explicitly. YAML merge keys and advanced YAML value
forms are unsupported. Upstream `arai:extends` rules inherit the local source's
scope, retain their tier and source label, and cannot replace its activation header.

File scope uses the host's file path and working-directory context. NotebookEdit
uses `notebook_path`; MultiEdit uses its file path. Codex patches are evaluated
per affected path. Shell calls can match a directory scope using their reported
working directory; Arai does not infer arbitrary shell target files or interpret
`cd` to activate filename globs. Filename-glob rules therefore need a file action.

This is Arai's shared enforcement scope, not a complete reimplementation of each
host's instruction precedence. Codex `AGENTS.override.md`, configurable fallback
filenames and host-specific override precedence are not implemented. Such
precedence needs host provenance so it does not replace Claude or Grok policy
in the shared store. Native Codex hooks are described in [codex.md](codex.md).

FileChanged refresh uses the same instruction-path recognition as discovery,
including `.mdc` and nested rules. It runs asynchronously where the host emits
that event. On other hosts, run `arai scan` after instruction changes.

## Snapshot and embedding boundaries

A successful local scan replaces its discovery-owned snapshot in one SQLite
transaction. Deleted sources disappear; a failed scan preserves the previous
snapshot. Explicit severity overrides survive unrelated edits to the same rule.
Content changes retain upstream provenance in the re-extracted triples.

Sources installed through public `Store::upsert_file` are externally managed,
including when the content checksum is unchanged. Discovery does not replace,
prune, or reclassify them. Automatic sentence-transformer enrichment is limited
to discovery-owned sources; adding a manual rule classifies/enriches only that
source. The existing explicit library-wide classification/enrichment APIs remain
available to embedders. Explicit LLM/API/file enrichment commands still operate
on the selected store and should be controlled by its policy owner.

Ownership protects composition from incidental CLI refreshes; it is not an
authorization boundary against an actor who can modify the database. Kete still
owns signature acceptance, atomic policy activation and distribution. See
[the stack integration contract](stack-integration.md).

## Upgrading an existing store

Old stores did not record source ownership. Migration retains their rules as
legacy sources with the prior global activation behavior, and scans print a
notice. Arai cannot safely infer whether a path was populated by the CLI or an
embedder such as Kete.

For a locally managed project, review the discovered sources and run:

```sh
arai scan --adopt-legacy-sources
```

This adopts only legacy rows whose paths appear in the successful current scan,
applies their scope, and enables subsequent refresh/removal. It never adopts
explicitly external sources. Absent legacy rows remain retained: restore the
matching local source, adopt it, then remove it and scan if that rule source is
indeed obsolete. Do not use adoption to replace verified embedding policy.

Host references: [Claude memory and rules](https://code.claude.com/docs/en/memory),
[Cursor rules](https://cursor.com/docs/rules), and
[Codex AGENTS.md](https://learn.chatgpt.com/docs/agent-configuration/agents-md).
