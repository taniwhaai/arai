# Arai

**CLAUDE.md that actually works.** One command. Runs locally. Zero cost.

Arai makes your AI coding assistant instruction files structurally enforceable — not just suggestions that get forgotten as context grows.

## Quick Start

```bash
curl -sSf https://arai.taniwha.ai/install | sh

cd your-project
arai init
```

That's it. Arai discovers your instruction files, extracts the rules, classifies their intent, scans your codebase for context, and sets up Claude Code hooks so guardrails fire at the right moment.

## What It Does

When Claude Code is about to do something your rules cover, Arai injects the relevant guardrail — right when it matters.

```
You: "Create a new database migration"

  PreToolUse: Write migrations/versions/001_add_users.py
  → Arai guardrails:
    - Alembic never: hand-write migration files

Claude: "I should use alembic revision --autogenerate instead..."
```

Rules only fire when relevant. No noise on `ls`. No repeating principles already in CLAUDE.md.

## How It Works

1. **Discovers** instruction files in your project and home directory
2. **Extracts** rules by pattern-matching imperative language ("never", "always", "don't", "must")
3. **Classifies** each rule's intent — what action it governs, which tools it applies to, when it should fire
4. **Scans** your codebase with tree-sitter to understand which tools own which directories
5. **Tracks** session state — knows if you've already run tests before pushing
6. **Fires** only relevant rules at the right moment via Claude Code hooks

## Supported Instruction Files

| File | Tool |
|------|------|
| `CLAUDE.md` | Claude Code |
| `~/.claude/CLAUDE.md` | Claude Code (global) |
| `.cursorrules` / `.cursor/rules` | Cursor |
| `.github/copilot-instructions.md` | GitHub Copilot |
| `.windsurfrules` | Windsurf |
| `~/.claude/projects/*/memory/*.md` | Claude Code memory |

## Smart Matching

Arai doesn't just do keyword matching. It understands your rules:

- **Intent classification** — "never hand-write migration files" only fires on Write, not Edit (editing existing migrations is fine)
- **Code graph** — writing to `migrations/versions/` triggers alembic rules even if the file doesn't mention alembic, because sibling files import it
- **Content sniffing** — detects `from alembic import op` in file content being written
- **Session awareness** — "never push without running tests" suppresses after tests have been run
- **Timing routing** — domain rules fire on tool calls, principles stay silent (already in CLAUDE.md)

## Enrichment

Three tiers of rule understanding, each more accurate:

```bash
arai scan                  # Tier 1: Built-in verb taxonomy (free, instant)
arai scan --enrich         # Tier 2: Sentence transformer model (local, ~80MB download)
arai scan --enrich-llm     # Tier 3: LLM classification (any provider)
```

Configure your LLM:
```bash
# Via environment variable
ARAI_LLM_CMD="claude -p" arai scan --enrich-llm
ARAI_LLM_CMD="ollama run llama3" arai scan --enrich-llm

# Or in ~/.arai/config.toml
[enrich]
llm_command = "llm -m gpt-4o-mini"
```

## Commands

```bash
arai init                  # Discover, extract, classify, scan, set up hooks
arai status                # Show what's being enforced
arai guardrails            # List all active rules
arai scan                  # Re-scan instruction files
arai scan --code           # Also scan source code (tree-sitter AST)
arai scan --enrich-llm     # Enhance rules via LLM
arai add "Never X"         # Add a rule manually
arai upgrade --full        # Switch to full binary (with ONNX enrichment)
```

## Installation

```bash
# Install script (recommended)
curl -sSf https://arai.taniwha.ai/install | sh

# Full binary (with local sentence transformer)
ARAI_FULL=1 curl -sSf https://arai.taniwha.ai/install | sh

# Cargo
cargo install arai
cargo install arai --features enrich   # with ONNX model support

# Homebrew
brew install taniwhaai/tap/arai
```

## Performance

| Operation | Time |
|-----------|------|
| Hook check (no match) | <5ms |
| Hook check (with match) | <12ms |
| Full init | <200ms |

## Telemetry

Arai collects anonymous usage data to help us understand if guardrails are actually useful. We track:

- Whether a rule fired and on which tool
- Hook response latency
- Rule counts and enrichment tier on init

We **never** collect file paths, rule text, code content, API keys, or anything that could identify you or your codebase.

**Opt out** at any time:

```bash
export ARAI_TELEMETRY=off   # or DO_NOT_TRACK=1
```

## Built By

[Taniwha.ai](https://taniwha.ai) — extracted from the [Kete](https://github.com/taniwhaai/kete) code intelligence platform.

## License

MIT / Apache-2.0
