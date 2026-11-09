# Codex native hooks re-verification, November 2026

Verified on 9 November 2026 with Codex CLI **0.166.0-alpha.5.1**, Arai
**1.1.4** and Windows 11 build **26200**. Codex ran with the MXC sandbox and a
workspace-write permission profile in a disposable Git repository. A loopback
Responses server supplied deterministic tool calls. The installed Codex CLI
dispatched the tools and ran the registered hooks. A companion capture hook
recorded the incoming envelopes without making decisions.

## Setup

From the Arai checkout, use a fresh probe directory and a separate Codex home.

~~~powershell
$arai = (Resolve-Path target/debug/arai.exe).Path
$probe = Join-Path $env:TEMP ("arai-codex-" + [guid]::NewGuid().ToString("N"))
$project = Join-Path $probe project
$env:CODEX_HOME = Join-Path $probe codex
$env:ARAI_BASE_DIR = Join-Path $probe state
$env:ARAI_TELEMETRY = "off"
New-Item -ItemType Directory -Path $project,$env:CODEX_HOME | Out-Null
Set-Location $project
git init -q
@"
## Probe

- Never run cargo clean

## Alembic

- Never hand-write migration files
"@ | Set-Content AGENTS.md -Encoding utf8
& $arai init
& $arai status
& $arai why 'Write-Output "cargo clean"'
& $arai why --tool Write 'alembic/blocked.py'
~~~

Arai reported two active rules and both `why` checks found a block. The
probe used the following configuration in the isolated Codex home.

~~~toml
model = "gpt-6.1-sol"
model_provider = "arai-probe"
approval_policy = "never"
sandbox_mode = "workspace-write"

[model_providers.arai-probe]
name = "Arai local probe"
base_url = "http://127.0.0.1:48831/v1"
env_key = "ARAI_CODEX_PROBE_DUMMY"
wire_api = "responses"
~~~

The local stdio MCP server, `mcp.cjs`, advertised a single `echo` tool
with `readOnlyHint: true` that returns its `text` argument as a text block.
A separate capture handler in each event group appended the incoming event
name, tool name, input and response to a local log.

## Trust review

~~~powershell
$env:ARAI_CODEX_PROBE_DUMMY = "local-probe"
codex mcp add arai_probe -- node (Join-Path $probe mcp.cjs)
codex --no-daemon --no-alt-screen -C $project
~~~

On startup Codex asked to trust the folder, then separately reported eight hook
definitions pending review, four Arai handlers and four capture handlers. The
hook browser showed zero active handlers until trust was granted through the
review screen. Reopening `/hooks` showed both handlers active for each of
`PreToolUse`, `PostToolUse`, `UserPromptSubmit` and `SessionStart`. The
subsequent headless run used the persisted reviews from that isolated Codex
home.

## Native tool calls

~~~powershell
codex exec --json --ephemeral --disable incremental_tools --disable plugins `
  -c windows.sandbox=mxc -C $project 'Run the deterministic Arai hook probe'
~~~

The local provider returned one tool call per response in this order, then a
final assistant message.

| Tool call and input | Observed result |
| --- | --- |
| `exec_command` with `cmd` set to `Set-Content -LiteralPath allowed-marker.txt -Value allowed` | Codex reported exit 0 and the file contained `allowed`. `PreToolUse` and `PostToolUse` both reported `Bash` and put the command in `tool_input.command`. A PowerShell provider warning appeared as the PostToolUse response string. |
| `exec_command` with `cmd` set to `Set-Content -LiteralPath denied-marker.txt -Value "cargo clean"` | Codex reported `Command blocked by PreToolUse hook` with Arai's Cargo rule. The marker was absent. The quoted text exercised the rule without running Cargo. |
| Custom `apply_patch` with the patch below | Codex delivered the full patch in `tool_input.command` and reported Arai's Alembic deny reason. Both destination files were absent. The audit preview lists both Write operations. |
| `echo` in MCP namespace `mcp__arai_probe` with `{"text":"local-probe"}` | Both hooks reported `mcp__arai_probe__echo` with the JSON object in `tool_input`. The tool returned `local-probe`. The post hook carried structured `tool_response.content`. |

~~~diff
*** Begin Patch
*** Add File: harmless.txt
+allowed
*** Add File: alembic/blocked.py
+from alembic import op
*** End Patch
~~~

The capture hook also saw `SessionStart` with `source: "startup"` and
`UserPromptSubmit` with the probe text. The local provider received Arai's
two-rule summary as developer context on the first request.

## Audit evidence

`arai audit --json --event=PreToolUse --limit=2` returned these two decisions
(fields projected to omit local paths and session identifiers).

~~~jsonl
{"event":"PreToolUse","tool":"Edit","decision":"deny","prompt_preview":"Write harmless.txt; Write alembic/blocked.py"}
{"event":"PreToolUse","tool":"Bash","decision":"deny","prompt_preview":"Set-Content -LiteralPath denied-marker.txt -Value \"cargo clean\""}
~~~

`arai audit --verify` reported a clean chain. The recorded shell, patch and MCP
envelopes fit the existing adapter. The echo checks host delivery of MCP hook
events. Rule coverage for an individual MCP operation depends on the rules
that apply to its tool and arguments.

The test covered CLI startup review and `/hooks`. Desktop review,
managed configuration and subagent execution were outside the probe.
OpenAI documents `SubagentStart` and `SubagentStop` separately, and lists
`Agent` as a matcher alias for `spawn_agent` on the ordinary tool hook path.
[OpenAI Codex hooks](https://learn.chatgpt.com/docs/hooks)
