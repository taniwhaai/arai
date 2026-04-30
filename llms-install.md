# Installing Arai's MCP server

This document is written for an LLM-driven installer (e.g. Cline). Follow
the steps in order. Each step is verifiable; do not proceed past a failed
verification.

## What this server does

`arai mcp` is a stdio MCP server exposing three tools:

- `arai_add_guard(rule, reason?)` — register a new guardrail rule.
- `arai_list_guards(pattern?)` — list active guardrails, optionally filtered.
- `arai_recent_decisions(session_id?, limit?, since?)` — read recent Arai
  decisions (deny/inject/review) from the local audit log.

It is local-only. No network calls. No credentials required. State lives
under `~/.arai/`.

## Step 1 — Install the binary

Pick the first option that succeeds in the user's environment.

### Option A — Cargo (works on any machine with Rust toolchain)

```bash
cargo install arai
```

### Option B — npm

```bash
npm install -g @taniwhaai/arai
```

### Option C — Homebrew (macOS / Linux)

```bash
brew install taniwhaai/tap/arai
```

### Option D — Install script (macOS / Linux)

```bash
curl -sSf https://arai.taniwha.ai/install | sh
```

## Step 2 — Verify the binary is on PATH

```bash
arai --version
```

Expected: a version string like `arai 0.2.9`. If the command is not
found, the user's shell may need to reload `PATH` — instruct them to
open a new terminal.

## Step 3 — Verify the MCP server starts

```bash
arai mcp
```

Expected: the process blocks waiting for stdin. There should be no error
output. Press `Ctrl+C` to exit. If it crashes immediately, capture the
stderr and report it to the user — do not proceed.

## Step 4 — Register the server with the host

For Cline, add this block to `cline_mcp_settings.json` (or use the MCP
UI):

```json
{
  "mcpServers": {
    "arai": {
      "command": "arai",
      "args": ["mcp"],
      "disabled": false,
      "autoApprove": []
    }
  }
}
```

For other MCP-capable clients, the equivalent shape is:

```json
{
  "mcpServers": {
    "arai": {
      "command": "arai",
      "args": ["mcp"]
    }
  }
}
```

## Step 5 — Confirm tools are exposed

After the host reloads the MCP configuration, ask it to list the tools
exposed by the `arai` server. Expected names:

- `arai_add_guard`
- `arai_list_guards`
- `arai_recent_decisions`

If those three names appear, the server is healthy and the install is
complete.

## Step 6 — Optional smoke test

Call `arai_add_guard` with:

```json
{ "rule": "Never force-push to main", "reason": "smoke test" }
```

Expected: a success response. Then call `arai_list_guards` with no
arguments and confirm the new rule appears in the result.

If `arai_add_guard` returns an error like `could not extract a guardrail
from: ...`, the rule string is not a recognized imperative. Use the
shape `Never <verb> <object>` or `Always <verb> <object>` — these parse
reliably.

## Troubleshooting

- **`arai: command not found`** — install path is not on `PATH`. With
  Cargo, ensure `~/.cargo/bin` is on `PATH`. With npm, ensure the global
  bin directory is on `PATH` (`npm bin -g` shows it).
- **MCP server starts but no tools appear in the host** — host has not
  reloaded its MCP configuration. Restart the host or use its
  "reconnect MCP servers" command.
- **Tool call fails with `MCP error -32000`** — payload didn't parse.
  Check argument names and shapes against the schemas the server
  advertises.

## Reference

Source: [github.com/taniwhaai/arai](https://github.com/taniwhaai/arai)
Issues: [github.com/taniwhaai/arai/issues](https://github.com/taniwhaai/arai/issues)
