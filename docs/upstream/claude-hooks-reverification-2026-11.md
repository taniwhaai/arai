# Claude Code hook re-verification

9 November 2026 · Windows x64 · Claude Code 2.1.319 from
`@anthropic-ai/claude-code` · Arai 1.1.4 at `2434bdc` and with the launcher fix

## Native results

The installed Claude Code 2.1.112 reported a saved login. A prompt in 2.1.319
using that login returned `OAuth session expired and could not be refreshed`
before requesting any tool. The tool probes used the native CLI with an isolated
profile and a loopback Messages endpoint that supplied two deterministic tool
calls. Claude Code itself loaded the project settings, launched the hooks,
interpreted their output and dispatched the permitted tool. The loopback
endpoint supplied model output only.

Each marker run used a fresh disposable git repository. The first Bash call
was `git status --short && printf DENY > deny.marker` and the second was
`printf ALLOW > allow.marker`. Arai's rule was `- Never run git status`.
Claude ran with `--permission-mode dontAsk --allowedTools Bash` so the
ordinary permission flow would allow either call. `arai status` found one
rule and the generated hook registration, and `arai why "git status --short"`
found the blocking rule at `CLAUDE.md:1`.

| PreToolUse registration | First hook response | Deny marker | Allow marker |
| --- | --- | --- | --- |
| Arai as generated at `2434bdc`, default Git Bash | Exit 0, JSON `permissionDecision: "deny"` | Absent | Present |
| Stock command hook writing a reason to stderr | Exit 2 | Absent | Present |
| Stock command hook failing without `onFailure` | Exit 1 | Present | Present |
| Same stock failure with `onFailure: "block"` | Exit 1, host reported that failure blocks | Absent | Present |
| Original Arai command, explicitly launched by PowerShell | Exit 1, PowerShell parser error | Present | Present |
| Encoded Arai command, default Git Bash | Exit 0, JSON deny | Absent | Present |
| Encoded Arai command, explicitly launched by PowerShell | Exit 0, JSON deny | Absent | Present |

The Arai deny reason appeared in the native tool result and named the rule.
For an unmatched Bash call Arai returned exit 0 with empty stdout and Claude
executed the command. Native `UserPromptSubmit` returned the active rule
summary, `PostToolUse` fired for the permitted call, and `PostToolBatch`
carried both outcomes. A separate native `PowerShell` tool probe under the
default Git Bash hook launcher also blocked `git status --short; Set-Content
deny.marker DENY` and allowed `Set-Content allow.marker ALLOW`. Arai
canonicalized the tool to Bash.

A recorder on the same hook captured `prompt_id` and `effort` alongside the
known PreToolUse keys, `duration_ms` in PostToolUse, and `tool_calls` in
PostToolBatch. The batch's blocked item carried the denial as a
`tool_response` string. Both captured PreToolUse payloads replayed through
`arai test` with the same match counts. Two Bash calls from one model response
were dispatched serially. Two native sessions in one project had overlapping
PreToolUse handlers with the recorder starts 2 ms apart, both denied the
forbidden call, both ran `git --version`, and `arai audit --verify` passed for
the shared project.

Projected audit line from the native marker run, with the session and source
path redacted and the chain fields omitted:

```json
{"decision":"deny","event":"PreToolUse","tool":"Bash","session":"[session]","prompt_preview":"git status --short && printf DENY > deny.marker","rules":[{"subject":"Git","predicate":"never","object":"run git status","severity":"block","source":"[project]\\CLAUDE.md","line":1}]}
```

## Windows launcher

