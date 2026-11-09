# Cursor hook re-verification

9 November 2026 · Windows x64 · Cursor Agent CLI `2026.11.05-86047df` · Arai
1.1.4 development build with the stdout hold and Windows launcher changes.

## Cursor CLI

The Windows package from the official installer was extracted into a
temporary directory. `cursor-agent.cmd status` reported `Not logged in`.
The headless probe stopped before making tool calls and asked for a Cursor
login or `CURSOR_API_KEY`.

```powershell
& $cursorAgent --version
& $cursorAgent status
& $cursorAgent -p --trust --output-format text 'Use Shell to run cargo clean --help, and in parallel use Write to create notes.txt containing probe. Do not retry any blocked call.'
```

Host-side stdout capture and deny message delivery during concurrent calls
still need an authenticated Cursor session. Cursor staff describes the
stdout race and recommends holding the process open briefly after writing.
[Cursor staff response](https://forum.cursor.com/t/pretooluse-hook-exit-0-but-empty-stdout-under-concurrent-tool-calls-failclosed-blocks/174214/6)

## Registered command and replay

The integration test initializes a disposable repository and launches its
actual `.cursor/hooks.json` command. On Windows this goes through the encoded
PowerShell launcher. The test starts both child processes before sending the
Shell and Write payloads from the same conversation and generation.

```powershell
cargo test --locked --test hook_registration cursor_registered_command_answers_overlapping_pretooluse_calls -- --exact
```

For the scenario replay, run these commands from a checkout with a built Arai
binary.

```powershell
$arai = (Resolve-Path target/debug/arai.exe).Path
$fixture = (Resolve-Path tests/fixtures/cursor-pre-tool-use.json).Path
$probe = Join-Path $env:TEMP ('arai-cursor-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path "$probe/project" | Out-Null
$env:ARAI_BASE_DIR = "$probe/state"
$env:ARAI_TELEMETRY = 'off'
Set-Location "$probe/project"
git init -q
& $arai init
& $arai add 'Never run cargo clean'
& $arai status
& $arai why 'cargo clean'
& $arai test $fixture
```

Both scenario expectations passed. Over 20 pairs of overlapping invocations
of the registered command, all 40 processes exited 0 and returned complete
JSON. Each Shell response had `permission: "deny"` and the Cargo rule in
`user_message`. Each Write response was `{"permission":"allow"}`.
Measured process lifetime included PowerShell startup and the 150 ms hold.
The average was 419 ms and the slowest was 587 ms against the registered
3-second timeout.

`arai audit --event PreToolUse --limit 1 --json` yielded the following
projected entry. `arai audit --verify` reported a clean chain.

```json
{"event":"PreToolUse","tool":"Bash","decision":"deny","prompt_preview":"cargo clean"}
```
