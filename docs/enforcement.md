# Enforcement in depth

Deny mode, per-rule severity rollout, dry-run explanations, compliance verdicts, and rule expiry.

## Deny mode — actually block bad actions

Starting in v0.2.3, Arai no longer just *advises*: rules derived from
prohibitive predicates (`never`, `forbids`, `must_not`) emit
`permissionDecision: "deny"` (or equivalent) so the assistant refuses the tool call. Advisory
rules (`always`, `requires`, `prefers`) keep the previous behaviour.

Severity is inferred from the predicate at extract time:

| Predicate | Severity | Hook behaviour |
|-----------|----------|----------------|
| `never`, `forbids`, `must_not` | `block`  | `permissionDecision: "deny"` + reason |
| `always`, `requires`, `enforces` | `warn` | `permissionDecision: "allow"` + context |
| `prefers`, `learned_from` | `inform` | `permissionDecision: "allow"` + context |

Rolling Arai out incrementally? Flip deny mode off at the env level:

```bash
ARAI_DENY_MODE=off   # advisory-only — rules still fire in additionalContext
```

Useful pattern: ship Arai in advise mode for a week, watch `arai audit
--outcome=ignored`, tune the rules the model keeps flouting, then enable
deny mode when the rule set is trustworthy.


## Severity — per-rule deny-mode rollout

`arai severity` pins a rule's enforcement strength so re-running
`arai scan` won't reset it to the predicate-derived classification.
Use it for **incremental deny-mode rollout**: ship the rule set in
advise mode (`ARAI_DENY_MODE=off`), watch `arai stats --by-rule`,
and flip individual rules into `block` once the model is honouring
them in the wild — without forcing the whole rule set into a strict
mode it isn't ready for yet.

```bash
arai severity                          # List active overrides
arai severity alembic block            # Pin every rule whose subject/object
                                       # contains "alembic" to block
arai severity git warn                 # Demote git rules to advise-only
arai severity --reset alembic          # Drop the override; severity reverts
                                       # to the predicate-derived value
arai severity alembic block --json     # Machine-readable list of changes
```

Pattern matching is case-insensitive substring against the rule's
subject *or* object, so `arai severity migrate` covers both
`alembic must_not: hand-write migrations` and `migrations require:
backfill_plan`.

Overrides survive `arai scan` and `arai init` — they live in their
own column and are never touched by re-classification. Drop one with
`--reset` when you're ready to re-derive severity from the rule's
predicate.


## arai why — explain before you commit

`arai why <action>` replays a hypothetical tool call through the live
matching pipeline and prints the rules that would fire, with severity,
derivation (source + line + parser layer), and match percentage. No audit
write; read-only against the rule set.

```bash
arai why "git push --force origin main"
arai why --tool Write /src/migrations/001_init.py
arai why --tool Bash --event PostToolUse "rm -rf /data"
arai why "git push --force" --json   # machine-readable
```

Use it to: debug "why did that rule fire?", preview new rules before
committing them, or include the output in a PR description when you
change a CLAUDE.md.


## Compliance tracking

After every PostToolUse, Arai correlates the call against recent
PreToolUse firings in the same session and emits a `Compliance` event to
the audit log per rule:

- **obeyed** — forbidden phrase absent from the executed command (for
  prohibitive rules), or the required evidence present (for affirmative
  rules).
- **ignored** — forbidden phrase still in the executed command.
  The model ran the thing anyway (either deny was off or the assistant
  chose to proceed).
- **unclear** — not enough signal to decide (short object text, or
  affirmative rule without evidence in this call).

```bash
arai audit --event=Compliance     # all verdicts
arai audit --outcome=ignored      # shortcut for the painful ones
arai audit --outcome=obeyed       # show the rules doing their job
```

This closes the feedback loop the audit log was missing: not just *which*
rules fired, but *which ones the model actually honoured*.


## Rule expiry — self-pruning rules

Annotate rules with `(expires YYYY-MM-DD)` or `(until YYYY-MM-DD)` at the
end of the line. The annotation is stripped from the rule body at parse
time and stored separately; `load_guardrails` filters out expired rows so
the rule stops firing on its own, without you having to remember to
clean it up.

```markdown
- Never touch the old auth module (expires 2026-09-01)
- Always rebase against release-1.8 until 2026-12-31
- Prefer the new payment SDK over the legacy one (until 2027-06-30)
```

Perfect for `learned_from` incidents that have a shelf life, migration
windows, and "temporarily forbid X until we finish the refactor" rules.
