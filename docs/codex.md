# Codex integration

This integration requires Arai v1.1.2 or newer.

## Setup

Install a v1.1.2+ binary (or `cargo install --path . --locked` from this
repository). In the project you want to protect, run `arai init`. This scans instruction
files and merges Arai's handlers into `.codex/hooks.json`, preserving unrelated
handlers. Re-running init refreshes old Arai executable paths. `arai deinit`
removes the Arai registrations without removing other handlers.

In Codex, open `/hooks`, review the project configuration and enable/trust it.
Codex controls project trust and approval of hook contents; Arai never edits
that trust state. Review again when Codex asks after configuration changes.
Use a Codex version that exposes the documented native hooks interface.

The registration uses the absolute executable path. On Windows,
`commandWindows` explicitly runs PowerShell with an encoded command so spaces,
apostrophes and shell metacharacters in that path remain literal. Stdin and
the hook exit status pass through to Arai. Installing a new Arai binary at a
different path requires another `arai init`.

## What is checked

| Event or operation | Arai behavior |
| --- | --- |
| PreToolUse shell call | Matches Codex's canonical `Bash` command payload; applicable Block rules return `permissionDecision: deny`. |
| `apply_patch` Add File | Checks each file as Write, using both its path and added content. |
| `apply_patch` Update File | Checks the original path and changed content as Edit. |
| `apply_patch` Move to | Also checks the destination as Write and Edit, covering both creation and overwrite policies without relying on a filesystem snapshot. |
| `apply_patch` Delete File | Checks its path as Edit; the current rule schema has no separate Delete action. |
| PostToolUse | Records observations and compliance; accepts Codex's structured `tool_response`. |
| UserPromptSubmit | Injects applicable prompt-time guidance. |

Codex passes the raw patch in `tool_input.command` and keeps `tool_name` as
`apply_patch`. Arai parses all file operations before deciding, combines their
matches, and denies malformed or unsupported patch inputs. This is policy
matching, not a replacement for the host's patch validation or filesystem
permissions. No patch is executed by the hook.

Run `arai scan` after editing instruction files: this integration does not
register instruction-change or working-directory-change events. `arai status`
reports configuration presence and recorded firings, but cannot certify that
Codex has trusted or activated a handler. Use `/hooks` and a harmless synthetic
blocked-rule probe to check host activation.

## Boundaries

The rule engine is based on classified actions and matching terms, not a
general shell interpreter or a filesystem access-control boundary. Deletion
checks use existing Edit policies; delete-specific semantics are not supplied.
Shell commands that indirectly write files remain shell checks.

Codex controls which tools emit hooks. Native hooks do not imply interception
of every hosted tool, continued input through `write_stdin`, or every operation
performed inside a subprocess. Code-mode tool invocations are subject to the
host's documented nested-call hooks. MCP access alone does not automatically
intercept or deny arbitrary tools.

Payload and decision behavior is covered by Arai's tests, including launching
the Windows hook command. Enabling and observing a live Codex session remains
a host-level verification step, not something `arai init` can silently approve.

Protocol reference: [OpenAI Codex hooks](https://learn.chatgpt.com/docs/hooks).

Nested `AGENTS.md` files are discovered with directory scope. Host-specific
`AGENTS.override.md` precedence and configurable fallback filenames are not yet
implemented; see [instruction discovery and upgrade handling](instruction-discovery.md).
