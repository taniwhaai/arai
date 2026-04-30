# Synthetic parser-coverage corpus

This file is a synthetic CLAUDE.md shaped to exercise every pattern in
`parser.rs::match_imperative`, including the v0.2.11 broadening additions.
Each list item below is paired with an entry in `expected.jsonl` saying
whether the parser should extract a rule from it (and at which layer),
or whether it should skip it as a non-rule.

The shapes were curated from a 93-file public-corpus survey; counts of
each pattern in that survey are noted in the v0.2.11 PR description.

## Layer 1 — start-of-sentence imperatives

- Never force-push to main
- Always run cargo test before merging
- Don't skip code review
- Do not commit `.env` files
- Must not bypass tests
- Must use feature branches
- Avoid hand-writing migrations
- Ensure all imports are sorted
- Stop pushing without review
- Skip generated files in git
- Only push to feature branches

## Layer 1 — v0.2.11 additions

- Should run linter before commits
- Should not commit binary blobs
- Shouldn't push without running tests
- Cannot commit private keys
- Refuse to merge without two approvals
- Enforce strict typing in Python
- Make sure tests pass before pushing
- Make sure that imports are sorted
- Be sure to run prettier before commit
- Consider compression for distribution
- Recommend using uv over pip
- Recommended pattern is dependency injection

## Layer 1b — bare `No X` prohibitions

- No AI attribution in commit messages
- No emojis in commit messages

## Layer 1b — bold-label `**No X**` is descriptive, must NOT extract

- **No build process** - this is a zero-build extension.
- **No CORS handling** — Traefik manages all cross-origin handling.
- **No authentication code** - Traefik handles all auth via ForwardAuth.

## Bold-label `**Consider X:**` is a section header, must NOT extract

- **Consider constraints:** What are the goals and limitations?

## Bold-emphasis on a Layer 1 leader IS a rule (guard does not over-fire)

- **Always** run tests before push

## Layer 5 — `use X` with section-context gate

## Conventions

- Use the `cn()` utility from $lib/utils for class merging
- Use proper temp directory for downloads

## Architecture

- Use the diagram below to follow the flow

## Layer 6 — verb-start catch-all (v0.2.11 verb additions)

- Create lookup functions for quick queries
- Implement try_from for type-specific parsing
- Document decision-making processes
- Define color variables in `_sass/`
- Store results for each socket separately

## Layer 7 — conditional imperatives

- When working in parallel, run tests in isolation
- Before completing work, run the full test suite
- After every code change, run the linter
- If the test suite is slow, use `--release` for benchmarks
- For tasks that need more compute, use subagents to work in parallel
- When suggesting changes: state impact on the broader system
- If missing → show "Data Download Required" dialog
- When deploying to production, never skip smoke tests

## Conditional with unrecognised verb — must NOT extract

- When uncertain, see the troubleshooting guide for guidance

## Plain prose (descriptive) — must NOT extract

- The same seed always gives the same mods in the same socket location
- Background file watcher auto-updates the graph on changes
- Thread-safe queries and watcher share a lock
