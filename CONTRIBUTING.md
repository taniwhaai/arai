# Contributing to Arai

Thanks for your interest in contributing to Arai. This guide will help you get started.

## Development Setup

```bash
git clone https://github.com/taniwhaai/arai.git
cd arai
cargo build
cargo test
```

Contribution flow is **PR-to-main**. Do not push feature work straight to `main`.

### Build variants

```bash
cargo build                        # Lean (default, ~9MB)
cargo build --features enrich      # Full with ONNX sentence transformer (~32MB)
```

## Making Changes

### Commit messages

We use [Conventional Commits](https://www.conventionalcommits.org/) for automated changelog generation:

```
feat: add support for .editorconfig rules
fix: false positive on "go" in "going forward"
refactor: extract term matching into shared module
docs: update README with new install methods
test: add integration test for session tracking
chore: bump tree-sitter-python to 0.26
```

### Running tests

```bash
cargo test                         # Unit + integration suite
cargo test parser                  # Parser tests only
cargo test intent                  # Intent classification tests
cargo test session                 # Session tracking tests
cargo test --features enrich       # Tests with ONNX feature
```

Do not pin a test count in this file; the suite grows. Run `cargo test` before opening a PR.

### Project structure

```
src/
├── lib.rs             # Library crate — documented embedding API
├── main.rs            # Thin CLI over the library (clap)
├── config.rs          # Configuration, paths, env vars
├── discovery.rs       # Find instruction files (scoped / nested)
├── parser.rs          # Extract rules from markdown
├── intent.rs          # Classify rule intent (action, timing, tool scope)
├── store.rs           # SQLite persistence
├── guardrails.rs      # Term extraction + matching
├── hooks.rs           # Host hook protocol (Claude Code, Grok Build, Codex, Cursor)
├── cursor.rs          # Cursor Agent hook payloads → canonical envelope
├── init.rs            # arai init / deinit — register and remove hooks
├── session.rs         # Session state + prerequisite tracking
├── code_scanner.rs    # tree-sitter AST import extraction
├── enrich.rs          # Sentence transformer + LLM enrichment
├── audit.rs           # Hash-chained local JSONL log
├── compliance.rs      # Pre/Post obeyed / ignored / unclear
├── canonicalize.rs    # Instruction files → arai.toml
├── sync.rs            # arai.toml → per-tool instruction files
├── repo_check.rs      # git-diff matcher (check-diff / pre-commit)
├── mcp.rs             # Stdio MCP server
├── extends.rs         # arai:extends upstream policy
├── ship.rs            # arai audit --ship
├── migrate.rs         # ~/.arai → ~/.taniwha/arai layout
├── upgrade.rs         # Self-upgrade between binary variants
└── …
```

Native host payload details live in `codex.rs` and `hooks.rs`. Integration tests live under `tests/`.

## What to Contribute

### Good first issues

- Improve parser pattern matching for edge cases
- Add more languages to the tree-sitter code scanner
- Expand the known tools list and host tool-name aliases
- Add integration tests for a host hook payload
- Improve subject extraction accuracy

### Bigger contributions

These are still open. Do not treat them as missing product surface that already shipped:

- Web dashboard for rule management
- Rule-pack publication (canonical `arai.toml` packs beyond a single file)
- Additional native PreToolUse hosts beyond Claude Code, Grok Build, Codex, and Cursor

Already shipped — do not re-propose:

- Blocking mode (`permissionDecision: "deny"` / Grok `decision: deny`)
- `arai deinit`
- Grok Build native hooks
- Codex native hooks (v1.1.2)
- Direct LLM API enrichment (`arai scan --enrich-api`)
- `arai check-diff` / `arai init --pre-commit`
- `arai canonicalize` / `arai sync`

## Guidelines

- Run `cargo test` before submitting a PR
- Keep the lean binary under 15MB
- Hook responses should stay under 50 ms median end-to-end (cold-start floor is ~20 ms; matching adds 5–15 ms). Run `bench/hot_path.sh` before/after perf-sensitive changes and post the before/after table in the commit body.
- Don't add network calls to the hook path (only at scan/enrich/ship time)
- Prefer expanding the verb taxonomy over adding ML complexity
- Host-integration changes must keep Claude Code, Grok Build, Codex, and Cursor paths tested; a missing tool-name alias is a silent fail-open

## License

By contributing, you agree that your contributions will be dual-licensed under
the same MIT OR Apache-2.0 terms as the project (see [LICENSE-MIT](LICENSE-MIT)
and [LICENSE-APACHE](LICENSE-APACHE)), without any additional terms or
conditions.
