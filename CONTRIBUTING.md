# Contributing to Arai

Thanks for your interest in contributing to Arai! This guide will help you get started.

## Development Setup

```bash
git clone https://github.com/taniwhaai/arai.git
cd arai
cargo build
cargo test
```

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
cargo test                         # All 52 tests
cargo test parser                  # Parser tests only
cargo test intent                  # Intent classification tests
cargo test session                 # Session tracking tests
cargo test --features enrich       # Tests with ONNX feature
```

### Project structure

```
src/
├── main.rs            # CLI entry point (clap)
├── config.rs          # Configuration, paths, env vars
├── discovery.rs       # Find instruction files
├── parser.rs          # Extract rules from markdown
├── intent.rs          # Classify rule intent (action, timing, tool scope)
├── store.rs           # SQLite persistence
├── guardrails.rs      # Term extraction + matching
├── hooks.rs           # Claude Code hook protocol (stdin/stdout JSON)
├── session.rs         # Session state + prerequisite tracking
├── code_scanner.rs    # tree-sitter AST import extraction
├── enrich.rs          # Sentence transformer + LLM enrichment
└── upgrade.rs         # Self-upgrade between binary variants
```

## What to Contribute

### Good first issues

- Improve parser pattern matching for edge cases
- Add more languages to the tree-sitter code scanner
- Expand the known tools list in `parser.rs`
- Add integration tests for hook protocol
- Improve subject extraction accuracy

### Bigger contributions

- Direct LLM API support ([#1](https://github.com/taniwhaai/arai/issues/1))
- Support for additional AI coding tools beyond Claude Code
- Blocking mode (`permissionDecision: "deny"`) for critical guardrails
- `arai deinit` to cleanly remove hooks
- Web dashboard for rule management

## Guidelines

- Run `cargo test` before submitting a PR
- Keep the lean binary under 15MB
- Hook responses must stay under 20ms
- Don't add network calls to the hook path (only at scan/enrich time)
- Prefer expanding the verb taxonomy over adding ML complexity

## License

By contributing, you agree that your contributions will be licensed under the Apache-2.0 license.
