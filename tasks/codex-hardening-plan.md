# Arai hardening and Codex integration

User authorization: resolve the reviewed issues, then add Codex support. Based on remote main c47d499.

- [x] Fix shell self-command exemption, preserve blocking matches, and parse diff hunks safely; add reproductions as regression tests.
- [x] Repair pre-commit ownership/removal, worktree/custom hooks paths, and refresh existing registrations without deleting unrelated hooks.
- [x] Fix full-binary staging and Homebrew sequencing; make release credential requirements explicit and validate preflight behavior without exposing secrets. Actual RELEASE_TOKEN replacement remains an account-owner action.
- [x] Add native Codex hook registration, trust guidance, Bash compatibility, and per-file apply_patch normalization using the existing matcher.
- [x] Align installation/platform docs and changelog; preserve Claude and Grok contracts.
- [x] Run formatting, lint, default and enrichment tests, targeted hook/patch scenarios, and independent review. Hosted enrichment passed; follow-up CI validates its newer Clippy suggestion and the permanent Windows hook job.
- [x] Commit and prepare a draft PR; report any credential or host-trust action that still requires the account owner. Draft PR: https://github.com/taniwhaai/arai/pull/180.

Scope: isolated checkout; do not alter the user's original checkout, global host configuration, or existing worktrees. Codex applies only documented hook contracts; do not bypass host trust. Patch parsing must fail closed for unsupported/malformed inputs. Existing rule schema has no Delete action, so document that limitation explicitly rather than claim deletion coverage. Normal release must remain tag-driven and use a valid PAT/App token to trigger downstream jobs.

Validation before draft PR: complete default suites pass on Windows and Linux;
CI-equivalent Clippy passes on both; formatting, actionlint, installer sync and
release-script fixtures pass. Independent review of registration, patch matching
and release workflows completed; review findings have regression tests.
Local Linux enrichment build needs missing system pkg-config/OpenSSL development
dependencies; the hosted enrichment job passed on the implementation commit.
An expanded all-target Clippy run also identified four pre-existing test-only
lints outside this change; the required CI Clippy scope passes.
Windows registration tests execute the generated PowerShell hook from a path
with spaces, apostrophe and dollar sign. Live Codex trust/activation and actual
publishing are not claimed; no credentials or user/global configuration changed.
