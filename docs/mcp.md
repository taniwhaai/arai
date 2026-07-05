# MCP integration

The stdio MCP server: agent-authored guards, decision self-checks, and authentication.

## MCP: agent-authored guardrails

`arai mcp` is also the integration path for assistants that don't have a
native PreToolUse hook surface. Cursor and Windsurf are both MCP clients — point
them at `arai mcp` and the agent can read the same rule set, register new guards
mid-session, and self-check recent decisions.
The strongest blocking enforcement is available in assistants with native hook
support (currently Claude Code and Grok TUI), but everything else — rule lookup,
agent-authored guards, decision history — is shared via MCP.

`arai mcp` runs a [Model Context Protocol](https://modelcontextprotocol.io/)
server on stdio. Three tools, exposed to any MCP-capable agent:

| Tool | What it does |
|------|--------------|
| `arai_add_guard(rule, reason?)` | Register a new guardrail mid-session. Takes effect on the next PreToolUse hook — same enforcement path as rules in your CLAUDE.md. |
| `arai_list_guards(pattern?)` | List active guardrails, optionally substring-filtered, so the agent can check what constraints are live before acting. |
| `arai_recent_decisions(session_id?, limit?, since?)` | Look up recent Ārai decisions (deny / inject / review) so the agent can self-check after a refusal — closes the model-side feedback loop. |

This closes two gaps instruction files don't cover. First, when an agent
discovers a rule mid-session (*"from now on, never write to /etc"*,
*"always run the full test suite before pushing"*), it now has
somewhere to register it for deterministic enforcement rather than
hoping context retention holds. Second, after a deny, the agent can
call `arai_recent_decisions` to see what it was just refused for —
useful for avoiding "try the same thing twice" loops when a single
rule keeps getting hit.

Register it with your assistant (for example in Claude Code or Cline) by adding to your MCP settings:

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

For Cline (in `cline_mcp_settings.json`, or via the MCP UI):

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

For Cursor and Windsurf, follow each tool's MCP server registration UI
and point it at the same `arai mcp` command — the protocol is identical.

Prerequisite: `arai` must be on your `PATH`. The install script, `cargo
install arai`, `npm install -g @taniwhaai/arai`, and the Homebrew tap all
put it there.
