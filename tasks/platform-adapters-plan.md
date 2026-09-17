# Platform adapters and Cursor pilot

Approved by the user: “Lets do it”, following the adapter-first proposal.
Base: main `cb26bb3`; branch `codex/platform-adapters-and-cursor`.
The session has no enter/exit-plan-mode tools; this file records the already-approved plan.

- [x] Check native Cursor contracts, existing host behavior, and Atlas/Kete boundaries.
- [x] Add an explicit platform registry and CLI dispatch without changing supported embedding APIs.
- [x] Normalize Cursor pre/post tool calls into the existing matcher, including conservative file mutation handling, validated paths, session identity, and native response/error encoding.
- [x] Add selective, persistent init/deinit registration; preserve unrelated hooks and user trust settings.
- [x] Test protocol, failure, ownership, scoped enforcement, and existing Claude/Grok/Codex/Kete contracts on Windows and Linux.
- [x] Document capability limits, duplicate compatibility registration, and a real-host verification procedure.
- [x] Review the diff, run required checks, and prepare a focused pull request.

Scope: Cursor native preToolUse/postToolUse only. Prompt injection is not documented by Cursor and is not advertised or registered. Unknown mutation payloads fail closed; no filesystem existence guesses. Native Cursor has no documented allow-side advisory injection. Never suppress a second hook merely because a configuration file exists: configuration is not proof of activation. Warn about enabled Claude imports and explain how the operator can choose one integration in Cursor. Repeated Arai registration must not duplicate owned handlers.

Preserve hooks::match_hook and existing Config/Store/audit APIs. All matching remains local and available offline; Kete continues to own policy acceptance, identity, leases and evidence transport. Do not mutate build-state metadata or external policy ownership. Npm publication remains deferred. PR182 is independent and remains open.

Live verification limitation: Cursor is absent on Windows and WSL. Fixtures and executable protocol tests do not establish activation or blocking inside Cursor; report that separately until an installed, authenticated host is available.

Validation: Windows/Linux default suites, final Cursor13 + shared safety6 on each, native Windows enrich suite, Windows lean embedding/audit/Cursor/registration suites, rustfmt and Clippy passed. Local WSL enrichment build requires missing OpenSSL/pkg-config packages; PR CI will verify that configuration. Registered Windows release-command smoke: 30 correct responses, worst 411 ms of 3000 ms timeout. See docs/upstream/cursor-adapter-validation-2026-09-16.md.
