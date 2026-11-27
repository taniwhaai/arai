# Lessons

- Keep telemetry opt-out variables scoped to disposable CLI fixtures. A global
  DO_NOT_TRACK or ARAI_TELEMETRY override during cargo test can disable the
  transport exercised by telemetry unit tests and leave their loopback servers
  waiting. Record test-runner environment and avoid unrelated blanket overrides.

- 2026-11-27: Keep release work focused on working user flows. Record a bounded
  procedural exception once, obtain the necessary independent review, and
  continue implementation. Reuse successful checks against unchanged source
  hashes when only a contract clarification changes. Ask for approval once and
  carry the user's authorization forward. Use explicit UTF-8 and atomic writes
  for scripted source edits on Windows.

- Parallel subprocess tests can inherit an open Unix flock between fork and
  exec. Explicitly unlock a fixture before asserting immediate availability;
  closing its descriptor alone is not a deterministic synchronization point.

- 2026-09-16: When hardening Arai, check Atlas's binding decisions and Kete's
  real library usage before changing discovery, policy lifecycle or hook APIs.
  A local host adapter must not narrow the shared enforcement core or turn an
  optional upstream integration into a mandatory remote dependency. Track and
  resolve surfaced gaps explicitly instead of leaving them in review notes.

- Windows installer migrations need native Git Bash checks: MSYS resolves
  `test -e arai` to arai.exe even when no extensionless file exists. Enumerate
  actual filenames before moving a legacy payload, preserve it outside PATH,
  and verify repeated installs do not accumulate unnecessary backups.
