# CLAUDE.md — Arai

Arai is a Rust CLI that enforces AI coding assistant instruction files (CLAUDE.md, AGENTS.md, .cursorrules, etc.) via hooks (Claude Code + native Grok TUI).

## Commands

```bash
cargo build                    # Build
cargo test                     # Run tests
cargo install --path .         # Install lean binary
cargo install --path . --features enrich  # Install with ONNX enrichment
cargo run -- init              # Test init flow
cargo run -- guardrails        # List guardrails
cargo run -- status            # Show enforcement status
cargo run -- why "git push --force origin main"  # Explain matches (dry-run)
cargo run -- scan --code       # Re-scan with AST code graph
cargo run -- scan --enrich-llm # Enrich rules via LLM
cargo run -- add "Never X"     # Add a manual rule
cargo run -- audit             # Tail the local firing log (today)
cargo run -- audit --json      # JSONL stream
cargo run -- audit --verify    # Verify SHA-256 hash chain across all day-buckets
cargo run -- audit --event=Compliance   # Compliance verdicts (Pre/Post correlation)
cargo run -- audit --outcome=ignored    # Rules the model ran despite a Pre-firing
cargo run -- audit --rule alembic       # Filter audit by rule subject/predicate/object
cargo run -- audit --verify             # Walk hash chain across every day-bucket
cargo run -- audit --ship               # Ship pending day-buckets to [ship] url from config
cargo run -- audit --ship https://collector.example.com/arai  # One-off collector URL
cargo run -- audit --purge --older=90 --dry-run   # Plan a 90-day retention sweep
cargo run -- audit --purge --project=old-proj     # Full wipe of one project's audit
cargo run -- stats             # Aggregate the audit log (top rules, tools, days, compliance)
cargo run -- stats --by-rule   # Just the per-rule compliance ratios
cargo run -- severity          # List active severity overrides
cargo run -- severity alembic block      # Pin a rule's severity for incremental rollout
cargo run -- severity --reset alembic    # Drop the override, fall back to classification
cargo run -- diff CLAUDE.md    # Preview rule-set delta vs. live store, no writes
cargo run -- lint CLAUDE.md    # Parse a file and preview extracted rules, no DB writes
cargo run -- test scenarios/alembic-migration.json  # Replay the canonical scenario
cargo run -- record --since=1h # Build scenarios from recent audit entries
cargo run -- trust --add <url> # Approve a URL for arai:extends
cargo run -- trust --add <url> --bearer-env ARAI_EXTENDS_TOKEN  # Private source: send bearer from env var
cargo run -- migrate           # Move legacy ~/.arai → ~/.taniwha/arai (prompted, default no)
cargo run -- migrate --yes     # Non-interactive migration (for scripts)
cargo run -- mcp               # Run the MCP server on stdio (blocks on stdin)
echo '{"tool_name":"Bash","tool_input":{"command":"git push"}}' | cargo run -- guardrails --match-stdin
ARAI_DENY_MODE=off cargo run -- guardrails --match-stdin  # Advise-only (no deny)
```

## Architecture

```
src/
├── lib.rs                # Library crate root — pub mods for the embeddable core (parser, store, guardrails, hooks, audit, …); CLI-support modules #[doc(hidden)]; telemetry private
├── main.rs               # Thin CLI over the library (clap) — arg parsing + IO only; init, status, guardrails, scan, add, audit, mcp, upgrade, why
├── config.rs             # Config, project paths + slug, env vars, LLM command
├── discovery.rs          # Instruction file discovery (CLAUDE.md, .cursorrules, etc.)
├── parser.rs             # Rule extraction from markdown (7 layers of pattern matching); tracks layer + expiry
├── store.rs              # SQLite + FTS5 (files, triples, code_graph, rule_intent); expired-rule filter
├── guardrails.rs         # Term extraction, subject matching, tool scope filtering; format_trace
├── hooks.rs              # Hook protocol — PreToolUse/PostToolUse/UserPromptSubmit + FileChanged/InstructionsLoaded auto-rescan; severity → deny/allow
├── init.rs               # `arai init` flow — discover → extract → classify → scan → hook inject
├── intent.rs             # Intent classification — action, timing, tool scope, severity
├── migrate.rs            # `arai migrate` — move legacy ~/.arai → ~/.taniwha/arai (prompted)
├── session.rs            # Session state — prerequisite tracking across tool calls
├── code_scanner.rs       # tree-sitter AST parsing — import extraction for 7 languages
├── enrich.rs             # Tier 2 (ONNX sentence transformer) + Tier 3 (LLM shell-out)
├── audit.rs              # Local JSONL firing log — record_firing, record_event, layer_label
├── compliance.rs         # Pre/Post correlation — Obeyed/Ignored/Unclear verdicts per rule
├── stats.rs              # Aggregate views — `arai stats`, per-rule compliance, token economics
├── scenarios.rs          # Scenario replay harness — `arai test <file>`
├── extends.rs            # `arai:extends` upstream-policy fetch + trust list
├── mcp.rs                # Stdio MCP server — arai_add_guard + arai_list_guards for agent-authored rules
├── telemetry.rs          # Anonymous usage analytics (opt-out, no project context)
├── prompt_collector.rs   # Pure prompt-pattern collector — regex seed ruleset, PromptMatchReceipt; no enforcement
└── upgrade.rs            # Self-upgrade between lean/full binaries
```

