# Plan: Grok Build hook gap — verify, qualify, report, probe

Status: **draft, revision 2** (post peer review). Nothing has been executed
except the diagnosis in §1. No issues filed, no comments posted, no branches
cut.

Revision 2 changes: Phase 0 is now host-only first, in a clean temporary
repository (§4); evidence claims narrowed to what was actually observed
(§1, §7); the Phase 0 / "ship immediately" contradiction resolved (§6);
`src/init.rs` moved out of the docs PR (§6a); Defect A escalated from warn to
refuse, and extended to the file-extraction path (§6b); upstream report
reworded to specify behaviour rather than implementation (§5).

---

## 1. Diagnosis — observed, do not re-litigate

Host: grok **0.2.112**, WSL2 (Ubuntu), session over **ACP** (`grok-pager`
front end + `shell` backend). Arai **1.1.1** at `~/.cargo/bin/arai`.

| Claim | Status | Evidence |
|---|---|---|
| `.grok/hooks` + `.claude/settings.json` hooks are discovered and displayed | **True** | `/hooks-list` shows 6, incl. 4 Arai |
| Folder trust was the initial blocker | **True** | `~/.grok/trusted_folders.toml` absent until `--trust` |
| File hooks are invoked on the ACP tool path | **No evidence of invocation** | see below |
| Arai was invoked at all | **No evidence** | no audit entry in any project bucket after 03:50 UTC |
| `ARAI_DISABLED` explains the silence | **Ruled out** | that path writes a `bypassed` audit entry; none present |
| Arai matching is at fault | **Ruled out** | probe rule matches at 98% via `arai why` |
| Vendor's own tool name | `run_terminal_command` | confirms PR #172's alias target |

Decisive log line, at the moment of the probe:

```json
{"ts":"2026-07-27T04:25:15.928Z","src":"shell","msg":"shell.tool.exec_done",
 "ctx":{"tool_name":"run_terminal_command","elapsed_ms":200,"success":false}}
```

Preceded by `shell.turn.tool_prep_done`, with **no hook step between them**.
Across the full ~1.7 MB `~/.grok/logs/unified.jsonl`: **zero** occurrences of
`hook`, `pre_tool`, `guardrail`, or `arai`.

**Working conclusion: a Grok Build defect on the ACP tool path.** Not Arai
matching, not registration, not trust, not the binary. Phase 0 exists to make
this conclusion unassailable rather than merely well-supported.

Contradicts the vendor's own `15-agent-mode.md`: *"Deny rules and hooks still
apply."*

### Incidental observations

- **Effective provider suppression.** `.grok/hooks/arai.json` entries never
  appear in `/hooks-list`; the displayed providers are `.claude/settings.json`
  (Claude compatibility) and the global `~/.grok/hooks/arai-pretool.json`.
  Identical `(event, command)` registrations appear to collapse — but the
  mechanism is unverified. It could equally be source precedence,
  compatibility-layer canonicalisation, display-only collapsing with multiple
  registrations retained at execution time, or filtering of unsupported
  events. Claude-only events (`FileChanged`, `InstructionsLoaded`,
  `CwdChanged`, `PostToolBatch`) are not displayed, as PR #172 predicted.
  **Needs its own minimal repro. Not part of the headline report.**
- **`PermissionDenied` is registered** through the Claude-compat layer and
  appears in `/hooks-list`. **Invocation is unverified.** The documented event
  is passive (`10-hooks.md`, can-block: No), so the immediate enforcement risk
  appears low — but PR #172's description claims it is unregistered, and that
  is wrong regardless of whether it ever fires.
- **Advisory path, from vendor docs rather than testing.** `10-hooks.md`
  documents the `PreToolUse` contract as `{"decision":"allow"}` and
  `{"decision":"deny","reason":…}`. `additionalContext` appears only under
  `hookSpecificOutput` for `Stop`/`SubagentStop`. Precisely: **the documented
  `PreToolUse` contract in 0.2.112 exposes no context-injection field.**
  Absence from documentation is not proof that no undocumented behaviour
  exists, but it is sufficient to justify qualifying Arai's advisory claim.

