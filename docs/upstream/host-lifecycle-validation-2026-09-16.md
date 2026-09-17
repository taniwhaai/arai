# Existing-host lifecycle validation — 2026-09-16

This record covers Arai executable protocols and generated command launches.
It does not certify fresh authenticated Claude/Grok/Codex desktop/TUI sessions.
Installed versions and primary contract sources are in [host activation](../host-activation.md).
Cursor remains an opt-in pilot with its [separate verification boundary](cursor-adapter-validation-2026-09-16.md).

| Check | Result |
| --- | --- |
| Full default suite | Passed on Windows and Ubuntu WSL |
| Full enrichment suite | Passed on Windows; hosted Linux enrichment CI verifies the Linux feature build |
| Lean library suite | 491 passed on Windows |
| Host lifecycle executable contracts | 6 passed on each OS |
| Host verb CLI and public matcher regressions | 6 passed on each OS |
| Shared hook safety, including final legacy alias fix | 10 passed on each OS |
| Existing Cursor executable and Kete embedding suites | 13 and 5 passed on each OS |
| Registration integration suites | Passed on both OS; Windows launches all three generated commands from a path containing spaces, apostrophe and dollar sign |
| Default/all-target and lean Clippy | Passed on both OS |
| Rustfmt and diff whitespace checks | Passed |
| New live-host trust/startup/tool-gate sequence | Not exercised; requires version/environment-specific verification |

Startup module tests cover missing/corrupt/legacy read-only stores, no policy
initialization, disabled/advisory modes, bounded receipts and interrupted/busy
writes. Integration tests cover startup/subagent output, passive-event
isolation, real block rules, permission-neutral advice, modern/legacy batch
summaries, native denial reasons and deferred Grok accounting, including
Claude compatibility imports. Described tool commands are never executed.

Release-binary Windows smoke invoked each exact generated PowerShell command
ten times for SessionStart and ten times for a matched PreToolUse denial.
All 60 responses had the expected host JSON and exit status.

| Adapter | Startup median / maximum | Pre-tool denial median / maximum |
| --- | --- | --- |
| Claude | 227.3 / 333.4 ms | 214.2 / 245.4 ms |
| Codex | 203.2 / 322.6 ms | 196.7 / 272.6 ms |
| Grok | 213.7 / 260.9 ms | 196.2 / 267.1 ms |

All were below the configured 3-second timeout on this machine. These local
measurements include PowerShell/process startup and receipt persistence; they
are not host-runtime or production latency guarantees.

Fixtures used isolated home/project/state directories. No installed host trust,
global settings, original Arai/Atlas/Kete checkout or npm credentials changed.
Hosted CI results belong to the [current PR183 head](https://github.com/taniwhaai/arai/pull/183/checks);
this source record does not assert future CI success.
