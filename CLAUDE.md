# CLAUDE.md — Arai

Arai is a Rust CLI that enforces AI coding assistant instruction files (CLAUDE.md, .cursorrules, etc.) via Claude Code hooks.

## Commands

```bash
cargo build                    # Build
cargo test                     # Run tests
cargo install --path .         # Install lean binary
cargo install --path . --features enrich  # Install with ONNX enrichment
cargo run -- init              # Test init flow
cargo run -- guardrails        # List guardrails
cargo run -- status            # Show enforcement status
cargo run -- scan --code       # Re-scan with AST code graph
cargo run -- scan --enrich-llm # Enrich rules via LLM
cargo run -- add "Never X"     # Add a manual rule
cargo run -- audit             # Tail the local firing log (today)
cargo run -- audit --json      # JSONL stream
cargo run -- mcp               # Run the MCP server on stdio (blocks on stdin)
echo '{"tool_name":"Bash","tool_input":{"command":"git push"}}' | cargo run -- guardrails --match-stdin
```

## Architecture

```
src/
├── main.rs               # CLI entry (clap) — init, status, guardrails, scan, add, audit, mcp, upgrade
├── config.rs             # Config, project paths + slug, env vars, LLM command
├── discovery.rs          # Instruction file discovery (CLAUDE.md, .cursorrules, etc.)
├── parser.rs             # Rule extraction from markdown (6 layers of pattern matching)
├── store.rs              # SQLite + FTS5 (files, triples, code_graph, rule_intent)
├── guardrails.rs         # Term extraction, subject matching, tool scope filtering
├── hooks.rs              # Hook protocol — PreToolUse, PostToolUse, UserPromptSubmit; writes audit log
├── init.rs               # `arai init` flow — discover → extract → classify → scan → hook inject
├── intent.rs             # Intent classification — action (create/modify/execute), timing, tool scope
├── session.rs            # Session state — prerequisite tracking across tool calls
├── code_scanner.rs       # tree-sitter AST parsing — import extraction for 7 languages
├── enrich.rs             # Tier 2 (ONNX sentence transformer) + Tier 3 (LLM shell-out)
├── audit.rs              # Local JSONL firing log — append on hook match, query via `arai audit`
├── mcp.rs                # Stdio MCP server — arai_add_guard + arai_list_guards for agent-authored rules
├── telemetry.rs          # Anonymous usage analytics (opt-out, no project context)
└── upgrade.rs            # Self-upgrade between lean/full binaries
```

## Two layers of observability

- **`telemetry.rs`** — *anonymous usage.* Tracks aggregate counters
  ("a rule fired on some Bash call") so we can tell whether guardrails
  are useful at all. No project paths, no rule text, no code content.
  Opt-out via `ARAI_TELEMETRY=off` or `DO_NOT_TRACK=1`.
- **`audit.rs`** — *local inspection.* Per-project JSONL of every
  firing, with full rule + tool + prompt-preview context. Stays on the
  user's machine. Surfaced via `arai audit`.

They are intentionally separate paths: turning off telemetry does not
disable the local audit log, and nothing in the local audit log ever
leaves the machine.

## Key Design Constraints

- **Zero noise** — only fire domain-specific guardrails, never repeat CLAUDE.md content
- **Domain rules only** — rules must reference a known tool to fire on tool calls
- **Session-aware** — tracks prerequisites (e.g., "cargo test" before "git push")
- **Three enrichment tiers** — taxonomy (free) → ONNX model (local) → LLM (any provider)
- **Timing-aware** — rules route to the right hook event (PreToolUse vs UserPromptSubmit)
- **<5ms no-match hook** — fast exit when no guardrails apply
- **Single binary** — no runtime dependencies for users
