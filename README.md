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
arai scan --enrich-llm     # Tier 3a: LLM classification via CLI
arai scan --enrich-api     # Tier 3b: LLM classification via API (no CLI needed)
```

Configure your LLM:
```bash
# Via CLI tool (shell-out)
ARAI_LLM_CMD="claude -p" arai scan --enrich-llm
ARAI_LLM_CMD="ollama run llama3" arai scan --enrich-llm

# Via API (OpenAI-compatible endpoints)
ARAI_API_KEY=sk-... arai scan --enrich-api                    # OpenAI (default)
ARAI_API_URL=http://localhost:11434/v1 arai scan --enrich-api  # Ollama (auto-detected)
ARAI_API_URL=https://api.groq.com/openai/v1 ARAI_API_KEY=gsk-... ARAI_API_MODEL=llama-3.3-70b-versatile arai scan --enrich-api

# Or in ~/.arai/config.toml
[enrich]
llm_command = "llm -m gpt-4o-mini"       # for --enrich-llm
api_url = "https://api.openai.com/v1"     # for --enrich-api
api_key_env = "OPENAI_API_KEY"
model = "gpt-4o-mini"
```

## Commands

```bash
arai init                  # Discover, extract, classify, scan, set up hooks
arai status                # Show what's being enforced
arai guardrails            # List all active rules
arai scan                  # Re-scan instruction files
arai scan --code           # Also scan source code (tree-sitter AST)
arai scan --enrich-llm     # Enhance rules via LLM CLI
arai scan --enrich-api     # Enhance rules via API (OpenAI-compatible)
arai add "Never X"         # Add a rule manually
arai audit                 # Inspect the local log of rule firings
arai stats                 # Aggregate audit log — top rules, tools, days
arai test scenarios.json   # Replay synthetic hook scenarios against rules
arai record --since=1h     # Capture recent firings as a scenario skeleton
arai lint CLAUDE.md        # Parse a file and preview extracted rules
arai trust                 # Manage URLs trusted for shared-policy extends
arai mcp                   # Run the MCP server (stdio) for agent-authored guards
arai upgrade --full        # Switch to full binary (with ONNX enrichment)
```

## Audit log

Every time a rule fires, Arai appends one line to a local JSONL log at
`~/.arai/audit/<project-slug>/<YYYYMMDD>.jsonl`. The log captures the
hook event, the tool that was called, a truncated prompt preview, and
every rule that matched (with source file and confidence).

Nothing leaves your machine — this is separate from the anonymous
usage telemetry below.

```bash
arai audit                    # Today's firings, table view
arai audit --since=7d         # Last week
arai audit --tool=Bash        # Only Bash tool calls
arai audit --event=PreToolUse # Only pre-tool-use firings
arai audit --json             # JSONL stream (pipe-friendly)
```

Useful for answering:

- *"Why did Claude suddenly change approach halfway through?"* —
  look up the firing, see which rule matched.
- *"Which rules are actually load-bearing?"* — sort firings by rule,
  prune rules that never trigger.
- *"Did the guardrail fire before that regrettable git push?"* —
  grep by session id.

## Status — health check your rule set

`arai status` shows how many rules are loaded, where they came from,
and when they were last scanned. As of v0.2.2 it also surfaces two
common rule-set health issues:

- **Duplicate rules** — the same (subject, predicate, object) ingested
  from more than one source file. Usually safe to consolidate into
  one source to reduce drift.
- **Opposing predicates** — the same subject carries both a
  prohibitive predicate (`never`, `must_not`, `avoid`) and a required
  predicate (`always`, `must`, `requires`, `ensure`). Not always a
  real conflict (the objects may differ), but worth a human look.

These are advisory only — the hook path ignores them. Fix them at the
source.

## Stats — aggregate the audit log

`arai stats` rolls up the same JSONL `arai audit` tails and answers
the questions every maintainer asks after a few weeks of use:

```bash
arai stats                # Top rules, tools, days since the log began
arai stats --since=30d    # Window to the last month
arai stats --top=5        # Show only top 5 per section
arai stats --json         # Machine-readable for dashboards
```

Output includes: total firings, most-fired rules, tools attracting the
most guardrails, day-by-day activity. Nothing leaves the machine —
stats are a local view over your own audit log.

## Lint — preview what a file produces

`arai lint <file>` parses an instruction file and prints every rule it
would extract along with the intent classification, without touching
the DB. Use it to iterate on CLAUDE.md wording and see the effect
before you commit.

```bash
arai lint CLAUDE.md
arai lint memory/feedback_testing.md --json   # machine-readable
```

Output for each rule: subject / predicate / object, the classified
action (Create / Modify / Execute / General), the hook timing it routes
to (ToolCall / Stop / Start / Principle), and which tools the rule
applies to.

## Test — regression harness for rules

`arai test` replays synthetic hook payloads through the *same*
`match_hook` pipeline the live hook handler uses, so rule changes get
caught before they affect a real session.

The canonical [alembic example](scenarios/alembic-migration.json) is
checked in — run it after `arai init` on any repo with an alembic rule
in CLAUDE.md:

```bash
arai test scenarios/alembic-migration.json
```

Scenario files are JSON:

```json
{
  "scenarios": [
    {
      "name": "force-push triggers the git guardrail",
      "hook": {
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": "git push --force origin master" }
      },
      "expect": {
        "matches_subject": ["git"],
        "does_not_match_subject": ["alembic"],
        "min_matches": 1
      }
    }
  ]
}
```

```bash
arai test scenarios/guards.json
arai test scenarios/guards.json --json   # structured pass/fail for CI
```

Exit code is non-zero when any scenario fails. Matches are checked by
subject substring because full SPO triples tend to drift across
re-ingest.

## Record — seed scenarios from real firings

`arai record` turns entries in the audit log into scenario skeletons
so you don't hand-write regression tests. Flow: run Claude Code, hit a
rule firing you want pinned, `arai record --since=1h > tests.json`,
tune the expectations, check in.

```bash
arai record --since=1h              # last hour
arai record --since=7d --tool=Bash  # only Bash firings from the last week
arai record --limit=50              # cap audit entries scanned
```

Deduplicates by (tool, prompt) so repeated identical firings collapse
to one scenario. Each scenario's `expect` seeds `matches_subject` with
whatever actually fired and `min_matches: 1` — tune from there.

Runtime-capturing *new rules* (as opposed to testing existing ones) is
a different loop: that goes through the MCP `arai_add_guard` tool,
documented below.

## Shared policies — `arai:extends`

Instruction files can inherit rules from a trusted upstream URL. This
is the "org-wide CLAUDE.md" pattern without a policy service — just
another markdown file hosted wherever you like.

Declare the upstream in your CLAUDE.md:

```markdown
<!-- arai:extends https://example.com/standards/rust-backend.md -->

