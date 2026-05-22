# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

## [0.2.22] - 2026-05-22

### Miscellaneous

- Update Cargo.lock dependencies


## [0.2.21] - 2026-05-21

### Miscellaneous

- Clean out four open issues — migrate cmd, compliance/severity integration tests, rules-file spec ([#114](https://github.com/taniwhaai/arai/pull/114))


## [0.2.20] - 2026-05-15

### Added

- *(audit)* Arai audit --purge for retention / deletion controls ([#108](https://github.com/taniwhaai/arai/pull/108))
- *(hooks)* Tier 1+2 event coverage (FileChanged, InstructionsLoaded, CwdChanged, PostToolBatch, PermissionDenied) + Tier-3 Kete-HTTP design doc ([#111](https://github.com/taniwhaai/arai/pull/111))


### Added

- *(hooks)* **Tier-1 hook event coverage — rule set stays live,
  monorepo navigation works, parallel-tool compliance reconciled.**
  Four new hook events wired in alongside `PreToolUse`/`PostToolUse`/
  `UserPromptSubmit`. Part of [#110](https://github.com/taniwhaai/arai/issues/110).
  - **`FileChanged` + `InstructionsLoaded`** — when `CLAUDE.md`,
    `.cursorrules`, `.windsurfrules`, `copilot-instructions.md`, any
    `.claude/rules/*.md` or `.cursor/rules/*.md` file, or a per-project
    Claude Code memory file changes on disk or is loaded into context,
    Arai spawns an `arai scan` in the background. The next tool-call
    hook sees the refreshed guardrails — no manual rescan, no stale
    rules enforcing a previous wording.
  - **`CwdChanged`** — monorepo fix. When Claude `cd`s into a
    subpackage, Arai spawns a scan rooted at the *new* working
    directory so the destination's per-project DB is populated. The
    next tool-call hook in that dir matches against the correct rule
    set. `arai audit --event=CwdChanged` shows the navigation history.
  - **`PostToolBatch`** — parallel-tool compliance correlation. The
    existing per-call PostToolUse correlator under-counts verdicts when
    Claude does parallel tool calls (multi-Edit, parallel Bash). The
    new handler iterates the batch's `tool_calls[]` / `tool_results[]`
    pairs and feeds each through `compliance::record_post_compliance`,
    so every tool in the batch gets its own
    Obeyed/Ignored/Unclear verdict. Closes the per-rule-ratio gap
    advertised on the marketing site.
  - All four are observability-only — no `permissionDecision` surface,
    no agentic-loop blocking. Gating still happens at PreToolUse.
- *(hooks)* **`PermissionDenied` — unified audit + Warn-level retry
  override.** When Claude Code's auto-mode classifier denies a tool
  call, Arai now (a) writes a `PermissionDenied` audit entry capturing
  both classifiers' opinions so the unified record shows the
  disagreement, and (b) returns `{retry: true}` to override the
  auto-deny *iff* Arai's own matching for that tool call produces a
  Warn-level severity. Block-level matches (Arai agrees with the deny)
  and no-match cases (Arai has no opinion) both leave the auto-deny
  in place. Honours `ARAI_DENY_MODE=off` — advise-only mode never
  overrides another classifier. Part of [#110](https://github.com/taniwhaai/arai/issues/110).
- *(audit)* **`arai audit --purge` for retention / deletion controls.**
  Drops day-bucket files (and their `.head.` sidecars) under
  `~/.taniwha/arai/audit/<project>/`. Two scoping forms:
  `--older=N` (age-based retention; `--older=0` keeps only today) and
  `--project=<slug>` (full project wipe — offboarding / decommission).
  Today's day-bucket is always preserved so the live hook chain isn't
  disturbed, and whole files are deleted (never individual lines) so the
  hash chain on retained days stays valid. `--dry-run` + `--json` for a
  pre-purge review. Refuses to run without an explicit scope so a bare
  `arai audit --purge` can't accidentally nuke history. Closes the
  deletion-on-demand gap flagged in #95 (item 5).
- *(docs)* **HTTP hooks / Kete integration design doc**
  ([`docs/design-http-hooks-kete-integration.md`](docs/design-http-hooks-kete-integration.md)).
  Tier-3 deliverable from [#110](https://github.com/taniwhaai/arai/issues/110): the contract for how a developer-side Arai
  install talks to an org-hosted Kete policy server via Claude Code's
  new `type: "http"` hook handler. Captures the request/response
  contract, auth, failure modes, and migration path. Design only —
  no code changes; implementation gated on sign-off.

## [0.2.19] - 2026-05-14

### Documentation

- Surface hash-chain / MCP auth / cache sig + SOC 2 TSC mapping ([#106](https://github.com/taniwhaai/arai/pull/106))


### Documentation

- *(compliance)* Surface the hash chain + `arai audit --verify`, MCP auth
  via `ARAI_MCP_AUTH_TOKEN`, `arai:extends` cache-at-rest signature,
  Windows ACL pin, and telemetry queue cap in the README, marketing site,
  and the procurement deliverable. Add an explicit SOC 2 Trust Service
  Criteria mapping (CC6.1 / CC6.6 / CC7.2 / CC7.3 / CC8.1 / CC9.2) to
  `docs/arai-compliance-features.docx` + PDF so a reviewer doesn't have
  to do the mapping themselves.

## [0.2.18] - 2026-05-13

### Security

- *(audit)* **Tamper-evident hash chain.** Every audit-log line carries
  `prev_hash` + `hash` (SHA-256 over canonical bytes); a per-day
  `.head.YYYYMMDD` sidecar anchors the chain tip. New `arai audit --verify`
  walks every day-bucket and exits non-zero on any tamper / reorder /
  deletion. Backs the previously-overclaimed "tamper-evident" with an
  actual mechanism. Day-buckets are retained indefinitely — bucketing is
  the segmentation, no auto-prune.
  ([#104](https://github.com/taniwhaai/arai/pull/104))
- *(audit)* **Windows audit ACLs pinned.** First audit write on Windows
  shells once to `icacls /inheritance:r /grant:r USER:(OI)(CI)F` and drops
  a `.arai_acl_set` marker so subsequent writes skip the call. Matches the
  Unix 0700/0600 lock-down.
- *(mcp)* **Shared-secret authentication** via `ARAI_MCP_AUTH_TOKEN`. When
  set, `initialize.params.auth_token` must match (constant-time compare);
  subsequent calls on the same stdio connection inherit auth. Open behaviour
  preserved when the env var is unset. Notification handling hoisted above
  the auth gate to stay JSON-RPC 2.0 compliant.
- *(extends)* **Cache-at-rest signature** for `arai:extends`. Cached
  upstream-policy files now carry a `<hash>.md.sha256` sidecar. Mismatched
  or missing sidecars are treated as a cache miss in both the fresh-read
  and stale-while-error paths. Closes the cache-tampering surface beneath
  the trust list.
- *(telemetry)* **2 MiB hard cap** on `telemetry_queue.jsonl`. One
  `metadata()` syscall in `track()` drops events above the cap so installs
  that only ever invoke hooks (and never the non-hook CLI commands that
  flush) can't grow the queue unbounded.


## [0.2.17] - 2026-05-11

### Performance

- *(release)* Panic=abort + codegen-units=1 — 4.3% smaller binary ([#101](https://github.com/taniwhaai/arai/pull/101))

### Style+ci

- Cargo fmt sweep + CI gate ([#102](https://github.com/taniwhaai/arai/pull/102))


## [0.2.16] - 2026-05-11

### Added

- *(enrich)* Per-rule (noenrich) opt-out + pre-send destination notice ([#94](https://github.com/taniwhaai/arai/pull/94)) ([#96](https://github.com/taniwhaai/arai/pull/96))

### Fixed

- *(matching)* Close substring-leak + self-block bugs ([#86](https://github.com/taniwhaai/arai/pull/86)) ([#91](https://github.com/taniwhaai/arai/pull/91))


## [0.2.15] - 2026-05-10

### Added

- *(demos)* Add example showcasing command blocking with Arai guardrails
- *(demos)* Update blocking demo to enhance user interaction flow
- *(paths)* Move default state dir to .taniwha/arai, rename ARAI_DB_DIR ([#71](https://github.com/taniwhaai/arai/pull/71))
- *(paths)* Deprecation shim for ARAI_DB_DIR + ~/.arai ([#73](https://github.com/taniwhaai/arai/pull/73))

### Documentation

- *(paths)* Sweep README/llms-install/CHANGELOG for v0.2.15 rename ([#74](https://github.com/taniwhaai/arai/pull/74))

### Fixed

- *(demos)* Trim leading typing frames so README poster shows prompt

### Miscellaneous

- *(deps)* Bump openssl in the cargo group across 1 directory ([#59](https://github.com/taniwhaai/arai/pull/59))
- Track Taniwha + Claude tooling state in repo
- Add Taniwha agents + skills, ignore WSL Zone.Identifier noise

### Performance

- *(parser)* Hoist regex compilations into OnceLock statics ([#92](https://github.com/taniwhaai/arai/pull/92)) ([#93](https://github.com/taniwhaai/arai/pull/93))


### Changed

- **Default state path**: moved from `~/.arai/` to `~/.taniwha/arai/`
  ([#87](https://github.com/taniwhaai/arai/pull/87)). Audit logs, config,
  and the local SQLite store now live under
  `~/.taniwha/arai/{audit,config.toml,db}` by default. The old `~/.arai/`
  path is still honoured by the deprecation shim
  ([#89](https://github.com/taniwhaai/arai/pull/89)) — when detected, Arai
  reads from the legacy location and emits a one-time stderr warning so
  existing installs keep working until users migrate.
- **Env var rename**: `ARAI_DB_DIR` → `ARAI_BASE_DIR`
  ([#87](https://github.com/taniwhaai/arai/pull/87)). The new name reflects
  that the variable now overrides the entire state directory (audit + config
  + db), not just the database. The old `ARAI_DB_DIR` is still honoured by
  the deprecation shim ([#89](https://github.com/taniwhaai/arai/pull/89))
  with a stderr warning prompting users to switch.

## [0.2.14] - 2026-05-02

### Documentation

- Surface compliance + audit pitch on site and README ([#54](https://github.com/taniwhaai/arai/pull/54))
- Ship compliance inventory as PDF (renders inline in browsers) ([#57](https://github.com/taniwhaai/arai/pull/57))


## [0.2.13] - 2026-05-02

### Testing

- Proptest property tests for input validators + WAL upgrade note


### Upgrade notes

- **WAL mode is now enabled on the local store** (set in 0.2.12 by the
  versioned migration framework).  SQLite in WAL mode writes to
  `<db>-wal` and `<db>-shm` sidecar files alongside the main `.db` file.
  If you have any backup or sync tooling that copies only the `.db` file,
  recently-written rules and audit data may be missed.  Either include the
  sidecar files in your backup, or run `arai status` once before the
  copy (it triggers a checkpoint that flushes the WAL into the main
  file).  Most users will never notice — the local store lives at
  `~/.arai/db/<project>.db` by default.

### Hooks

- Validate `hook_event_name` against an allow-list before propagating to
  the fail-closed gate; spoofed values like `"PreToolUseFOO"` no longer
  defeat the C10 deny-on-error contract
- Surface `Config::load()` failures from the `ARAI_DISABLED` short-circuit
  to stderr instead of swallowing them silently

### Parser

- `<!-- arai:skip -->` markers now track heading depth correctly:
  sub-headings under a marked section stay inside the skip; only same-
  or-shallower headings clear it

### Store

- Defensive identifier validation in `add_column_if_missing` (closes a
  future SQLi window if a refactor wires it to a runtime string)

### Telemetry

- Removed dead `track_rule_followed` function that still queued raw
  subject text — the rule_hash anonymisation from 0.2.12 was only
  applied to the live `track_rule_fired` path

### Tests

- Property and integration tests for `known_hook_event`,
  `valid_session_id`, parser nested-heading scope, ARAI_DISABLED
  short-circuit, fail-closed PreToolUse, and the `arai_check_action`
  MCP probe's no-audit-write contract

## [0.2.12] - 2026-05-02

### Documentation

- Replace <5ms latency claims with measured numbers

### Bench

- Subprocess timing harness for the hook hot path

### Discovery

- Walk .cursor/rules as a directory of rule files

### Guardrails

- Replace O(N×L) sniff with Aho-Corasick automaton

### Hooks

- Validate session_id + ARAI_DISABLED kill switch with bypass audit
- Deny reason includes line number; PreToolUse errors fail-closed

### Mcp

- Add arai_check_action probe tool

### Parser

- Respect <!-- arai:skip --> markers, scoped to next heading

### Store

- Gate schema init behind versioned migration framework + WAL PRAGMAs
- LEFT JOIN rule_intent into load_guardrails / rules_for_file

### Store+cli

- Arai disable/enable + disabled_rules table (migration v2)

### Telemetry

- Hash rule subject/predicate before queueing for upload


## [0.2.11] - 2026-04-30

### Added

- *(parser)* Broaden imperative coverage with 12 patterns + corpus regression test ([#51](https://github.com/taniwhaai/arai/pull/51))

### Miscellaneous

- Add glama.json for MCP server claim ([#49](https://github.com/taniwhaai/arai/pull/49))


### Added

- *(parser)* Twelve new imperative-extraction patterns, measured against
  a 93-file public CLAUDE.md corpus and shipped together so users who
  write rules in any of these styles get them honoured rather than
  silently dropped:
  - **Layer 1**: `^should\s+not` / `^shouldn't` → `must_not` (Block);
    `^should` → `prefers` (Inform — softer than `must`/`always`);
    `^cannot` → `must_not`; `^refuse to` → `forbids`;
    `^enforce` → `enforces`; `^make sure` / `^be sure` → `enforces`;
    `^consider` / `^recommend(ed)?` → `prefers`.
  - **Layer 1b**: bare `^no\s+(.+)` → `must_not` (Block), gated against
    bold-label feature-absence form (`**No build process** - this is a
    zero-build extension.` does NOT extract).
  - **Layer 5**: `^use\s+` now also fires when the section header
    matches `Conventions / Rules / Style / Guidelines / Best Practices /
    Coding Standards / Policies` — covers the very common style-guide
    pattern where the section framing makes the imperative explicit
    even without a known tool name in the line.
  - **Layer 6 verb additions**: `create`, `implement`, `document`,
    `define`, `store` — measured ~80 real rules across the public
    corpus.
  - **Layer 7 (new)**: conditional imperatives —
    `^(Before|After|When|Whenever|If|For)\s+<condition>(,|:|→|—)\s+<verb>\s+<rest>`
    where `<verb>` is in the recognised imperative whitelist.  Catches
    the trigger-paired-with-imperative form ("When working in parallel,
    run tests in isolation") that previously slipped past every layer.
- *(audit)* Layer 7 label in the derivation trace.
- *(tests)* `tests/parser_coverage/corpus.md` + `tests/parser_coverage.rs`
  — synthetic CLAUDE.md exercising every pattern (positive + negative
  cases) plus an integration test driven through the live `arai lint
  --json` binary.  17 spot-check assertions including the
  `**No build process**` regression guard.

## [0.2.10] - 2026-04-30

### Documentation

- Clarify enforcement scope across non-Claude assistants ([#44](https://github.com/taniwhaai/arai/pull/44))
- Prep for Cline MCP marketplace submission ([#46](https://github.com/taniwhaai/arai/pull/46))

### Build

- Add Dockerfile + .dockerignore ([#48](https://github.com/taniwhaai/arai/pull/48))


## [0.2.9] - 2026-04-29

### Added

- Per-session repeat-injection suppression + token-economics estimates ([#41](https://github.com/taniwhaai/arai/pull/41))

### Documentation

- Post-merge polish for token-economics work ([#43](https://github.com/taniwhaai/arai/pull/43))


### Added

- *(hooks)* Per-session repeat-injection suppression.  When a rule
  fires a second time in the same session, the hook emits a compact
  "still: subject predicate object" line instead of re-injecting
  the full source / layer / severity payload — the model already
  has that context from the first firing.  Saves roughly 50 tokens
  per re-fire on long sessions and reduces attention dilution from
  reading the same rule N times.
- *(audit)* New `seen_before` flag per rule on every firing entry.
  `arai stats` rolls this up into a suppression count.  Additive
  field; older audit lines are treated as `seen_before: false`.
- *(stats)* Token-economics section in `arai stats` — calibrated
  estimates of saved tokens from three streams: repeat-injection
  suppressions (50 each), denied-and-honored mistakes (2000 each),
  advised-and-honored events (500 each).  Labelled as estimates,
  not measurements; constants documented in `src/stats.rs`.  JSON
  output exposes a `token_economics` object with the per-stream
  counts.

## [0.2.8] - 2026-04-29

This release closes attack surfaces flagged in an internal audit. No known
exploits in the wild; report any future findings via GitHub Security
Advisories at <https://github.com/taniwhaai/arai/security/advisories/new>.

### Security

- **`arai:extends` SSRF.** The upstream-policy fetcher now resolves
  hostnames and refuses loopback, RFC1918, link-local (including cloud
  metadata at `169.254.169.254`), CGNAT, multicast, and IPv6 ULA /
  link-local addresses. Redirects are disabled (`max_redirects(0)`) so a
  302 cannot bypass the trust list, and `Accept-Encoding: identity` is
  forced so a small gzip body cannot decompress past the 512 KB cap.
  Cache writes refuse to follow symlinks on both the fresh-write and
  stale-while-error paths.
- **MCP server unbounded input.** `arai_add_guard` now caps rule bodies
  at 1 KiB, reasons at 4 KiB, and refuses new adds once a project has
  accumulated 1000 MCP-source rules. Prevents a malfunctioning or
  malicious agent from filling the local SQLite store and triggering
  expensive re-classification on each call.
- **Hook stdin OOM.** The hook handler caps stdin at 1 MiB so a runaway
  pipe cannot exhaust memory before JSON parsing.
- **Audit log file mode.** Audit JSONL files are now created with mode
  `0600` and the audit directory with `0700` on Unix. Previously the
  umask-derived defaults (typically `0644` / `0755`) left session ids,
  prompt previews, and rule subjects readable by other users on
  multi-user systems.
- **AST recursion bound.** `code_scanner` now caps tree-sitter walk depth
  at 500, preventing a stack overflow on adversarially nested source
  files during a `--code` scan.
- **Installer signature verification.** `install.sh` and
  `npm/install.js` now download `checksums.txt` from the matching
  release and verify the binary's SHA-256 before writing it to disk.
  `ARAI_SKIP_CHECKSUM=1` is supported as an escape hatch for local dev
  against unsigned builds. Closes the supply-chain gap where a
  compromised GitHub release or DNS hijack could ship arbitrary code to
  every `curl | sh` or `npm install` user.

### Dependencies

- Bumped `rustls-webpki` 0.103.12 → 0.103.13
  ([RUSTSEC-2026-0104](https://rustsec.org/advisories/RUSTSEC-2026-0104):
  denial of service via panic on malformed CRL `BIT STRING`).

### Fixed

- *(security)* Harden hot path, MCP server, and arai:extends fetcher
- *(release)* Verify SHA-256 in install.sh and npm before installing


## [0.2.7] - 2026-04-28

### Fixed

- *(stats)* Dedupe compliance verdicts per Pre firing (closes #37) ([#38](https://github.com/taniwhaai/arai/pull/38))


### Fixed

- *(stats)* Per-Pre dedupe in compliance roll-up.  A single Pre
  firing now produces at most one rolled-up verdict regardless of
  how many Posts correlated against it inside the 5-minute window.
  First-definitive-wins: the first `obeyed` or `ignored` verdict
  for a Pre is the one counted; later Posts against the same Pre
  are evidence about later state, not about the original warning.
  Audit log behaviour is unchanged — `arai audit` still surfaces
  every correlated firing for investigation.  Closes
  [#37](https://github.com/taniwhaai/arai/issues/37).

## [0.2.6] - 2026-04-27

### Added

- Per-rule compliance, severity override, recent_decisions, audit --rule, diff ([#35](https://github.com/taniwhaai/arai/pull/35))


### Added

- *(stats)* Per-rule compliance roll-up — `fires / obeyed / ignored /
  unclear / ratio` joined across Pre firings and Compliance verdicts
  via `triple_id`.  `arai stats` shows it inline; `arai stats
  --by-rule` shows only that section.  ⚠ flag highlights low-ratio
  rules with enough volume to mean it.
- *(severity)* `arai severity` subcommand — pin a rule's severity
  for incremental deny-mode rollout.  Stored in a new
  `rule_intent.severity_override` column that survives `arai scan`
  re-classification.  `arai severity --reset` drops the override.
- *(mcp)* `arai_recent_decisions` tool — surfaces recent deny /
  inject / review decisions back to the agent, so the model can
  self-check after a refusal instead of repeating the same action.
  Filters by `session_id`, `limit`, and `since`; skips `Compliance`
  events.
- *(audit)* `--rule` filter — case-insensitive substring match
  against rule subject/predicate/object across both top-level
  firings and Compliance `payload.rules[]`.  Pairs with `--outcome`.
- *(diff)* `arai diff <file>` — preview rule-set delta vs. the live
  store before saving an instruction-file edit.  Reports added,
  removed, and moved (same SPO, different line); JSON output for
  pre-commit hooks.


## [0.2.5] - 2026-04-27

### Miscellaneous

- Update Cargo.lock dependencies


## [0.2.4] - 2026-04-26

### Documentation

- Align v0.3 references with actual v0.2.3 release

## [0.2.3] - 2026-04-24

### Added

- Deny mode, compliance tracking, derivation trace, rule expiry, arai why

### Documentation

- Describe v0.2.3 features (deny mode, compliance, derivation trace) across README, CLAUDE.md, and site


## [0.2.2] - 2026-04-20

### Added

- Arai lint, arai record, rule-health check in arai status, alembic example

### Miscellaneous

- Add .gitattributes to enforce LF line endings


## [0.2.1] - 2026-04-20

### Added

- Add stats, test harness, and shared-policy extends

### Documentation

- Cover stats, test, trust, and extends in README/CLAUDE/site


## [0.2.0] - 2026-04-20

### Added

- *(audit)* Local JSONL log of rule firings + `arai audit` CLI
- *(mcp)* Stdio MCP server exposing `arai_add_guard` + `arai_list_guards`

### Documentation

- Cover `arai audit` + `arai mcp` in README, CLAUDE.md, and site

### Fixed

- *(audit)* Promote `era` to i64 in civil_to_epoch

### Testing

- *(audit)* Fix bad expected value in test_epoch_roundtrip

### Style

- Silence clippy lints on new audit + CLI code


## [0.1.11] - 2026-04-16

### Added

- *(npm)* Add npm package with binary installer and setup script


## [0.1.10] - 2026-04-16

### Added

- *(enrich)* Add API-based enrichment and support for OpenAI-compatible endpoints


## [0.1.9] - 2026-04-15

### Added

- *(enrich)* Add advanced error handling and file-based enrichment support


## [0.1.8] - 2026-04-14

### Miscellaneous

- *(README)* Remove deprecated npm installation instructions


## [0.1.7] - 2026-04-14

### Added

- *(guardrails)* Improve rule matching by adding relevance scoring and ranking


## [0.1.6] - 2026-04-14

### Miscellaneous

- *(dependencies)* Remove `walkdir`, update exclusions in Cargo.toml


## [0.1.5] - 2026-04-14

### Miscellaneous

- *(workflows)* Switch release step to use `gh release upload` for better token support


## [0.1.4] - 2026-04-14

### Refactored

- *(upgrade)* Replace manual JSON parsing with serde_json for reliability


## [0.1.3] - 2026-04-14

### Added

- *(site)* Add static landing page and install script
- *(site)* Integrate PostHog analytics script for usage tracking


## [0.1.2] - 2026-04-14

### Miscellaneous

- *(workflows)* Consolidate CI and release workflows into a single workflow, remove redundant files

### Refactored

- *(core)* Simplify string handling and improve code clarity


## [0.1.1] - 2026-04-14

### Added

- *(parser)* Improve tool detection with contextual validation

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
