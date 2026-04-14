# CLAUDE.md — Arai

Arai is a Rust CLI that enforces AI coding assistant instruction files (CLAUDE.md, .cursorrules, etc.) via Claude Code hooks.

## Commands

```bash
cargo build                    # Build
cargo test                     # Run tests (52 tests)
cargo install --path .         # Install lean binary
cargo install --path . --features enrich  # Install with ONNX enrichment
cargo run -- init              # Test init flow
cargo run -- guardrails        # List guardrails
cargo run -- status            # Show enforcement status
cargo run -- scan --code       # Re-scan with AST code graph
cargo run -- scan --enrich-llm # Enrich rules via LLM
cargo run -- add "Never X"     # Add a manual rule
echo '{"tool_name":"Bash","tool_input":{"command":"git push"}}' | cargo run -- guardrails --match-stdin
```

## Architecture

```
src/
├── main.rs               # CLI entry (clap) — init, status, guardrails, scan, add, upgrade
├── config.rs              # Config, project paths, env vars, LLM command
├── discovery.rs           # Instruction file discovery (CLAUDE.md, .cursorrules, etc.)
├── parser.rs              # Rule extraction from markdown (6 layers of pattern matching)
├── store.rs               # SQLite + FTS5 (files, triples, code_graph, rule_intent)
├── guardrails.rs          # Term extraction, subject matching, tool scope filtering
├── hooks.rs               # Hook protocol — PreToolUse, PostToolUse, UserPromptSubmit
├── init.rs                # `arai init` flow — discover → extract → classify → scan → hook inject
├── intent.rs              # Intent classification — action (create/modify/execute), timing, tool scope
├── session.rs             # Session state — prerequisite tracking across tool calls
├── code_scanner.rs        # tree-sitter AST parsing — import extraction for 7 languages
├── enrich.rs              # Tier 2 (ONNX sentence transformer) + Tier 3 (LLM shell-out)
└── upgrade.rs             # Self-upgrade between lean/full binaries
```

## Key Design Constraints

- **Zero noise** — only fire domain-specific guardrails, never repeat CLAUDE.md content
- **Domain rules only** — rules must reference a known tool to fire on tool calls
- **Session-aware** — tracks prerequisites (e.g., "cargo test" before "git push")
- **Three enrichment tiers** — taxonomy (free) → ONNX model (local) → LLM (any provider)
- **Timing-aware** — rules route to the right hook event (PreToolUse vs UserPromptSubmit)
- **<5ms no-match hook** — fast exit when no guardrails apply
- **Single binary** — no runtime dependencies for users