## Prompt-collector module (`src/prompt_collector.rs`)

`prompt_collector` is a pure computation module that tests a set of labelled regex patterns against a user prompt and returns `PromptMatchReceipt` values — one per matched rule.  It carries a compiled-in seed ruleset (starter labels: `deploy`, `production`, `secret`, `password`, `kubectl apply`, `force push`) that represent observation points, not policy decisions; operators should tune this list for their context.  The `UserPromptSubmit` hook handler calls `collect_prompt_matches` with the seed ruleset and writes each receipt to the local audit log via `record_event`, so matches are visible under `arai audit --event=PromptMatch`.  The module performs no enforcement (no block, no warn, no response mutation) and makes no network calls; it is within the kete charter boundary for local-only, read-only observation of prompt content.

## Extending the match pipeline

`hooks::match_hook(hook, cfg, db)` is the single pure entry point used
by both the live stdio handler and the `arai test` scenario runner.
Any change to rule-matching logic goes there so the two paths stay
identical.  Side effects (audit write, telemetry, session write) live
only on the `handle_stdin` path.

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
- **Severity-aware** — prohibitive predicates block, affirmative predicates warn, prefers informs
- **~22 ms skip-tool fast-exit, ~32 ms full match (median)** — see `bench/hot_path.sh` for the breakdown; cost is dominated by binary fork+exec, not matching
- **Single binary** — no runtime dependencies for users

## v0.2.11 additions at a glance

- **Twelve new parser patterns** (`parser::match_imperative`) shipped
  together so users who write rules in any of these styles get them
  honoured rather than silently dropped.  Severity mapping mirrors
  grammatical weight (`should` is softer than `must`, so it routes to
  `prefers`/Inform; `should not` is an explicit prohibition so it
  routes to `must_not`/Block).
- **New Layer 7 (`conditional imperative`)** — catches the trigger-
  paired-with-imperative shape that previously slipped past every
  layer ("When working in parallel, run tests in isolation").
- **Layer 5 section-context gate** — `^use\s+` now also fires when the
  section header matches `Conventions / Rules / Style / Guidelines /
  Best Practices / ...`, capturing the style-guide pattern where the
  framing makes the imperative explicit.
- **Bold-label discriminator** (`is_bold_label`) — `**No build process**
  - this is a zero-build extension.` is feature-absence DESCRIPTION,
  not a rule; the guard prevents `^no` and `^consider` from
  over-extracting on labelled list items, while still letting
  `**Always** run tests` (bold emphasis on a Layer 1 leader) extract
  normally.
- **Coverage corpus + integration test** — `tests/parser_coverage/`
  ships a synthetic CLAUDE.md exercising every pattern with positive
  AND negative cases, plus a `tests/parser_coverage.rs` integration
  test that drives the live `arai lint --json` binary.  Locks the
  expected behaviour so future parser changes can't silently
  regress on the most-common shapes.

## v0.2.9 additions at a glance

- **Per-session repeat-injection suppression** (`session::partition_
  seen_rules`, `session::mark_rules_seen`).  When the same rule fires
  a second time in a session the hook emits a compact one-liner
  (`- still: subject predicate object`) instead of re-injecting the
  full source/layer/severity payload.  Saves ~50 tokens per re-fire
  and reduces attention dilution.  State lives in the existing
  per-session JSON pattern, capped at 500 ids.  Mark-as-seen happens
  *after* the audit write so a panic between match and write can't
  permanently suppress a rule the model never saw.
