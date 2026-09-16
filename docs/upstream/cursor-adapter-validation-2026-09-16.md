# Cursor adapter validation — 2026-09-16

This is an **Arai executable/protocol** verification record. Cursor itself
was not installed on the Windows machine or its Ubuntu WSL environment;
no authenticated Cursor agent session was exercised. No Cursor version or
desktop/CLI/cloud mode is promoted to live-verified by this record.

Primary references: [native hooks](https://cursor.com/docs/hooks) and
[Claude compatibility](https://cursor.com/docs/reference/third-party-hooks).
Fixtures are synthetic, not captured from an actual host version.

## Local results

Rust 1.95.0, native Windows x64 and Ubuntu WSL x64:

| Check | Result |
| --- | --- |
| Default test suite | Passed on both systems, including existing host and embedding regressions |
| Final Cursor subprocess / shared safety suites | 13 + 6 passed on each system |
| Cursor normalizer unit tests | 12 passed on each system |
| Registration tests | Passed on each system; Windows also executes encoded PowerShell from a path with spaces, apostrophe and dollar sign |
| No-default-features embedding, audit concurrency, Cursor and registration tests | Passed on Windows |
| Default/all-target and no-default-features Clippy | Passed on Windows |
| Enrichment test suite | Passed on Windows; local WSL build unavailable because pkg-config/OpenSSL development packages are absent |
| Rustfmt | Passed |
| Real Cursor allow/block, timeout and activation | Pending |
| macOS host/runtime verification | Pending |

Additional Windows release-binary smoke: invoked the exact generated
PowerShell command 15 times for allowed `cargo check` payloads and 15 times
for blocked `cargo clean` payloads. Checked native JSON, exit codes and that
the denial cited the rule rather than an internal error. The described
cargo commands were never executed. Allow median/max was 219/300 ms;
deny median/max was 208/411 ms, below the registration's three-second timeout
on this machine. This is a local latency sample, not a production SLO or a
measurement of Cursor's own startup/timeout behavior.

All fixtures used isolated projects, home directories and state stores.
No editor trust, global hooks or policy credentials were changed. Tests
cover malformed/oversize input, registration event spoofing, corrupt stores,
unknown mutations, scoped files, conservative Write/Edit union, a lower-score
block surviving a higher-score advisory, audit deduplication, and avoiding
false advisory/compliance/token-savings claims.

For promotion, follow the [real-host procedure](../cursor.md) and append a
separate version/environment-specific record with sanitized captured payloads.