# My project rules
- Never publish artifacts before tag push
```

Then trust the URL:

```bash
arai trust --add https://example.com/standards/rust-backend.md
arai trust                  # List trusted URLs
arai trust --remove <url>   # Revoke
```

Ārai never fetches a URL that isn't explicitly trusted. HTTPS only,
512 KB size cap, 24-hour cache with stale-while-error fallback, and
extends are not recursive — the fetched file can't pull in further
URLs. On `arai init`, trusted upstream content is inlined ahead of the
local rules before the parser runs, so the rest of the pipeline sees
one merged file.

## MCP: agent-authored guardrails

`arai mcp` runs a [Model Context Protocol](https://modelcontextprotocol.io/)
server on stdio. Two tools, exposed to any MCP-capable agent:

| Tool | What it does |
|------|--------------|
| `arai_add_guard(rule, reason?)` | Register a new guardrail mid-session. Takes effect on the next PreToolUse hook — same enforcement path as rules in your CLAUDE.md. |
| `arai_list_guards(pattern?)` | List active guardrails, optionally substring-filtered, so the agent can check what constraints are live before acting. |

This closes a gap instruction files don't cover: when an agent
discovers a rule mid-session (*"from now on, never write to /etc"*,
*"always run the full test suite before pushing"*), it now has
somewhere to register it for deterministic enforcement rather than
hoping context retention holds.

Register it with Claude Code by adding to your MCP settings:

```json
{
  "mcpServers": {
    "arai": {
      "command": "arai",
      "args": ["mcp"]
    }
  }
}
```

## Installation

```bash
# Install script (recommended)
curl -sSf https://arai.taniwha.ai/install | sh

# Full binary (with local sentence transformer)
ARAI_FULL=1 curl -sSf https://arai.taniwha.ai/install | sh

# npm
npm install -g @taniwhaai/arai

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