- **`seen_before` per-rule audit field**.  Additive boolean on
  every entry in `rules[]`; older lines are read as `false`.  Lets
  `arai stats` count suppression events post-hoc without holding
  any session state of its own.
- **Token-economics section in `arai stats`** (`stats::TokenEconomics`).
  Three streams: repeat-injection suppressions (~50 ea.), denied-
  and-honored mistakes (`obeyed` + `block` severity, ~2000 ea.),
  advised-and-honored events (~500 ea.).  Calibration constants
  documented in `src/stats.rs` so they move atomically.  Output is
  labelled "estimates, not measurements" everywhere — over-claiming
  here is the easy mistake to make.  Suppressed entirely when no
  streams have data, so first-run users don't see a "0 saved" line.
  `arai stats --json` exposes a `token_economics` object.

## v0.2.6 additions at a glance

- **Per-rule compliance roll-up** in `arai stats` — joins Pre firings
  and Compliance verdicts via `triple_id` to produce
  `fires/obeyed/ignored/unclear/ratio` per rule. `--by-rule` shows
  only that section. The maintainer's "is this rule actually
  working?" question, answered from data Arai already collects.
  *(v0.2.7: dedupe per Pre — first-definitive-wins so unrelated
  Posts don't inflate the denominator.)*
- **Per-rule severity override** (`arai severity`).  Stored in a new
  `rule_intent.severity_override` column that `classify_all_guardrails`
  doesn't touch on re-scan, so manual rollout decisions survive.
  `get_rule_intent` returns the override when set; falls back to the
  classified severity otherwise.
- **`arai_recent_decisions` MCP tool**.  Mirror of the maintainer-
  side audit feed, exposed to the agent so it can self-check after a
  deny without parsing the on-screen reason or re-trying the same
  thing twice.  Strips `Compliance` events (verdicts, not decisions).
- **`arai audit --rule <pattern>`** filter.  Substring match against
  rule subject/predicate/object across both top-level firings and
  Compliance `payload.rules[]`.  Pairs with `--outcome=ignored`.
- **`arai diff <file>`**.  Preview rule-set delta against the live
  store before saving an instruction-file edit; emits added /
  removed / moved (line number changed for an unchanged SPO).
  Pre-commit-hook fodder via `--json`.

## v0.2.3 additions at a glance

- **Severity + deny mode** (`intent::Severity`).  `never` / `forbids` /
  `must_not` → `Block` → `permissionDecision: "deny"` on PreToolUse.
  `ARAI_DENY_MODE=off` forces advise-only for incremental rollout.
- **Rule derivation trace** (`parser::Triple::layer`,
  `Guardrail::layer`, `Guardrail::line_start`, `audit::layer_label`).
  Every firing records which of the seven `match_imperative` layers
  produced the rule, plus the source line — exposed via
  `additionalContext`, the audit JSON, and `arai why`.
- **Compliance tracking** (`compliance.rs`).  On PostToolUse, correlate
  against recent PreToolUse firings and emit a `Compliance` audit event
  with an `Obeyed | Ignored | Unclear` verdict per rule.  CLI filter:
  `arai audit --outcome=ignored`.
- **Rule expiry** (`parser::Triple::expires_at`,
  `parser::extract_expiry`).  Trailing `(expires YYYY-MM-DD)` or
  `(until YYYY-MM-DD)` annotation parses into a date; SQL filter drops
  expired rules on load.
- **`arai why`** (in `main.rs`).  Read-only dry-run through the live
  `match_hook` pipeline.  Shows matched rules, severity, layer, source,
  line, and match-percentage.

## rustc 1.95 caveat

The dead-code liveness pass in rustc 1.95 ICEs on certain `pub fn`
shapes declared inside `parser.rs` (specifically a `match`-returning-
`&'static str` with escaped quote strings).  If you add a new helper in
`parser.rs` and `cargo check` panics with
`#0 [check_mod_deathness] checking deathness of variables in module 'parser'`,
move the helper to `audit.rs` or another module.  `audit::layer_label`
was moved for this reason.
