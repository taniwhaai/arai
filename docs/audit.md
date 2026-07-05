# The audit log

Querying the local firing log, chain verification, rule-set health, aggregate stats, and token economics.

## Audit log

Every time a rule fires, Arai appends one line to a local JSONL log at
`~/.taniwha/arai/audit/<project-slug>/<YYYYMMDD>.jsonl`. The log captures the
hook event, the tool that was called, a truncated prompt preview, the
decision (`inject`, `deny`, `review`), and every rule that matched —
with source file, line number, parser layer, severity, and confidence.

Nothing leaves your machine — this is separate from the anonymous
usage telemetry below.

```bash
arai audit                    # Today's firings, table view
arai audit --since=7d         # Last week
arai audit --tool=Bash        # Only Bash tool calls
arai audit --event=PreToolUse # Only pre-tool-use firings
arai audit --event=Compliance # Compliance verdicts (Pre/Post correlation)
arai audit --outcome=ignored  # Shortcut: Compliance events marked ignored
arai audit --rule alembic     # Filter to firings/verdicts touching this rule
arai audit --json             # JSONL stream (pipe-friendly)
arai audit --verify           # Verify the SHA-256 hash chain (exits non-zero on any tamper)
arai audit --verify --json    # Machine-readable verify report for CI / cron
```

`--rule` is a case-insensitive substring match against the rule's
subject, predicate, or object — the same shape `arai severity` uses.
Pairs naturally with `--outcome=ignored` to answer "every time the
alembic rule was ignored this week".

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
arai stats                # Top rules, compliance, token economics
arai stats --since=30d    # Window to the last month
arai stats --top=5        # Show only top 5 per section
arai stats --by-rule      # Compliance + token economics only
arai stats --json         # Machine-readable for dashboards
```

Output includes: total firings, most-fired rules, tools attracting the
most guardrails, day-by-day activity, **and a per-rule compliance
roll-up** — for every rule that has fired, how many Pre/Post pairs
ended up `obeyed` vs `ignored`, plus a ratio:

```
Per-rule compliance
  fires obeyed ignored unclear   ratio  rule
     12     11       1       0     92%  alembic must_not: hand-write migrations
      7      4       3       0     57%  git must_not: --no-verify  ⚠
      9      9       0       0    100%  cargo always: test before commit
```

The ⚠ flag highlights rules with low ratios and enough volume to
mean it — these are the ones to either rewrite (rule subject too
narrow / object too vague) or escalate via `arai severity` (see
below) once you trust the wording.

The ratio is computed **once per Pre firing** using a first-
definitive-wins rule: the first non-`unclear` Compliance verdict
correlated against a Pre is the verdict for that Pre, regardless
of how many subsequent Posts also fall inside the 5-minute
correlation window. So a rule that fires once and is honored stays
at 1 obeyed / 1 fire, not 8 obeyed / 1 fire just because eight
unrelated commands followed.

Nothing leaves the machine — stats are a local view over your own
audit log.


## Token economics — calibrated estimates

`arai stats` also surfaces a *token economics* section with
calibrated estimates of how Arai is affecting your model's token
burn. Two streams contribute:

```
Token economics (estimates)
     12  repeat-injection suppressions  (~600 tokens, 50 ea.)
      4  denied-and-honored mistakes    (~8000 tokens, 2000 ea.)
     17  advised-and-honored events     (~8500 tokens, 500 ea.)
            total estimated tokens saved:  ~17100
            (calibrated estimates, not measurements)
```

- **Repeat-injection suppressions** — when a rule fires a second
  time in the same session, Arai emits a compact "still: subject
  predicate object" line instead of re-injecting the full source /
  layer / severity payload. The model already has that context from
  the first firing. The 50-token estimate is the rough delta
  between the full and compact forms.
- **Denied-and-honored mistakes** — a `block`-severity rule fired,
  the model would otherwise have run a destructive action, and the
  PostToolUse correlation confirms it didn't. The 2000-token
  estimate is a conservative bound on what "fix the mess" cycles
  cost (revert files, undo migrations, rollback deploys).
- **Advised-and-honored events** — a `warn` or `inform` rule fired
  and the model complied. Lower confidence saving (the model might
  have done the right thing anyway), so a smaller 500-token
  estimate.

These are **estimates, not measurements**. The constants live in
[`src/stats.rs`](src/stats.rs) and are documented there; treat the
total as an order-of-magnitude reading, not a precise number. If
you want to see the underlying counts, `arai stats --json` exposes
the `token_economics` object with all three streams broken out.