---

## 2. Seams available to Arai

| # | Seam | Reach | Status |
|---|---|---|---|
| 1 | `.grok/hooks/*.json` command hooks | everyone | Arai's current surface. Discovered on ACP, **no evidence of invocation** |
| 2 | Client-registered `PreToolUse` gates (`grok-agent-sdk`, `timeoutS`, 30 s default) | custom ACP clients only — **not** grok-pager | **Unverified**; same doc family already broke seam 1 |
| 3 | ACP permission step | agent mode | Off under `permission_mode = "always-approve"`, which the vendor recommends for agent servers. Not usable in practice |
| 4 | ACP proxy (Arai interposed between client and agent) | everyone | Large, fragile surface. **Hold** |

---

## 3. Goals, in priority order

1. Establish, with an isolated test, whether the host invokes hooks at all —
   and on which entrypoints (**gates everything else**).
2. Freeze the evidence while it is still reconstructible.
3. Report upstream — zero Arai code if xAI honours their documented contract.
4. Stop making unqualified public enforcement claims.
5. Fix our own silent-enforcement defects.
6. Decide the SDK-gate question by experiment, not by reading docs.
7. Close out #161.

Explicitly **out of scope**: implementing an SDK adapter, an ACP proxy, or any
new hook protocol. No feature code on those until Phase 5 returns a verdict.

---

## 4. Phase 0 — isolated host probe (BLOCKING)

Everything downstream asserts a TUI/ACP split. **That split is currently
unverified.** Every successful deny observed so far was `--match-stdin`, which
bypasses the host entirely. No hook has been observed firing from any Grok
session on this machine.

The earlier draft proposed testing this with Arai, rule 78, the existing repo
and the existing audit store. That is the plan's own lesson violated: **do not
use Arai to prove whether the host invokes hooks.** A negative result would
remain contestable on binary resolution, provider suppression, rule storage or
audit-bucket selection — every one of which this investigation has already
found to be a live confound in this repository.

### 0A — host-only probe, interactive TUI, clean directory

1. Fresh temporary directory, `git init`, nothing else.
2. Exactly one `PreToolUse` hook, registered with an **absolute** command path.
   It appends a timestamp to a uniquely-named marker file and returns
   `{"decision":"deny","reason":"<unique string>"}`.
3. Open the interactive TUI there; `--trust`; confirm via `/hooks-list`.
4. One harmless tool call.
5. Record three things independently: did the marker file appear; did the
   command run; did the unique denial reason reach the agent.

| Marker | Command | Meaning |
|---|---|---|
| absent | runs | Host never invoked the hook |
| present | runs | Hook invoked, denial ignored |
| present | blocked | Host hook contract works — proceed to 0B |
| absent | blocked | Something else blocked it; test contaminated, re-run |

### 0B — Arai probe, interactive TUI

Only meaningful if 0A lands on "present + blocked". Same repo as 0A with Arai
hooks registered, or the arai repo with the confounds noted. `git log
--arai-probe` matches block rule 78 at 98% and is inert if unblocked.

A 0A pass with a 0B failure isolates the problem to Arai and is itself a
valuable result.

### 0C — host-only probe over ACP (mandatory, not optional)

**Same hook file, same tool call, ACP entrypoint.** The reviewer marked this
"if needed"; it is not optional. The matched comparison — identical trivial
hook, identical call, differing only by entrypoint — is what makes the
TUI/ACP split indisputable rather than inferred. Without it the upstream
report rests on a comparison between a trivial hook on one path and Arai on
the other.

Timebox: 30 minutes for all three. Do not start Phase 2 or 3 before 0A and 0C
return.

---

## 5. Phase 1 — freeze the evidence

**Immediately after Phase 0, before channel discovery.** Evidence decays: logs
rotate, versions move, configuration drifts, repro state becomes hard to
reconstruct. Write the canonical document first; resolve where to send it in
parallel.

**Deliverable:** `docs/upstream/grok-acp-hooks-not-firing.md`.

**Title:** ACP tool path shows no PreToolUse/PostToolUse hook invocation
despite registration (grok 0.2.112)

