# Existing hosts and early availability

User-authorized follow-up to PR183: accommodate current Claude/Grok/Codex hook events and tool verbs, and make Arai visible early in TUI/desktop sessions. Extend the existing adapter branch; preserve Cursor's opt-in pilot boundary and Kete's supported APIs. The session does not expose plan-mode tools; this file records the implementation plan for the requested follow-up.

- [x] Review current primary contracts and compare registration, native tool names, lifecycle output, activation and desktop/TUI surfaces.
- [x] Move Claude/Grok/Codex registrations to explicit platform and pinned event dispatch, migrating only owned handlers and preserving trust settings.
- [x] Add useful startup/subagent lifecycle handling with concise local availability/context and visible diagnostics where supported; never invent a Grok context-delivery surface.
- [x] Repair documented Claude verb, watcher, batch-accounting and denial-reason drift; preserve existing enforcement behavior and source custody.
- [x] Keep advisory output permission-neutral: omit explicit Claude/Codex allow, and observe PermissionDenied without an automatic retry request.
- [x] Record bounded local invocation evidence and distinguish configuration/paths, observed invocation and host activation in status diagnostics.
- [x] Test modern contracts, legacy migration, no-network/fast startup, malformed inputs and existing host/Kete/Windows/Linux suites; document environment-specific activation steps and remaining live verification limits.
- [ ] Update PR183 and wait for final-head CI.

Startup must not run discovery::discover, scan, remote extends, models, telemetry flushes, or broad source traversal. It reads accepted local state; missing/stale/disabled state is explicit. No global hook installation or host trust edits in the user's environment. No permission auto-approval or retry loops. Lifecycle events must never fall through into tool-decision encoding. Only a real invocation establishes local invocation evidence; neither a config file nor a SessionStart receipt proves all tool gates were trusted. Record no prompt/tool contents or credentials in diagnostics.

Useful scope: SessionStart/SubagentStart, canonical passive-event recognition, correct known-tool normalization, existing Claude lifecycle repairs, platform-specific command launch and concise activation instructions. Other available hooks get documented as intentionally unused unless they close a demonstrated gap. Full automatic policy refresh, plugin distribution and exact cross-host call attribution remain separate work requiring their own contracts.

Validation and remaining host-level limits: docs/upstream/host-lifecycle-validation-2026-09-16.md. Final-head CI is tracked on PR183 rather than asserted by the source commit that triggers it.
