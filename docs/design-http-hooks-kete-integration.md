# Design — HTTP hooks as the Arai ↔ Kete policy transport

**Status:** Draft. Not implemented. Soliciting review on the contract before any code.
**Tracking:** [arai#110](https://github.com/taniwhaai/arai/issues/110) Tier-3 deliverable; pairs with the Kete roadmap re-scope at [kete#1](https://github.com/taniwhaai/kete/issues/1).
**Authors:** [@Tim-Marsden](https://github.com/Tim-Marsden) + Claude (initial draft)

---

## Problem

The Arai/Kete split — Arai handles per-developer enforcement locally; Kete handles fleet-layer concerns (centralised distribution, cross-developer rollup, org retention policy) — works today only by *convention*. There's no transport between the two. Each developer's Arai install has its own SQLite, its own audit JSONL, its own rule extraction. Kete has no concrete API surface.

The gap manifests in three real scenarios:

1. **Centralised rule distribution.** An org-wide rule set lives in Kete (or a Kete-fronted git repo). Today the only way to deploy it is `arai:extends https://...` per-developer. Works, but it's pull-only; the org can't push a new rule version to a fleet.
2. **Cross-developer compliance rollup.** "Show me every developer who's been auto-denied on `git push --force` this week" needs a central store with the audit lines. Today each developer's audit log stays local; there's no aggregation path.
3. **Policy as a service.** A regulated team may want all enforcement decisions to flow through a server that logs them server-side, applies org-wide policy not expressible in CLAUDE.md, and returns deny/allow per call. That server is Kete.

Claude Code recently added **HTTP hooks** as a hook handler type — instead of running a local command, the hook POSTs the event JSON to a URL and reads the decision from the response body. This is the transport we need.

## Proposal (one-paragraph version)

When an organisation opts in, Arai's settings.json registers Kete's HTTP-hook endpoint *in addition to* the local `arai guardrails --match-stdin` command. Both run on each tool call; the deny-or-allow decision is the **most restrictive of the two**. Arai's local store stays the source of truth for per-developer guardrails (the CLAUDE.md + `arai:extends` rules each developer's machine sees); Kete adds an org-policy layer that no individual developer can disable or talk around. The local audit log keeps its tamper-evident hash chain; Kete keeps its own fleet-aggregated log fed by the same event stream.

## Non-goals

- Replacing Arai's local enforcement. Kete is an *additional* gate, not a substitute. Air-gapped / offline developers must still get per-machine enforcement when the Kete endpoint is unreachable.
- Routing every Anthropic API call through Kete. We're hooking Claude Code's hook surface, not the LLM transport.
- A general-purpose policy DSL. Kete inherits Arai's triple-store model (subject/predicate/object + intent classification). The on-the-wire payload speaks that vocabulary.

## Architecture

```
   ┌─────────────────────────────────┐
   │  Developer machine              │
   │                                 │
   │  Claude Code                    │
   │     │                           │
   │     │ PreToolUse                │
   │     ├──→ arai guardrails ──┐    │
   │     │   (local match)      │    │     ┌─────────────────────────┐
   │     │                      │    │     │  Kete (org-hosted)      │
   │     └──→ HTTPS POST ───────┼────┼────▶│                         │
   │         to Kete            │    │     │  ┌───────────────────┐  │
   │         (event JSON)       │    │     │  │ Policy graph eval │  │
   │                            │    │     │  └───────────────────┘  │
   │   Most-restrictive merge   │    │     │       │                 │
   │   of the two responses     │    │     │       ▼                 │
   │   becomes the actual       │    │     │  ┌───────────────────┐  │
   │   permissionDecision       │    │     │  │ Fleet audit log   │  │
   │                            │    │     │  └───────────────────┘  │
   │                            │    │     │       │                 │
   │                            ▼    │     │       ▼                 │
   │   {hookSpecificOutput: …}       │     │  ┌───────────────────┐  │
   └─────────────────────────────────┘     │  │ Compliance        │  │
                                            │  │ dashboards        │  │
                                            │  └───────────────────┘  │
                                            └─────────────────────────┘
```

Both hooks run **in parallel** (Claude Code's hook system invokes them concurrently). Whichever returns first contributes its decision; the merge waits for both up to a per-event timeout (3s today, configurable). The merge is **most-restrictive wins**:

| Local Arai | Kete | Final |
|---|---|---|
| allow | allow | allow |
| allow | deny | deny |
| deny | allow | deny |
| deny | deny | deny |
| (timeout) | allow | allow |
| (timeout) | deny | deny |
| allow | (timeout) | allow |
| deny | (timeout) | deny |
| (timeout) | (timeout) | allow (fail-open on org gate) |

**Fail-open on Kete timeout** is deliberate: if your org gate is unreachable, the developer should still be able to work (their machine has Arai's local rules as the floor). This is the inverse of Arai's own fail-closed behaviour — Arai's local store is the always-available safety net; Kete is the additional org policy. A regulated org that wants Kete to be fail-closed can run a sidecar mirror.

## On-the-wire contract

### Request — Arai → Kete

Claude Code's HTTP hook handler POSTs the universal hook payload as-is (`hook_event_name`, `tool_name`, `tool_input`, `session_id`, `cwd`, `permission_mode`, etc.) to a URL configured in `settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "",
        "hooks": [
          { "type": "command", "command": "arai guardrails --match-stdin", "timeout": 3 },
          { "type": "http", "url": "https://kete.example.org/v1/hook", "timeout": 3 }
        ]
      }
    ]
  }
}
```

`arai init --kete=https://kete.example.org` adds the second handler to every event in `ARAI_HOOK_REGISTRATIONS`. The Arai CLI manages the registration — operators don't hand-edit settings.json.

