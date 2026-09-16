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

## Follow-up: remaining holes and stack compatibility

User explicitly requested fixing the surfaced holes, looking for more evident
defects, and consulting Atlas so Arai's joins with Kete and the wider code stack
remain open. Continue on PR #180; preserve original working copies.

- [x] Read current Atlas decisions/manifests and Kete's actual embedding contract; distinguish binding architecture from draft proposals.
- [x] Reproduce discovery/refresh and other evident enforcement gaps; design scope handling before ingesting nested or path-scoped instructions.
- [x] Fix confirmed gaps with regressions, preserving public embedding APIs, offline/local decisions, provenance, and externally managed policy state.
- [x] Add an embedding compatibility check based on Kete's consumption; update architecture/coverage docs and eliminate surfaced test-only lint defects.
- [x] Review the combined change; run platform tests, enrichment CI, formatting and lint; update PR #180 with evidence and remaining operational requirements.

Architectural starting point from Atlas: Arai is the open enforcement library
upstream of kete-agent; Kete adds the graph/org control plane and carries
codeworld grounding. Remote services must not become mandatory for local
enforcement. No new organization-specific transport or host trust bypass is
part of this hardening pass.

Follow-up local validation: complete default suites pass on Windows (710 tests)
and Linux (712), with one existing ignored doc test on each. All-target Clippy
passes; Linux no-default-features embedding/audit tests (13) and all-target lint
pass. Language-only scanner compilation and five normalization tests pass.
Actionlint and patch whitespace checks pass. Independent review covered source
scope, ownership migration, provenance and audit concurrency. Hosted run
35049540138 passed default, enrichment, Windows, formatting and release checks;
its newer all-target Clippy found one test-only chunks_exact warning, now fixed.
The rerun is tracked on PR #180. RELEASE_TOKEN metadata now shows an
update at 2026-09-16T02:20:50Z; the last main release failure predates that update,
so the replacement token is unverified rather than known-invalid. No Homebrew
tap token is configured. No release or host-trust state was changed.

Hosted rerun 35049787601 passed every applicable check, including lean and
enrichment builds. A final reproduced retention race is also fixed: purge
respects the writer's bucket lock and reports failed removals accurately.
Focused audit checks pass on Windows (24 tests) and Linux (26); the final PR
run rechecks the complete suites after this bounded follow-up.