The original shell-form command is `'<absolute-exe>' guardrails --match-stdin`.
PowerShell parsed the quoted path as a string and reported
`Unexpected token 'guardrails' in expression or statement`. The explicit
PowerShell case above used a machine with Git Bash installed. Anthropic also
documents PowerShell as the default hook shell on Windows without Git Bash and
describes different failure behavior on that path. That installation was
outside this probe. [Claude hook command forms and Windows failures](https://code.claude.com/docs/en/hooks#failed-hooks-on-windows-without-git-bash)

The successful variants used the `powershell.exe -NoLogo -NoProfile
-NonInteractive -EncodedCommand` launcher shared with Cursor on Windows.
Both outer shells forwarded stdin and returned the same decision shape and
exit code with empty stderr. The rebuilt `arai init` registration passed
under Git Bash and explicit PowerShell in 2.1.319 and under Git Bash in 2.1.112,
with Arai installed in a path containing spaces, an apostrophe and a dollar
sign. Claude's native `command` plus `args` form passed in 2.1.319. The
2.1.112 probe with that form ran the path as a shell command, returned exit
127 and allowed the forbidden marker.

The rebuilt launcher sets `$ErrorActionPreference = 'Stop'` and forwards the
child's exit code. When pointed at a missing binary it returned exit 1 and
Claude reported a hook error. The default Git Bash permission flow still
allowed the tool call because the handler had no `onFailure` field.

Direct stdin timing measured seven warmed launches per case with the initial
encoded command, including shell startup and Arai evaluation. This is
separate from the native CLI probe.

| Launch path | Allow median | Deny median |
| --- | ---: | ---: |
| Git Bash, original command | 58 ms | 66 ms |
| Git Bash, encoded command | 241 ms | 307 ms |
| PowerShell, encoded command | 466 ms | 458 ms |

The slowest encoded sample was 540 ms. Arai's registered command timeout was
3 seconds.

## Permission boundaries

- Arai emits the Claude JSON deny at exit 0. Exit 2 is the host's separate
  blocking convention and uses stderr for the reason. Arai's existing
  registration did not specify `onFailure`. Anthropic documents that field
  for command and HTTP hooks starting in 2.1.295. [Claude hook exit codes and failure policy](https://code.claude.com/docs/en/hooks#block-the-action-when-a-hook-fails)
- This used `--print`, where Claude Code treats the directory as trusted.
  Interactive Claude checks workspace trust before running settings hooks.
  [Claude workspace trust](https://code.claude.com/docs/en/hooks#workspace-trust)
- A `tool.check` Mod runs after `PreToolUse`. Anthropic documents that a Mod
  can approve a blocked call when the session has neither managed settings
  nor Team or Enterprise login credentials. With either of those in effect
  the hook block holds by default, including a project hook. These probes
  used neither Mods nor managed settings. [Claude permission precedence](https://code.claude.com/docs/en/permissions#precedence-among-rules-hooks-and-mods)
- Claude's documented `SubagentStart` exposes the agent ID and type and can
  inject context. The event does not block creation. `SubagentStop` controls
  completion. Arai does not register either lifecycle event. [Claude subagent hooks](https://code.claude.com/docs/en/hooks#subagentstart)

## Reproduce the marker probe

From a Windows checkout in PowerShell 7, build Arai and install the tested
Claude package in a fresh temporary directory.

```powershell
cargo build --locked
$probe = Join-Path $env:TEMP ("arai-claude-probe-" + [guid]::NewGuid())
New-Item -ItemType Directory "$probe/project" -Force | Out-Null
Copy-Item target/debug/arai.exe "$probe/arai.exe"
npm.cmd install --prefix $probe --no-audit --no-fund --save-exact '@anthropic-ai/claude-code@2.1.319'
Set-Location "$probe/project"
git init -q
Set-Content CLAUDE.md '- Never run git status' -Encoding utf8NoBOM
$env:ARAI_BASE_DIR = "$probe/arai-state"
$env:ARAI_TELEMETRY = 'off'
& "$probe/arai.exe" --version
& "$probe/node_modules/.bin/claude.cmd" --version
& "$probe/arai.exe" init
& "$probe/arai.exe" status
& "$probe/arai.exe" why 'git status --short'
```

Save the following as `$probe/probe.cjs`. It needs Node and Git for Windows.
Adjust the Git Bash path if installed elsewhere.

```javascript
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const {spawn} = require('node:child_process');

const project = path.join(__dirname, 'project');
const claude = path.join(__dirname, 'node_modules', '@anthropic-ai',
  'claude-code', 'bin', 'claude.exe');
let requests = 0;

const server = http.createServer(async (req, res) => {
  let raw = '';
  for await (const chunk of req) raw += chunk;
  const body = JSON.parse(raw || '{}');
  if (!/^\/v1\/messages(?:\?|$)/.test(req.url)) {
    res.writeHead(200, {'content-type': 'application/json'});
    return res.end(JSON.stringify({input_tokens: 12}));
  }
  const first = ++requests === 1;
  res.writeHead(200, {'content-type': 'text/event-stream'});
  const send = (type, data = {}) =>
    res.write('event: ' + type + '\ndata: ' + JSON.stringify({type, ...data}) + '\n\n');

  send('message_start', {message: {
    id: 'msg_probe_' + requests, type: 'message', role: 'assistant',
    model: body.model, content: [], stop_reason: null, stop_sequence: null,
    usage: {input_tokens: 12, output_tokens: 1},
  }});
  if (first) {
    const commands = ['git status --short && printf DENY > deny.marker',
      'printf ALLOW > allow.marker'];
    commands.forEach((command, index) => {
      send('content_block_start', {index, content_block: {
        type: 'tool_use', id: 'toolu_probe_' + index, name: 'Bash', input: {},
      }});
      send('content_block_delta', {index, delta: {
        type: 'input_json_delta', partial_json: JSON.stringify({command, description: 'Probe'}),
      }});
      send('content_block_stop', {index});
    });
  } else {
    send('content_block_start', {index: 0, content_block: {type: 'text', text: ''}});
    send('content_block_delta', {index: 0, delta: {type: 'text_delta', text: 'Probe finished'}});
    send('content_block_stop', {index: 0});
  }
  send('message_delta', {delta: {
    stop_reason: first ? 'tool_use' : 'end_turn', stop_sequence: null,
  }, usage: {output_tokens: 12}});
  send('message_stop');
  res.end();
});

server.listen(0, '127.0.0.1', () => {
  const profile = fs.mkdtempSync(path.join(__dirname, 'claude-profile-'));
  const env = {
    ...process.env,
    ANTHROPIC_API_KEY: 'local-probe-only',
    ANTHROPIC_BASE_URL: 'http://127.0.0.1:' + server.address().port,
    CLAUDE_CONFIG_DIR: profile,
    CLAUDE_CODE_GIT_BASH_PATH: 'C:\\Program Files\\Git\\bin\\bash.exe',
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1',
    DISABLE_AUTOUPDATER: '1',
  };
  for (const name of ['CLAUDE_CODE_OAUTH_TOKEN', 'ANTHROPIC_AUTH_TOKEN',
    'CLAUDE_CODE_USE_BEDROCK', 'CLAUDE_CODE_USE_VERTEX', 'CLAUDE_CODE_USE_FOUNDRY']) {
    delete env[name];
  }
  const args = ['--print', '--model', 'haiku', '--effort', 'low',
    '--setting-sources', 'project', '--strict-mcp-config', '--no-session-persistence',
    '--tools', 'Bash', '--allowedTools', 'Bash', '--permission-mode', 'dontAsk',
    '--max-turns', '3', '--output-format', 'stream-json', '--verbose',
    '--include-hook-events', 'Hook probe'];
  const child = spawn(claude, args, {cwd: project, env, stdio: ['ignore', 'inherit', 'inherit']});
  const timer = setTimeout(() => child.kill(), 30000);
  child.on('close', code => {
    clearTimeout(timer);
    server.close();
    process.exitCode = code ?? 1;
  });
});
```

Run it and inspect the native results and local audit.

```powershell
node "$probe/probe.cjs" > "$probe/host.jsonl"
Test-Path allow.marker
Test-Path deny.marker
Get-Content "$probe/host.jsonl" | ForEach-Object { $_ | ConvertFrom-Json } |
  Where-Object { $_.subtype -eq 'hook_response' -or $_.type -eq 'user' } |
  ConvertTo-Json -Depth 10
& "$probe/arai.exe" audit --json --event=PreToolUse --limit=10
& "$probe/arai.exe" audit --verify
```

To reproduce the Windows parser failure in a fresh fixture, set
`hooks.PreToolUse[0].hooks[0].shell` to `"powershell"` and use the original
shell-form command from the table. To test the encoded variant, copy
`hooks.preToolUse[0].command` from the generated `.cursor/hooks.json` over the
Claude PreToolUse command. Leave `shell` unset for the Git Bash test. Use
fresh markers for each run.

For the standalone exit-code comparison, save `$probe/stock.cjs` as follows.

```javascript
const fs = require('node:fs');
const input = JSON.parse(fs.readFileSync(0, 'utf8'));
if (input.tool_input.command.startsWith('git status')) {
  process.stderr.write('stock probe exit ' + process.argv[2] + '\n');
  process.exitCode = Number(process.argv[2]);
}
```

In a fresh fixture replace the Claude PreToolUse handler with the following.
Change `'2'` to `'1'` for the failure runs and add `onFailure = 'block'` for
the fail-closed case.

```powershell
$settings = Get-Content .claude/settings.json | ConvertFrom-Json -AsHashtable
$settings.hooks.PreToolUse[0].hooks[0] = @{
    type = 'command'
    command = (Get-Command node.exe).Source
    args = @("$probe/stock.cjs", '2')
}
$settings | ConvertTo-Json -Depth 12 |
    Set-Content .claude/settings.json -Encoding utf8NoBOM
node "$probe/probe.cjs" > "$probe/stock.jsonl"
```
