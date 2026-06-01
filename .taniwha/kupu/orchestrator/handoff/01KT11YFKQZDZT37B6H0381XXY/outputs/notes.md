# Implementor notes — copy-tone-audit

## AC satisfaction summary

**AC1** — `docs/voice.md` created with the VoiceSpec (6 rules) and SelfReferenceGlossary (5-row table) verbatim from the contract.

**AC2** — `src/hooks.rs` deny strings retuned: `"Arai: blocking rule matched"` → `"Arai: a rule blocked this action."` (3 sites); `"Arai: internal error, blocking for safety"` → `"Arai: an internal error occurred; blocking this action."` (2 sites). JSON protocol keys (`permissionDecision`, `deny`, `hookEventName`, etc.) unchanged. Glyph wrappers and colour wrappers untouched.

**AC3** — `src/init.rs` init footer changed from `"Done. Arai is now watching your rules (Claude + Grok TUI)."` to `"Arai is enforcing this project's rules (Claude Code and Grok TUI)."`. Deinit footer changed from `"Arai is no longer watching this project."` to `"Arai is no longer enforcing this project's rules."`. All `"Failed to ..."` errors on init/deinit paths changed to `"Could not ..."`.

**AC4** — `"Failed to ..."` → `"Could not ..."` systematically across `src/hooks.rs`, `src/init.rs`, `src/main.rs`, `src/store.rs`, `src/upgrade.rs`, `src/extends.rs`. Internal-only strings (e.g. comments, dev-only log entries) left untouched.

**AC5** — `"No guardrails found. Run \`arai init\` first."` → `"No rules found. Run \`arai init\` first."`. Audit/stats empty-state lines calmed ("Rules haven't fired yet" → "No rules have fired yet"). `src/stats.rs` "Arai stats" header was already sentence case — left as-is.

**AC6** — "guardrail(s)" in user-facing prose changed to "rule(s)". `src/guardrails.rs` `format_context` header changed from `"Arai guardrails:"` to `"Arai rules:"`. The `arai guardrails` command name and the `guardrails` concept as collective system are preserved. No "we", "I", "the user", "the agent", "the assistant", "Claude" in user-facing strings.

**AC7** — README intro/tagline replaced with restrained-declarative infrastructure statement. No deeper sections touched. Follow-on flagged in manifest.

**AC8** — Behaviour, JSON protocol, colour (#83), and glyph (#84) all unchanged. Confirmed by reviewing every diff: only string content changed.

**AC9** — Grep of all test files (`tests/`, `#[cfg(test)]` blocks in `src/`) for every changed string literal returned zero matches. No test assertions required updating.

**AC10** — `cargo fmt --all`: clean. `cargo clippy --all-targets`: no new warnings (two pre-existing warnings in `src/store.rs` test block unchanged). `cargo test`: 18 suites, all green.

## Scope boundary decisions

- `src/guardrails.rs` `format_context` header `"Arai guardrails:"` is classified as user-facing `additionalContext` prose per the contract ("including the `'Arai guardrails:'` header"), changed to `"Arai rules:"`.
- `src/store.rs` error strings from `Store::open()` propagate directly to the user via `main.rs`'s error handler, so they are user-facing and were retuned.
- Step headers in `src/init.rs` (`"Scanning for instruction files..."`, etc.) use sentence case already — no change needed.
- The "Post-action review" prefix in PostToolUse context was shortened to "[Post-action]" to remove the review-flourish noun, while keeping the locating bracket format.