The Kete endpoint receives the raw Claude Code hook payload. It MAY consult its own policy graph (which can reference but isn't limited to Arai's triple store), and replies with a Claude Code hook response.

### Response — Kete → Arai (passes through to Claude Code)

Kete returns the standard Claude Code hook response shape. For PreToolUse:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "allow" | "deny" | "ask",
    "permissionDecisionReason": "Org policy: production deployments require ticket number"
  }
}
```

For PostToolUse / UserPromptSubmit / PostToolBatch:

```json
{
  "decision": "block",
  "reason": "Org policy: ...",
  "hookSpecificOutput": { "additionalContext": "..." }
}
```

PermissionDenied:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PermissionDenied",
    "retry": true | false
  }
}
```

Kete is, in this model, *just another hook handler*. No Arai-specific protocol layered on top of HTTP — anything Claude Code's hook spec defines is fair game.

### Authentication

Per-developer **bearer token** in the `Authorization` header, configured via env var:

```bash
export ARAI_KETE_TOKEN="..."
```

`arai init --kete=https://...` writes a placeholder to `.claude/settings.local.json` (gitignored) so the URL is project-bound but the token is per-machine.

Token rotation, scoping, revocation — these are Kete-side concerns. From Arai's perspective the token is opaque.

**TLS pinning:** Kete URLs MUST be HTTPS. Arai's existing `extends::trust` infrastructure (refuse loopback / RFC1918 / link-local / cloud metadata; no redirects) applies. Adding a Kete URL is equivalent to adding it to the `arai trust --add` list.

## What Arai keeps doing locally

Even with Kete configured:

