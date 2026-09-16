# Early availability and host activation

These changes are in PR183, not the published v1.1.2. Build the branch or use
the release that includes it. The same local policy engine serves all hosts;
adapters handle each host's actual event and response contracts.

## Per-project setup

Install Arai in the environment where the host runs, then from that project:

```sh
arai init --platform claude --platform grok --platform codex
arai status
```

Select only hosts you use. Cursor remains a separate opt-in pilot:
`arai init --platform cursor`. Explicit selection is remembered; it does not
remove previously installed other-host hooks. Use `arai deinit --platform ...`
when intentionally removing one.

Init scans policy and refreshes exact Arai-owned commands. Each command pins
its platform and event and uses an absolute executable path. Windows commands
launch PowerShell with no profile; they do not depend on a TUI or desktop
shell profile. Re-run init after moving/reinstalling the binary.

Windows, WSL, a remote workspace and a cloud workspace are separate execution
environments. Install and initialize inside the environment that runs hooks;
a Windows path is not a portable WSL/cloud registration. Do not commit a
machine-specific absolute path as a cross-machine installation mechanism.

## What the hosts receive

| Host | SessionStart | SubagentStart | Advisory tool guidance |
| --- | --- | --- | --- |
| Claude Code CLI / Desktop Code | User status plus brief model context | Model context | Before tool execution; does not approve permission |
| Codex CLI / desktop app hook runtime | User status plus brief model context | Model context | Before tool execution; does not approve permission |
| Grok Build | User status through systemMessage in current public source | Invocation receipt only | PreToolUse context arrives after execution |
| Cursor pilot | Not registered | Not registered | Pre-tool allow cannot inject advice; post context only |

SessionStart matches all sources, including startup/resume/clear/compact and
any host-supported fork source. There is no duplicate PostCompact injection.
Startup reads the existing store in read-only mode: stored rule count, last
scan age and mode. It explicitly reports missing/unreadable policy, disabled
or advisory mode, and unverified disk freshness. It does not discover files,
resolve remote extends, load models, scan code or flush telemetry. A missing
store is not automatically created. No startup summary marks rules as seen.

This is an availability check, not proof of enforcement. Arai cannot announce
itself through hooks that the host has disabled or has not trusted.

## Check in the host

1. **Claude Code:** inspect `/hooks` and `/status`, including configuration
   sources, workspace trust and managed hook restrictions. Desktop's Code
   environment shares Claude Code configuration; consumer chat is not covered.
2. **Codex:** trust the project config layer and review each new or changed
   definition through `/hooks`. Changes to commands require renewed review.
   Features or managed requirements may disable non-managed hooks. Installing
   a plugin or creating hooks.json does not grant trust.
3. **Grok Build:** inspect `/hooks` and `/hooks-trust`; review native
   .grok/hooks/arai.json entries. Grok can also import Claude and Cursor hooks.
   Choose one Arai delivery route in its hook controls to avoid duplicate
   calls. Arai warns about compatibility sources but does not change them or
   infer activation from their presence.
4. **Cursor:** follow the [pilot verification procedure](cursor.md), including
   compatibility-import duplication checks.

Then exercise a harmless known block and an unrelated allow in an isolated
test project. Inspect the host's hook log and `arai status` / `arai audit`.
Do not use a destructive command as a blocking test.

Status reports exact owned registrations, missing events, absolute executable
file existence, unresolved PATH-dependent commands, and recent per-event
adapter receipts. Receipts identify the selected adapter, not an authenticated
host: compatibility imports and manual invocations can produce them. They
contain no prompts, tool inputs, credentials or rule content. A SessionStart
receipt does not prove PreToolUse was trusted, and an old pre-tool receipt
does not prove the current session/configuration is active.

## New verbs and intentionally unused events

Claude PowerShell and Monitor with a command share Bash policy; Monitor with a
WebSocket source remains distinct. Grok search_replace with empty old_string
checks creation and modification policy. Ordinary nonempty replacements use
Edit. Native grep and spawn_subagent aliases are recognized.

Claude FileChanged uses literal filenames to seed watchers; dynamic watchPaths
is not used because it replaces the host's dynamic list. This covers common
root instruction files, not recursive rule directories. Existing
InstructionsLoaded/CwdChanged refresh remains asynchronous and may use the
normal scan's remote-extends or cached-model behavior. Run `arai scan` for an
explicit refresh; startup does not silently broaden policy acceptance.

PostToolBatch is a summary only because individual PostToolUse events also
fire. PermissionDenied records reason (with legacy field fallback) without
requesting retries. PermissionRequest, Stop, Interrupt, task/team, elicitation
and other known passive events are not registered just because they exist.
They cannot fall through into a PreToolUse allow/deny response.

## Contract and validation boundaries

Reviewed 2026-09-16 against [Claude hooks](https://code.claude.com/docs/en/hooks),
[Claude tools](https://code.claude.com/docs/en/tools-reference),
[Claude Desktop shared configuration](https://code.claude.com/docs/en/desktop#shared-configuration),
[Codex hooks](https://learn.chatgpt.com/docs/hooks), and
[Grok public hook contracts](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-pager/docs/user-guide/10-hooks.md).
Grok systemMessage forwarding is checked in its
[dispatcher](https://github.com/xai-org/grok-build/blob/482711333c7195dc16a272777f86086d615e2afb/crates/codegen/xai-grok-shell/src/session/acp_session_impl/hook_dispatch.rs).

Installed binaries during review: Claude Code 2.1.112, Codex CLI 0.153.4 and
Grok 1.0.13 (5e9a58528b76). These version checks are not end-to-end host
verification; newer documentation includes features absent from older clients.
In particular, Claude's WebSocket Monitor requires 2.1.195+. The Grok public
commit is not an asserted match for the installed release.

Executable fixtures and actual generated Windows command launches validate
protocol behavior. Fresh authenticated desktop/TUI sessions, managed
environments, remote/cloud modes and Cursor promotion still require their
own version-specific evidence. See the [validation record](upstream/host-lifecycle-validation-2026-09-16.md).
Kete's offline matching/provenance and activation responsibilities remain
unchanged; invocation receipts do not become upstream attestations.