**Contents:**

- Environment: version, OS, client (grok-pager / ACP), `permission_mode`,
  trust granted, `/hooks-list` output
- Expected, quoting their docs: `15-agent-mode.md` "Deny rules and hooks still
  apply"; `10-hooks.md` PreToolUse deny contract and discovery rules
- Actual: tool executes, no observable hook invocation
- Evidence: Phase 0A/0C matched results; the `shell.tool.exec_done` line; the
  absent hook step; the whole-log grep; the absent Arai audit entry;
  `/hooks-list` still listing the hooks
- **Host-only repro**, no Arai required — Phase 0's trivial marker hook,
  verbatim
- **Ask** (behaviour, not implementation): *ensure registered `PreToolUse` and
  `PostToolUse` command hooks are invoked for tool calls initiated through the
  documented ACP/agent-mode path, with deny decisions honoured according to the
  published hook contract.* We do not know whether the dispatcher belongs to
  the TUI, the shell backend, or shared middleware, and should not prescribe
  it.
- Framing: documented-contract violation, **not** a feature request, **not**
  "add Arai support"

Do not bundle the SDK-gate question or the provider-suppression observation
into this report — different claims, differently evidenced, and they muddy a
clean bug.

Then: identify the channel (public tracker vs private contact), submit, and
**record the ticket or delivery reference**. If submission proves impossible,
document the channels attempted and the failure.

---

## 6. Phase 2 — correct our own claims

Two categories, two different PRs.

**Sequencing.** Revision 1 said these "ship immediately" while also saying
Phase 0 gates them — a contradiction the reviewer caught. Resolved: **prepare
the wording immediately, merge only once Phase 0 establishes whether the
limitation is ACP-specific or affects all Grok Build entrypoints.** Otherwise
we replace one false public claim with another.

The docs PR is gated on **Phase 0 only** — not on upstream submission. Channel
discovery could take days and our public claims are wrong today.

### 6a. Host qualification — genuine docs-only PR

| File | Change |
|---|---|
| `README.md` | Split Grok Build into interactive TUI vs ACP/agent mode in the enforcement table and the bullets beneath |
| `site/index.html` | Same split in the support matrix (~line 359) and the two paragraphs pairing "Claude Code and Grok Build" (~414, ~520) |
| `docs/enforcement.md` | Short "host limitations" note (file exists) |
| `CHANGELOG.md` | Unreleased documentation entry |

`src/init.rs` is **not** in this PR. Revision 1 called the bundle "docs-only"
while including a Rust source change — it needs compilation, possibly CLI-output
test updates, and release-note consideration. It moves to 6b.

Wording constraints: name the **host entrypoint**, not the brand. Do not say
"Arai is broken on Grok" — say the ACP tool path shows no hook invocation. Do
not promise an adapter or proxy. Do not walk back Claude Code or TUI claims
without Phase 0 evidence.

**Do not expand this PR to Cursor and Windsurf.** The entrypoint-vs-brand
lesson generalises, but those rows lack equivalent evidence and changing them
here is scope creep. Open a follow-up issue to audit every support-table row
by host, entrypoint, enforcement mode and verified version — a matrix embodies
the lesson better than prose caveats do:

| Product | Entrypoint | Integration | Can block | Can advise | Verified version |
|---|---|---|---|---|---|
| Claude Code | Native hooks | Command hook | Yes | Yes | … |
| Grok Build | Interactive TUI | Command hook | Phase 0 | Limited | 0.2.112 |
| Grok Build | ACP / agent mode | Command hook | No — host defect | No | 0.2.112 |
| Cursor | MCP | Advisory | No | Yes | … |

### 6b. Arai-side defects — ours, not xAI's

Both are the same shape as the headline bug: *enforcement that presents as
present and isn't.*

**Defect A — silently inert rules.** `arai add "Never run echo
test161_block_marker"` prints `Added:`, appears in `arai guardrails`, and can
never fire: `arai why` returns 0 rules, because "domain rules only" requires a
known tool in the subject and `echo` is not one. Two such rules are live in
this repo's store (triple-ids 73, 74), and earlier verification that leaned on
them measured nothing.

