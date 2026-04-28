# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

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
