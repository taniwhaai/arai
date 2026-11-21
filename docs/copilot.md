# GitHub Copilot integration

Native Copilot command hooks are registered by `arai init` into
`.github/hooks/arai.json`. The contract below is the published hooks
reference
([hooks reference](https://docs.github.com/en/copilot/reference/hooks-reference),
[cloud agent hooks](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/coding-agent/use-hooks)).
The Copilot CLI is not installed in the environment that produced this
adapter, so none of this was probed against a live Copilot process. The
registered command is replayed locally.

## Setup

Run `arai init` in the project. This writes `.github/hooks/arai.json` with
`"version": 1` when that key is absent, and preserves unrelated handlers.
Re-running init refreshes the recorded binary path. `arai deinit` removes
only Arai's handlers. The file is removed when nothing remains except
`version` and empty hook arrays. A description, a neighbour handler, or any
other key keeps the file.

Registration does not grant workspace trust. Copilot runs repository hooks
only in a trusted workspace. The cloud agent loads `.github/hooks/*.json`
from the default branch, inside a Linux sandbox. It runs the `bash` field
and ignores `powershell`. A managed policy (`allowManagedHooksOnly`,
`strictPluginOnlyCustomization`, or policy hooks) can block repository hooks.

The Copilot CLI also loads `.claude/settings.json`. After `arai init`, one
tool call can be evaluated twice: once by the native file and once by the
Claude registration. The audit log then contains two events.

## What is registered

| Event | Arai behavior |
| --- | --- |
| `preToolUse` | Maps Copilot tool names onto the canonical tools and matches the call. A Block rule writes `{"permissionDecision":"deny","permissionDecisionReason":"..."}` and exits 0. The reason quotes the rule and its source line. Any other result writes nothing and exits 0. |
| `postToolUse` | Records the call for compliance. When the matcher returns rules for this event, advisory text is `additionalContext`. Tool-call rules are timed to `preToolUse`, so this response is empty for those rules. |
| `sessionStart` | Returns a one-line summary of the active domain rules as `additionalContext`, because Copilot drops command output on `userPromptSubmitted`. Then, if an instruction file was added, removed, or modified since the last scan, Arai starts a background scan. |

`userPromptSubmitted` is not registered. Copilot does not use `modifiedPrompt`
or other command output from that event.

`sessionStart` uses `timeoutSec` 10. Tool events use `timeoutSec` 3. Arai
does not set `timeout` or `command` on these handlers.

## Payload and response

A native `preToolUse` payload is camelCase and has no event name:
`sessionId`, `cwd`, `toolName`, `toolArgs`. `toolArgs` may be an object or a
JSON string. Arai reads one JSON value and does not wait for stdin to close.
A Copilot CLI build has left that write-end open; waiting for EOF runs into
the host timeout, and a timeout is fail-open.

Tool names are mapped only for this host: `bash` and `powershell` to Bash,
`view` to Read, `create` to Write, `edit`, `str_replace_editor`, and
`apply_patch` to Edit, `grep` and `rg` to Grep, `glob` to Glob, `task` to
Agent, and the documented web, question, and todo names to their Claude
names. File tools accept `path`, `filePath`, `file_path`, `file`,
`target_file`, or `targetPath`, and `contents`, `content`, `file_text`,
`text`, `new_string`, `new_str`, or `code`. The published CLI `edit` tool
names the previous body `old_str`; that value is copied to `old_string`
when `old_string` is absent. No captured hook payload confirms those two
names. A file tool with none of those path fields
is denied. A patch that arrives only as a diff in `command` has no path, so
it is denied rather than treated as a Codex `apply_patch`.

An explicit `permissionDecision: allow` suppresses Copilot's own approval
prompt, so Arai does not send one. `ask` is not sent either: the cloud agent
and CLI autopilot treat it as deny. A non-blocking match is recorded as
`inject` and is not marked seen. The session-start summary is what reaches
the model.

A handler error on `preToolUse` (malformed JSON, a missing field, a store
error) writes the same deny object and exits 2. Copilot denies a `preToolUse`
command hook on a non-zero exit even when stdout is unusable. A successful
deny stays at exit 0.

## Cloud agent and a missing binary

The handler records the absolute path of the `arai` that ran `arai init`.
That path is not present in the cloud sandbox unless the image has it. The
wrapper therefore checks before it runs:

```sh
if [ -x 'PATH' ]; then 'PATH' guardrails --match-stdin; else exit 0; fi
```

The PowerShell field does the same with `Test-Path` and is what the Windows
CLI runs. If the binary is missing or not executable, the wrapper exits 0
and writes nothing. Copilot then uses its default permission flow. Arai does
not enforce that call. A crash of a binary that does exist is fail-closed on
`preToolUse`, because Copilot treats a non-zero exit as deny. A timeout is
fail-open; that is Copilot's behavior, and Arai does not change it.

Commit `.github/hooks/arai.json` for the cloud agent to see it. Until `arai`
exists at the recorded path, the committed file allows every tool call.
