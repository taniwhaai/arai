# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- CLI with commands: init, status, guardrails, scan, add, upgrade
- Instruction file discovery (CLAUDE.md, .cursorrules, copilot-instructions.md, .windsurfrules, Claude Code memory)
- Rule extraction from markdown with 6 layers of pattern matching
- Intent classification with 3 tiers: verb taxonomy, ONNX sentence transformer, LLM shell-out
- Timing-aware rule routing (PreToolUse for domain rules, UserPromptSubmit for summaries)
- AST code graph via tree-sitter (Python, JS, TS, Rust, Go, Ruby, Java)
- Content sniffing for Write/Edit tool calls
- Session-aware prerequisite tracking (e.g., "cargo test" before "git push")
- Self-upgrade between lean/full binaries
- Hook injection into .claude/settings.json (PreToolUse, PostToolUse, UserPromptSubmit)
- Install script for curl-based installation
- npm wrapper package
- GitHub Actions CI for cross-platform releases
- Configurable LLM command for enrichment (ARAI_LLM_CMD)
