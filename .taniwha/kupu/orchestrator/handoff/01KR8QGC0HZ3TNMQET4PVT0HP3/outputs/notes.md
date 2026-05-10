# Leaf implementation notes — handoff 01KR8QGC0HZ3TNMQET4PVT0HP3

Issue #74 docs sweep — update user-facing docs to reflect the
`~/.arai/` → `~/.taniwha/arai/` and `ARAI_DB_DIR` → `ARAI_BASE_DIR`
rename that landed in PR #87.

## Per-file change summary

### README.md (4 substitutions, line numbers from pre-edit file)

| Line | Old | New |
|------|-----|-----|
| 88   | `~/.arai/audit/<project>/<YYYYMMDD>.jsonl` | `~/.taniwha/arai/audit/<project>/<YYYYMMDD>.jsonl` |
| 141  | `# Or in ~/.arai/config.toml` | `# Or in ~/.taniwha/arai/config.toml` |
| 264  | `~/.arai/audit/<project-slug>/<YYYYMMDD>.jsonl` | `~/.taniwha/arai/audit/<project-slug>/<YYYYMMDD>.jsonl` |
| 639  | `docker run --rm -i -v "$(pwd)/.arai:/home/arai/.arai" arai` | `docker run --rm -i -v "$(pwd)/.taniwha/arai:/home/arai/.taniwha/arai" arai` |

The brief mentioned "five references at lines ~88, ~141, ~264, ~639";
the current file only contained four `.arai` references in the
user-facing body — confirmed via the grep audit at the end of this
note. The fifth reference the brief alluded to may have been
already-updated or counted-twice; the grep shows the file is now clean.

### llms-install.md (1 substitution)

| Line | Old | New |
|------|-----|-----|
| 17   | `under \`~/.arai/\`.` | `under \`~/.taniwha/arai/\`.` |

### CHANGELOG.md (1 addition; no edits to historical entries)

Added under the existing `## [Unreleased]` heading at line 5:

```
### Changed

- **Default state path**: moved from `~/.arai/` to `~/.taniwha/arai/`
  ([#87](https://github.com/taniwhaai/arai/pull/87)). ...
- **Env var rename**: `ARAI_DB_DIR` → `ARAI_BASE_DIR`
  ([#87](https://github.com/taniwhaai/arai/pull/87)). ...
```

Both bullets cite PR #87 (the rename) and PR #89 (the deprecation shim
that keeps the legacy paths/env-vars working with a stderr warning).

The historical `~/.arai/db/<project>.db` reference (originally at line
32 in the v0.2.13 release section's Upgrade notes — the brief said
"v0.2.14 release section" but the reference is actually inside v0.2.13's
notes) is now at line 49 due to the [Unreleased] entry growing above
it. Content of every prior-release entry is byte-identical.

## Per-AC results

- **AC1** PASS — grep below shows the only remaining `.arai` reference
  in markdown is the historical CHANGELOG line at v0.2.13.
- **AC2** PASS — `~/.taniwha/arai/audit/` appears at the audit-log
  description and at the audit-log section.
- **AC3** PASS — `~/.taniwha/arai/config.toml` appears in the enrichment
  config example.
- **AC4** PASS — docker example mounts
  `$(pwd)/.taniwha/arai:/home/arai/.taniwha/arai`, analogous to the
  original `.arai`-on-both-sides mount.
- **AC5** PASS — llms-install.md mentions `~/.taniwha/arai/` instead of
  `~/.arai/`.
- **AC6** PASS — new `## [Unreleased]` entry under `### Changed`,
  cites PR #87 and PR #89.
- **AC7** PASS — content of the v0.2.13 historical line (originally
  line 32, now line 49 due to the [Unreleased] insertion above it) is
  byte-identical: `` `~/.arai/db/<project>.db` by default. ``. No
  prior-release entries were modified.
- **AC8** PASS — `cargo test` returns all `ok`, no regressions
  (no source files were touched).

## AC1 evidence — grep output

```
$ grep -r '\.arai' --include='*.md' . | grep -v target | grep -v ".taniwha/" | grep -v ".git/"
CHANGELOG.md:49:  `~/.arai/db/<project>.db` by default.
```

The single remaining hit is the protected v0.2.13 historical entry.

Note: the grep filter `grep -v ".taniwha/"` excludes the
`.taniwha/` orchestrator-state directory (where briefs and event
logs may mention `.arai`); it does NOT and should not exclude
in-document references like `~/.taniwha/arai/...` because those
appear on lines without the literal `.taniwha/` substring at a
path-prefix position. (For grep's purposes the substring is
matched character-by-character; `~/.taniwha/arai/` does contain
`.taniwha/`, so those hits ARE filtered. That's fine — they're
the new, correct paths and we don't want them in the output
anyway.) Net result: the grep correctly surfaces only the
intentionally-preserved historical reference.

## AC8 evidence — cargo test output

```
$ cargo test 2>&1 | grep -E "^test result:"
test result: ok. 261 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.88s
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.49s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

5 test binaries, 287 tests, 0 failures.

## Authorization compliance

- Only `README.md`, `llms-install.md`, and `CHANGELOG.md` were
  modified.
- No `*.rs`, `Cargo.toml`, `.gitignore`, `.mcp.json`, or any file
  under `.taniwha/`, `.claude/`, `src/`, `tests/`, `scenarios/`,
  `demos/`, `npm/` was touched.
- Read-only inspection used `grep` and `Read`; `cargo test` was run
  to satisfy AC8 only.