*Worse than the ACP defect*, because the user authors a rule and is told it
succeeded.

Revision 1 proposed a warning. That is too weak — `Added:` followed by a
warning still reads as success. Revised:

- **`arai add`: refuse by default** when the compiled rule has zero eligible
  tool domains. Explain why, suggest a rewrite, and offer `--allow-inert` for
  deliberately documentary rules.
- **File-extraction path (`arai scan`) cannot refuse** — rules arrive from
  CLAUDE.md/AGENTS.md wholesale, and rejecting them would break ingestion.
  This is the larger surface: most rules come from files, not `arai add`.
  There, inert rules must be **visibly flagged** in `arai guardrails`,
  `arai lint` and `arai diff`.

Proposed message:

```text
Rule not added: its subject does not map to any enforceable tool domain.
"echo" is treated as a command argument, not an enforceable tool subject.
Rewrite the rule against the shell/Bash tool, or pass --allow-inert to
retain it as a non-enforcing rule.
```

Note this is a behaviour change to an existing CLI command — scripts that call
`arai add` with inert rules will start failing. Minor-version note required.

Regression test covers the whole user path, not just `arai why`: the exit
status, the absence of an unconditional `Added:`, and that a retained inert
rule is visibly marked in `arai guardrails`.

**Defect B — stale binary fails open silently.** A stale `arai` earlier on
`PATH` (here: a 0.2.13 Windows PE at `/mnt/c/Users/tim/.local/bin/arai`, first
on a non-login shell's PATH) exits 0 with empty output on a payload 1.1.1
denies. Per `10-hooks.md`, all hook failures are fail-open, so the host reads
that as consent.

Absolute-path registration alone is insufficient — the recorded path can later
hold an old or replaced binary. Layered fix:

1. canonical absolute binary path in generated hook configuration;
2. hook invocations record binary version and canonical path in the audit
   entry;
3. a verification command comparing every generated hook command against
   `current_exe()`, warning when multiple `arai` binaries exist on relevant
   paths.

(3) implies a new subcommand — there is no `arai doctor` today. Scope
accordingly. Diagnostics must stay on stderr or in the audit channel: the host
expects strict protocol JSON on stdout.

Also in 6b: the `src/init.rs` string, which currently promises "Arai is
enforcing this project's rules (Claude Code and Grok Build)" without an
entrypoint caveat.

---

## 7. Phase 3 — #161 close-out

- **Naming** — done, merged in #172.
- **Advisory context** — close citing `10-hooks.md`, worded as *the documented
  contract exposes no context-injection field*, not as proof none exists.
- **PermissionDenied** — reword to: *registered through the Claude-compat
  layer and visible in `/hooks-list`; invocation unverified; the documented
  event is passive, so immediate risk appears low.* PR #172's description says
  unregistered, which is wrong. Do **not** call it "live" — that repeats the
  discovery-is-not-execution error this whole investigation is about.

Plus the Phase 0 result, and a note that live end-to-end verification is
blocked on the host rather than pending our effort.

---

## 8. Phase 4 — SDK gate probe (decide, do not build)

Purpose: establish whether a deny gate registered via `grok-agent-sdk` is
invoked and honoured on the ACP path. **Timebox half a day.**

| Result | Action |
|---|---|
| Gate invoked, deny honoured | Design issue for an optional adapter. Document its reach honestly — custom ACP clients only, **not** grok-pager. Upstream fix remains primary |
| Gate not invoked, or deny ignored | Record as a second broken seam. **Do not build** |
| Ambiguous | Tighten the probe (matcher, `timeoutS`, permission mode). Ship no product code |

**Preserve the probe, don't discard it.** A prose report without the client is
not reproducible. Keep the smallest working client in a clearly-labelled
`experiments/` directory excluded from product builds, or as a complete
appendix to the report. Record: exact SDK package and resolved versions,
permission mode, timeout, launch commands for both sides, the exact tool
request, whether the callback ran, and whether its denial affected execution.

Confirm the gate-registration API against current SDK types at probe time. A
doc claim is not evidence — that is the lesson of this entire investigation.

---

## 9. Explicit non-work

- No ACP proxy in this effort.
- No production SDK adapter until Phase 4 is green.
- No changes to matching, `normalize_tool_name`, or the dual response formats.
  PR #172's alias already matches the vendor's `run_terminal_command`, now
  confirmed by the host's own logs.
- No Cursor/Windsurf claim changes in the Grok correction PR.

---

## 10. Order

```text
[0A] Host-only hook probe, interactive TUI, clean temp repo   BLOCKING
[0B] Arai probe, interactive TUI                              (if 0A passes)
[0C] Host-only hook probe, ACP, same hook + same call         BLOCKING
[1]  Freeze evidence → canonical upstream report
[2]  Identify channel, submit, record reference               (parallel with 3)
[3]  Grok claims-correction docs PR                           (gated on 0 only)
[4]  File Defect A and Defect B issues separately
[5]  Defect A implementation PR (+ init.rs caveat)
[6]  #161 close-out with qualified language
[7]  SDK gate probe → record verdict → stop, or schedule adapter design
[8]  Follow-up issue: audit every support-table row by entrypoint
```

Steps 2 and 3 run in parallel — the docs correction must not wait on xAI.

---

## 11. Acceptance criteria

- [ ] Phase 0A and 0C complete, using the same trivial hook and same tool call
- [ ] TUI/ACP split either confirmed or corrected, with the result recorded
- [ ] Upstream report exists with a host-only repro and matched Phase 0 evidence
- [ ] Report **submitted** and its ticket/message/delivery reference recorded —
      or the attempted channels and failure documented
- [ ] README, `site/index.html`, `docs/enforcement.md`, `CHANGELOG.md`
      distinguish TUI from ACP
- [ ] `arai add` **detects and clearly rejects or flags** a rule that cannot
      match any tool; inert rules extracted from files are visibly marked
- [ ] `arai init` copy no longer promises unqualified enforcement
- [ ] Stale-binary fail-open filed, with the layered fix scoped
- [ ] #161 updated: naming and alias ticked, advise closed on the documented
      contract, PermissionDenied reworded as registered-not-verified
- [ ] SDK probe recorded as pass-with-next-step or fail-no-build, with the
      probe client preserved; no adapter code merged
- [ ] No proxy work started

---

## 12. Open questions

1. **Does any Grok entrypoint invoke hooks on 0.2.112?** Phase 0. Load-bearing
   for the framing of everything after it.
2. **Where do Grok Build bugs get filed?** Blocks submission, not evidence.
3. **Does the SDK gate API exist as documented?** Confirm against types.
4. **What actually suppresses duplicate providers?** Deduplication, precedence,
   display-only collapsing, or event filtering — unverified. Separate repro.

---

## 13. Lessons (append to `tasks/lessons.md` on ship)

- **Do not use Arai to prove whether the host invokes hooks.** First prove the
  host invokes a trivial hook; then prove it invokes Arai.
- Never build on an unverified seam from a doc set that has already broken a
  claim.
- Public enforcement claims must name the host **entrypoint**, not the product
  brand.
- "Registered" is not "invoked". `/hooks-list`, `arai guardrails`, and an
  `Added:` confirmation all report registration; none report execution.
  *Revision 1 of this plan broke that rule twice in its own prose.*
- A test fixture that cannot fire is worse than no fixture. Verify a probe
  *can* match before trusting a negative result.
- Evidence decays. Freeze it before resolving where to send it.

---

## Appendix — re-verification commands

```bash
# Diagnosis (§1)
arai why "git log --arai-probe"                       # expect 98%, block
arai audit --json | grep 2026-07-27 | tail -5         # expect no live-session entry
grep -icE "hook|pre_tool|guardrail|arai" ~/.grok/logs/unified.jsonl   # expect 0
grep "shell.tool.exec_done" ~/.grok/logs/unified.jsonl | tail -2

# Defect A (§6b)
arai why "echo test161_block_marker"                  # expect 0 rules

# Housekeeping — probe rule 78 also matches plain `git log`
arai disable 78
```
