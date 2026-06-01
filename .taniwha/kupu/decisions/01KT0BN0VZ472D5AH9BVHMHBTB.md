# Decision 01KT0BN0VZ472D5AH9BVHMHBTB

**Kind:** scope_change (contract promotion + leaf dispatch)
**Triggered by:** subagent_returned:contract-derivation:01KT0B51P5GZW77R4DJE63C9D9
**Orchestrator invocation:** 38

## What was decided

The contract-derivation subagent for module `gateway-outcome-glyphs` (design v6, issue #84) returned successfully with a complete single-module contract covering AC1–AC10. The contract was promoted to the canonical path `kupu/contracts/gateway-outcome-glyphs/v1.md`.

A leaf-implementation subagent was dispatched with handoff `01KT0BMTR2H69NN8FD0MQWCVZ6` to implement the contract against the working tree on branch `feat/84-gateway-glyphs`.

## Contract summary

The contract specifies two new functions added to `src/style.rs`:

- `should_use_unicode() -> bool` — reads `ARAI_ASCII`, `NO_UNICODE`, and locale variables (`LC_ALL` > `LC_CTYPE` > `LANG`); returns true only if no ASCII-force override is set and the locale contains `utf-8`/`utf8` (case-insensitive). TTY-independent.
- `outcome_glyph(outcome, unicode, colorize) -> String` — maps Block/Warn/Inform/Allow to gateway glyphs (Unicode: `●·│✕`/`│●│`/`●·│`; ASCII: `o.|x`/`|o|`/`o.|`). Ochre colour only on the `✕` in Unicode blocked form when `colorize=true`. Total function, no errors.

## Hard carve-outs (binding)

1. `src/hooks.rs` always passes `colorize=false` — glyph chars only, zero ANSI in hook output.
2. `--json` paths carry no glyph codepoints whatsoever.
3. Audit-chain `✓`/`✗` integrity markers are untouched.

## Integration surfaces (call-site changes only)

- `src/main.rs` — `arai audit` + `arai why` human render paths
- `src/stats.rs` — replace generic `⚠` with warned glyph
- `src/hooks.rs` — prefix Pre/Post deny reason + additionalContext with glyph, `colorize=false` literal

## Why this path

Single_module tier. Contract returned cleanly with no re-raise. The contract covers all 10 acceptance criteria and all three hard carve-outs. Straight dispatch to leaf-implementation per tier rules.

## Affects

- contracts/gateway-outcome-glyphs: v1 (new, current)
- tree: to be established by leaf dispatch
