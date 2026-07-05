//! Ārai as a library.
//!
//! Ārai enforces AI coding assistant instruction files (CLAUDE.md,
//! AGENTS.md, .cursorrules, …) via hooks.  This crate exposes the core
//! pipeline so wrappers, IDE integrations, and downstream tools can embed
//! enforcement as a dependency instead of shelling out to the `arai` CLI.
//!
//! # Core pipeline
//!
//! ```text
//! instruction file ──parser──▶ rules (triples) ──store──▶ SQLite + FTS5
//!                                                            │
//! hook JSON ──hooks::match_hook──▶ guardrails matching ◀─────┘
//!                    │
//!                    ├──▶ intent / severity  →  deny / warn / inform
//!                    └──▶ audit (hash-chained JSONL) + compliance verdicts
//! ```
//!
//! The single pure entry point used by both the live stdio handler and the
//! `arai test` scenario runner is [`hooks::match_hook`] — any change to
//! rule-matching logic goes there so the two paths stay identical.  Side
//! effects (audit write, telemetry, session write) live only on the CLI's
//! stdin-handling path.
//!
//! # Module map
//!
//! | Module | Role |
//! |--------|------|
//! | [`parser`] | Rule extraction from markdown (7 layers of pattern matching) |
//! | [`store`] | SQLite + FTS5 persistence (files, triples, code graph, rule intent) |
//! | [`guardrails`] | Term extraction, subject matching, tool-scope filtering |
//! | [`hooks`] | Hook protocol — PreToolUse/PostToolUse/UserPromptSubmit → decisions |
//! | [`intent`] | Intent classification — action, timing, tool scope, severity |
//! | [`audit`] | Local hash-chained JSONL firing log |
//! | [`compliance`] | Pre/Post correlation — Obeyed/Ignored/Unclear verdicts |
//! | [`config`] | Config file, project paths + slug, env vars |
//! | [`discovery`] | Instruction-file discovery |
//! | [`session`] | Per-session prerequisite + seen-rule tracking |
//! | [`prompt_collector`] | Read-only prompt-pattern observation (no enforcement) |
//! | [`canonicalize`] | Rule extraction from existing instruction files |
//!
//! Remaining modules (`init`, `mcp`, `stats`, `scenarios`, …) back specific
//! CLI subcommands; they are exported for the `arai` binary but hidden from
//! docs and not part of the supported library API.

pub mod audit;
pub mod canonicalize;
pub mod compliance;
pub mod config;
pub mod discovery;
pub mod guardrails;
pub mod hooks;
pub mod intent;
pub mod parser;
pub mod prompt_collector;
pub mod session;
pub mod store;

// CLI-support modules: exported so the `arai` binary (and integration
// tests) can reach them, but hidden from rustdoc — they back specific
// subcommands and are not a supported embedding surface.
#[doc(hidden)]
pub mod code_scanner;
#[doc(hidden)]
pub mod enrich;
#[doc(hidden)]
pub mod extends;
#[doc(hidden)]
pub mod init;
#[doc(hidden)]
pub mod mcp;
#[doc(hidden)]
pub mod migrate;
#[doc(hidden)]
pub mod scenarios;
#[doc(hidden)]
pub mod stats;
#[doc(hidden)]
pub mod style;
#[doc(hidden)]
pub mod sync;
#[doc(hidden)]
pub mod telemetry;
#[doc(hidden)]
pub mod upgrade;
