# Changelog

All notable changes to this project will be documented in this file.

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