- The local SQLite store + `arai scan` pipeline runs unchanged.
- `arai guardrails --match-stdin` runs alongside the HTTP hook on every event.
- The local audit log, hash chain, and `arai audit --verify` are unaffected.
- `arai audit --purge` operates on local audit only; Kete's fleet log has its own retention controls.
- `ARAI_DISABLED=1` short-circuits the local hook *but does not affect the HTTP hook* — Claude Code invokes them independently. To disable the Kete leg, remove the entry from settings.json (or use Claude Code's own hook-disable controls).

## What Kete owns

- **Rule distribution.** Kete can serve `arai:extends`-compatible policy bundles, with versioning and signing. Today's `arai:extends https://...` works against a static markdown URL; against Kete it becomes a versioned, auditable feed.
- **Fleet rollup.** Every developer's hook event flows to Kete on the HTTP leg. Kete aggregates `obeyed / ignored / unclear` verdicts across the fleet, surfaces per-rule routing-around patterns, and produces dashboards.
- **Org policy that local files can't express.** "No deploy to prod between 5pm Friday and 9am Monday" is a server-side rule, not a markdown line. Kete owns those.
- **Retention policy enforcement.** An org-level "retain 7 years of audit" or "delete this developer's records 30 days after offboarding" decision lives in Kete, applied to *Kete's* fleet log. Per-developer local logs still get whatever `arai audit --purge` policy each machine runs.

## Failure modes

| Failure | Behaviour |
|---|---|
| Kete URL unreachable | HTTP hook times out (3s default). Local Arai's decision stands. Audit-log a `kete_timeout` entry. |
| Kete returns 5xx | Treated as timeout. Same handling. |
| Kete returns malformed JSON | Treated as timeout. Same handling. |
| Kete deny + local allow | Tool call is denied. `arai audit` records both verdicts; the audit-trail shows org policy overrode local. |
| Kete allow + local deny | Tool call is denied. Local Arai's deny stands (most-restrictive). |
| Both timeout | Local Arai already has a fail-closed deny on PreToolUse internal errors; Kete leg fails open. Net: local's deny stands. |
| Network MITM | TLS pinning + Authorization header. Same threat model as `arai:extends` cache signature work in [#104](https://github.com/taniwhaai/arai/pull/104). |
| Token leak | Per-developer tokens limit blast radius. Kete-side revocation is immediate. |

## Migration path

1. **Phase 0 — today.** Arai standalone. Kete charter says it owns fleet concerns but has no transport. **Stale.**
2. **Phase 1 — this doc lands.** Contract agreed, no code yet.
3. **Phase 2 — Kete implements the receiver** (separate work in the Kete repo). Mirror the PreToolUse decision shape.
4. **Phase 3 — Arai implements `arai init --kete=<url>`.** Adds the HTTP hook entries to settings.json alongside the existing local hook. Defaults off — opt-in per project.
5. **Phase 4 — End-to-end.** A regulated team configures Kete with org policy, every developer runs `arai init --kete=...`, fleet rollup starts producing data.

Each phase is independently reviewable. **Nothing in Arai's existing user surface changes until Phase 3.**

## Open questions

1. **Hook-event scope.** Do we send *every* Claude Code hook event to Kete (including FileChanged / InstructionsLoaded / CwdChanged), or only the decision-bearing ones (PreToolUse / PostToolUse / UserPromptSubmit / PermissionDenied)? Sending everything is verbose but gives Kete a complete activity record. Decision-bearing-only is leaner but loses navigation context. **Default proposal: decision-bearing only. Make it configurable.**
2. **Tool-input redaction.** A `tool_input` for `Edit` carries the full edit content, possibly secrets. Today the local audit log truncates to a preview. Should the HTTP hook payload be similarly truncated? Kete's TLS-in-transit may or may not be sufficient for regulated data. **Proposal: add `ARAI_KETE_REDACT=full|preview|none`, default `preview`.**
3. **Async vs sync.** Claude Code's hook system waits for HTTP response before continuing. A slow Kete delays every tool call. Acceptable inside a 1-3s budget; an async-rewake variant where Kete responds out-of-band would let the tool call proceed and surface the verdict on the next loop. Probably out of scope for v1 — sync hook is the documented pattern.
4. **Where does the `arai:extends` upstream live in this model?** If a developer has both `arai:extends https://kete.example.org/policy.md` AND a Kete HTTP-hook URL, are those the same policy or two layers? **Proposal: they're orthogonal. `arai:extends` is markdown rules merged into the local store; HTTP hooks are per-event evaluations against the server's policy graph. A team may use either, both, or neither.**
5. **Schema versioning.** Claude Code's hook payload shape will evolve. Pin a version field in the request? Or rely on field-additive evolution? **Proposal: rely on Claude Code's evolution discipline; revisit if it bites.**

## What this doc explicitly does NOT decide

- The Kete API's internal architecture. That's Kete's design problem.
- Whether Kete is SaaS-only, self-hosted-only, or both.
- Pricing / billing tier shape.
- The auth-token issuance flow.
- The fleet-aggregated dashboard UI.

## Review plan

This is the contract document. Once it has sign-off from Tim (Arai maintainer) and whoever owns Kete, the work splits:

- **Kete side** — implement the receiver. Tracked separately under the Kete roadmap, replacing [kete#1](https://github.com/taniwhaai/kete/issues/1)'s legacy framing.
- **Arai side** — implement `arai init --kete=<url>` + the trust-list integration + the `kete_*` audit event types. Tracked under [arai#110](https://github.com/taniwhaai/arai/issues/110) as the final Tier-3 deliverable.

Either side can begin independently once this contract is approved. No code goes in either repo until then.
