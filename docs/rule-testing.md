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
`match_hook` pipeline and effective disposition calculation the live hook
handler uses. It reads the current store, source scopes, graph and session;
it writes no audit, session observations, seen-rule state, telemetry, host
responses, registrations or policy state.

### Fresh Alembic fixture

Use a fresh project with a `.git` directory, an empty isolated home and
state directory, no enrichment model/configuration, and exactly this
`CLAUDE.md` (the blank line and final newline are part of the fixture):

```markdown
## Alembic

- Never hand-write migration files
```

The checked-in [fixture](../tests/fixtures/card_a_alembic/CLAUDE.md) needs
no source files or import graph. Set `HOME`, `USERPROFILE` on Windows,
and `ARAI_BASE_DIR` to the isolated directories, and `DO_NOT_TRACK=1`.
Remove inherited Arai configuration, API/enrichment and host-identity
variables in this isolated process; keep deny enabled and bypass absent.
From the fixture project, using absolute paths to the binary and
[scenario](../scenarios/alembic-migration.json), run:

```bash
arai scan
arai test /absolute/path/to/arai/scenarios/alembic-migration.json --json
```

For native handler proof, pass each complete JSON input to
`arai guardrails --match-stdin`, replacing `<absolute fixture root>`
with that same project directory:

```json
{
  "hook_event_name": "PreToolUse",
  "tool_name": "Write",
  "tool_input": {"file_path": "alembic/versions/001_example.py", "content": ""},
  "session_id": "",
  "cwd": "<absolute fixture root>"
}
```

With host identity absent, the denied Write emits Claude-compatible
`hookSpecificOutput.permissionDecision: "deny"`, citing `CLAUDE.md:3`,
and exits 0 as required by that native protocol. Repeat with only
`file_path` changed to `app/example.py`: it exits 0 without a match or
stdout. These results are local stdin replay evidence, not live host
trust or execution evidence. Other hosts have their own response and
exit contracts. Successful hook registration only establishes a write.

The scenario asserts `decision: "deny"`, `matches_subject: ["Alembic"]`
and `min_matches: 1` for the first Write. The unrelated Write and skipped
Read assert `decision: "allow"` and `max_matches: 0`.

### Assertion schema and decisions

The file is a JSON object with a default-empty `scenarios` array; top-level
metadata such as `_comment` is accepted. Each scenario requires a string
`name` and native `hook` payload. Omitted `expect` defaults to `{}`.
The only supported assertion keys are:

| Key | Value and meaning |
| --- | --- |
| `matches_subject` | Array of strings; every substring must occur in a matched subject. |
| `does_not_match_subject` | Array of strings; none may occur in a matched subject. |
| `min_matches` / `max_matches` | Inclusive nonnegative integer bounds; omitted or explicit `null` means no bound. |
| `decision` | Exactly `"deny"`, `"advise"` or `"allow"`; supported only for PreToolUse. |

Subject substrings are case-sensitive. Assertions combine: a decision and
all supplied count/subject checks must pass. Unknown keys (including
`rule_ids`), wrong types, invalid decision strings and `decision: null`
fail validation. Known non-PreToolUse events, including PostToolUse and
UserPromptSubmit, cannot assert a decision. The whole file's assertions
are validated before replay; invalid files fail with a scenario or input
location diagnostic. Native aliases and absent-event defaults still apply.
Empty `expect` checks evaluation success. Empty scenario lists succeed.

Successful PreToolUse JSON results retain `name`, `passed`, `failures` and
`matched_subjects`, adding `decision`. Evaluation errors fail the scenario
without a decision. Other events retain legacy matching without a decision.
Failed assertions report expected/actual values; valid scenarios remain in
input order. Validation, evaluation, assertion, missing-store and command
errors cause a nonzero command exit; JSON results are printed for assertion
or evaluation failures, while validation errors prevent result evaluation.

`allow` means hard bypass, a skipped tool, or no matches remaining after
scope, inactive-rule and prerequisite filtering. An effective block with
denial enabled yields `deny`; remaining matches yield `advise`. Attached
severity overrides take precedence over predicate fallback. A previously
seen rule remains active. `ARAI_DENY_MODE=off`, `0`, `false` or `no`
(case-insensitive) makes blocking policy advisory; `ARAI_DISABLED=1`,
`true`, `on` or `yes` yields allow. The configuration `guardrails_mode`
field does not control production denial. Counts can still describe legacy
matches during hard bypass even when the effective decision is allow.

These are policy outcomes for the current effective context. Replay does
not execute tools, prove host trust/delivery, or turn transport failures
into policy denials. `advise` does not promise that every adapter can
deliver advisory context. Capture native protocol output and exits
separately, and qualify live host enforcement separately. Repeated replay
reads existing prerequisite/session context without accumulating state.
Authored rule IDs and executable canonical selectors are not supported.

```bash
arai test scenarios/guards.json
arai test scenarios/guards.json --json   # structured pass/fail for CI
```

Exit code is non-zero when any scenario fails. `init`, `scan` and
`status` share effective counts and runtime-mode labels. Extracted and
stored totals are distinct from block/advise eligibility, which is
conditional on scope and the actual call. Documentary rules such as
bare `Never force-push to main` remain inert. Advisory-only mode cannot
deny, and hard bypass has no enforcement. Test your own exact rule,
paths, content and import context rather than generalizing this fixture.


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
