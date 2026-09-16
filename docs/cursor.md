# Cursor native hook pilot

This opt-in adapter is **Unreleased**. Build this revision with
`cargo install --path . --locked`, then run:

```sh
cd your-project
arai init --platform cursor
```

This scans local instructions and merges Arai-owned `preToolUse` and
`postToolUse` commands into `.cursor/hooks.json` (version 1). Commands pin the
platform and event, use the installed executable's absolute path, and have
a three-second timeout. Pre-tool registration sets `failClosed: true`.
Windows commands explicitly invoke PowerShell with an encoded command;
POSIX commands quote the executable path. Re-run init if the binary moves.

Cursor's [native hook contract](https://cursor.com/docs/hooks) was reviewed
on 2026-09-16. The generic file-mutation payload schema is not fully specified
there. This adapter therefore remains a pilot until payloads and blocking
have been verified in a real Cursor session. Unit/subprocess tests establish
the Arai contract, not host activation. No Cursor version has yet been
live-verified for this adapter on desktop, CLI or cloud.
See the [executable validation record](upstream/cursor-adapter-validation-2026-09-16.md).

## Coverage

| Surface | Behavior |
| --- | --- |
| Shell | Maps `command` and `working_directory` to Bash matching; conflicting working-directory aliases fail closed. |
| Write | Accepts an explicit `file_path` with string `content`, paired `old_string`/`new_string`, or an array of string replacement pairs. Checks both Write and Edit policy because Cursor combines those categories. Unknown mutation fields/shapes fail closed. These shapes are fixture contracts, not captured host-version guarantees. |
| Delete | Accepts an explicit `file_path` and checks path-based Edit policy; no deletion-specific rule semantics. |
| Read / Task | Uses Arai's existing skipped-tool behavior. This does not establish interception inside a subagent. |
| Grep / `MCP:<name>` | Uses existing generic matching. MCP tool contents do not automatically become shell or file operations. |
| Other tool categories | Fail closed until their semantics have an adapter and fixtures. |
| Pre-tool Block | Returns native deny JSON plus exit 2. |
| Pre-tool allow | Always emits `permission: allow`, including no matches/no DB/skipped/disabled. Warn/inform matches are audited as allowed; no advisory text or token-saving credit is claimed. |
| Post-tool | Parses JSON-stringified `tool_output`, records session observations, and can return `additional_context`. Never blocks an action already executed. |
| Prompt guidance | Not registered: `beforeSubmitPrompt` does not document prompt context injection. |
| Compliance attribution | Not emitted for Cursor; the existing session/tool heuristic cannot establish exact correlation or compliance with advice that was never delivered. |

Conservative Write checking can block a modification under a creation-only
rule. That is intentional while the host category is ambiguous; there is no
filesystem snapshot or inferred operation type. Binary-specific encodings and
other mutation shapes are not silently accepted. Shell matching remains lexical policy matching,
not a shell interpreter or filesystem security boundary. Run `arai scan`
after changing instruction files; the pilot does not register refresh hooks.

## Native and compatibility hooks

Cursor can also import Claude hooks and runs matching native and imported
handlers. When using native Arai, review Cursor's third-party hook settings
and choose one Arai integration. Existing project, local and global Claude
registrations may otherwise produce duplicate firings. Arai warns but does
not alter editor preferences or global registrations, and does not suppress
enforcement on the assumption that a second configured hook actually ran.
See [Cursor's compatibility reference](https://cursor.com/docs/reference/third-party-hooks).

Repeated native init replaces Arai's owned handlers without adding duplicates
or removing other handlers. `arai deinit --platform cursor` removes only the
native Cursor integration. See [selection persistence](platform-adapters.md).

## Host verification before promotion

In a disposable trusted test workspace, record Cursor's exact version,
desktop/CLI environment, OS, Arai revision, enabled hook paths and payloads
with sensitive content removed. Install a harmless rule such as
`Never run cargo clean`. Ask the host to run `cargo check` (allow) and
`cargo clean` (deny), checking its hook log and filesystem evidence rather
than only the model's narration. Verify the denial occurs before execution.
Use a disposable negative control without Arai to prove the host would
otherwise attempt the command. Then exercise a file creation, modification,
deletion, scoped instruction, malformed response and timeout, and capture
their actual schemas before declaring those operations supported.

Check `arai audit` and the host log for duplicates. `arai status` cannot
certify that Cursor enabled the handler. Do not commit generated absolute
paths from one machine as portable cloud configuration. Cursor documents
that early read-only cloud turns can run without hooks; this pilot makes
no complete cloud-coverage claim.
