# Claude Code integration

Run `arai init` to register Arai's command hooks in `.claude/settings.json`.
Trust the project in Claude Code, then try a harmless action covered by a Block
rule. The tool call should be blocked and `arai audit` should show the decision.

On Windows the registered hook starts PowerShell with an encoded command.
Git Bash and PowerShell can both invoke it, and the executable path remains
literal even when it contains spaces or shell metacharacters. Re-run `arai
init` to refresh an existing registration.

## Claude Code Mods

Mods run inside Claude Code. An installed Mod that handles `tool.check` can
approve a call after a `PreToolUse` hook denies it when the session has neither
managed settings nor a Team or Enterprise `/login` credential. Under either
managed settings or that login, Claude Code preserves the hook's deny even
when the hook comes from project settings. An API key or long-lived
`CLAUDE_CODE_OAUTH_TOKEN` does not count as a Team or Enterprise `/login`.
[Claude Code permission precedence](https://code.claude.com/docs/en/permissions#precedence-among-rules-hooks-and-mods)

Claude Code enables Mods by default starting with 2.1.286. Administrators can
set `allowManagedModsOnly` in managed settings to limit which Mods load.
[Claude Code managed Mods](https://code.claude.com/docs/en/plugins/mods/admin)

## Hook failures

Claude Code 2.1.295 and later support `onFailure: "block"` for command hooks.
That option controls failures such as a missing binary or a timeout. Arai's
registered Claude hook uses the host's default failure policy and returns a
JSON deny when it evaluates a Block rule.
[Claude Code hook failure behavior](https://code.claude.com/docs/en/hooks#block-the-action-when-a-hook-fails)
