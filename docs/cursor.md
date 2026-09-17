# Cursor integration

Native Cursor Agent hooks require an Arai build newer than v1.1.2 (the first
release that includes this adapter is listed in the CHANGELOG). Older Arai
versions reach Cursor only through the MCP server (see [mcp.md](mcp.md)) and
the `arai check-diff` pre-commit gate.

## Setup

Run `arai init` in the project. This merges Arai's handlers into
`.cursor/hooks.json` (creating it with `"version": 1` when absent) and
preserves unrelated handlers. Re-running init refreshes old executable paths;
`arai deinit` removes only Arai's handlers and deletes the file when nothing
else is left in it.

Cursor loads project hooks from `<project>/.cursor/hooks.json`. Enterprise and
team hooks take precedence over project hooks; a managed policy can restrict
which hooks run. Check the Hooks tab in Cursor settings to confirm the project
hooks are loaded.

## What is registered

| Event | Registration | Arai behavior |
| --- | --- | --- |
| `preToolUse` | `failClosed: true` | Matches `Shell` as Bash, `Write`/`Delete`/`Edit` as file actions, MCP tools by name. Applicable Block rules return `{"permission": "deny"}` with the reason in `user_message` and `agent_message`. Every other call is answered with an explicit `{"permission": "allow"}`. |
| `postToolUse` | matcher `Shell` | Records the shell call for prerequisite tracking and compliance; `tool_output` content feeds observation. Advisory context returns as `additional_context`. |
| `afterFileEdit` | | Treats the edit as a PostToolUse Edit on `file_path`, joining `edits[]` old and new strings for content matching and compliance. |
| `sessionStart` | | Returns a one-line summary of the active domain rules as `additional_context`, then spawns a background `arai scan` if an instruction file was added, removed or modified since the last scan. |

`beforeShellExecution`, `beforeMCPExecution` and `beforeReadFile` are
understood if registered by hand (they become PreToolUse with a synthesised
tool), but `arai init` does not register them: `preToolUse` already covers
shell, MCP and file tools, and registering both would evaluate and audit each
shell call twice.

## Fail-closed posture

Cursor hooks are fail-open by default: a crash, timeout or non-zero exit other
than 2 lets the action through. Arai registers `preToolUse` with
`failClosed: true` so those failures block instead. Under that flag Cursor also
treats empty stdout as a failure, which is why the handler always writes an
explicit allow on PreToolUse, including the skip-tool and no-rules fast paths.
Do not remove `failClosed` from Arai's handler; without it a broken Arai
binary silently stops enforcing.

Host identity comes from the payload, never from the environment. Cursor
exports `CLAUDE_PROJECT_DIR` as a compatibility alias, and a Claude Code,
Codex or Grok session started from Cursor's integrated terminal inherits
`CURSOR_VERSION`, so those variables cannot tell the hosts apart. Arai
recognises Cursor by the `cursor_version` field every Cursor hook carries, or
by a Cursor-spelled event name, and answers every other payload in that
host's own shape. If Cursor ever sends a payload Arai cannot parse at all,
the deny is emitted in Claude's shape; Cursor treats a response outside its
schema as a block, so that path still fails closed.

Cursor's translation runs inside the shared matcher, so `arai test` replays a
recorded Cursor payload exactly as the live hook evaluates it.

## Boundaries

- The advisory path is narrower than on Claude Code. Cursor delivers
  `agent_message` only on deny, and `beforeSubmitPrompt` has no model-visible
  output, so Warn/Inform rules reach the model through the `sessionStart`
  summary and `postToolUse` context, not at the moment of the call. An
  advisory `preToolUse` match is still recorded in the audit log as
  `inject`, but Arai does not mark those rules as seen, so the post-tool
  context carries them in full. Read per-rule compliance ratios for Warn
  rules on Cursor with that in mind. Treat block as the guarantee.
- Cursor documents `tool_input` for `Shell` (`command`, `working_directory`)
  but not for `Write` or `Delete`. Arai accepts `file_path`, `path`,
  `filePath`, `file`, `target_file` or `relative_workspace_path` for the
  path and `content`, `contents`, `text` or `code_edit` for the body. A
  `Write` or `Delete` whose path is under none of those names is denied
  fail-closed with a `user_message` naming the missing field; if that
  happens in your Cursor build, report the payload shape so the alias list
  can be extended. `afterFileEdit` uses documented fields and is the
  reliable post-edit signal.
- `Delete` is evaluated as an Edit on its path; the rule schema has no
  delete-specific action.
- Cloud agents run command hooks only and skip `sessionStart`, so the
  session-start rescan and summary do not happen there. Project hooks still
  fire for `preToolUse`, `postToolUse` and `afterFileEdit`.
- On Windows the registered `command` is an explicit `powershell.exe
  -EncodedCommand` invocation of the binary path, because Cursor has no
  per-platform command field and does not document which shell runs the
  string; that form parses the same under cmd.exe and PowerShell. Cursor on
  Windows has not been probed end to end. Re-run `arai init` after moving
  the binary on any platform.

Protocol reference: [Cursor Agent hooks](https://cursor.com/docs/agent/hooks).
