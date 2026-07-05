# Testing your rule set

Preview, diff, replay, and regression-test rule changes before they hit a live session.

## Diff — preview rule-set changes

`arai diff <file>` shows what changes a candidate edit to an
instruction file would make to the live rule set — added, removed,
moved — before you save and run `arai scan`. Read-only against
the store; pairs with `arai lint` (preview a file in isolation)
and `arai why` (preview a single tool call).

```bash
arai diff CLAUDE.md                            # Plain table view
arai diff memory/feedback_testing.md --json    # For pre-commit hooks
```

Output is grouped into three sections — `Added` (rules in the file
that aren't in the store yet), `Removed` (rules in the store whose
text isn't in the new file), `Moved` (same rule, different line
number — caught when you re-order a file without changing its rules).
JSON output keeps the same shape for CI.


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
so you don't hand-write regression tests. Flow: run your assistant, hit a
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
