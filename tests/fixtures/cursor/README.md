# Cursor native protocol fixtures

These are synthetic fixtures based on the official
[Cursor hook reference](https://cursor.com/docs/hooks), reviewed 2026-09-16.
They are not payload captures from a live Cursor version.

`pre-shell.json` exercises documented native names, command/working-directory
translation and correlation fields. `post-shell.json` exercises the documented
JSON-stringified tool output. File schemas in `src/cursor.rs` tests are
conservative accepted shapes; the generic Write/Delete schema is not fully
specified by the reference and remains a host-verification requirement.

Before promoting an environment, add sanitized captured fixtures and record
Cursor version, OS, desktop/CLI/cloud mode, enabled registration, allow/deny
results and timeout/error behavior in a verification report. See
`docs/cursor.md`. An SDK/CLI version or fixture pass alone does not prove
actual host interception.
