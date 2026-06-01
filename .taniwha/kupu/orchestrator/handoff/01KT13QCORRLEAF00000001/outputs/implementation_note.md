# Corrective leaf implementation note — AC6 glossary-consistency nits

## Files changed

### src/init.rs (line 24)
- "Extracting guardrails..." -> "Extracting rules..."

### src/main.rs (lines 1213, 1337, 1559)
- "No guardrail database found.  Run `arai init` first." -> "No rule database found.  Run `arai init` first."

### src/scenarios.rs (lines 89, 165)
- Line 89: "Failed to read scenario file {}: {e}" -> "Could not read scenario file {}: {e}"
- Line 165: "No guardrail database found.  Run `arai init` first before running scenario tests." -> "No rule database found.  Run `arai init` first before running scenario tests."
  (This string was also applying the unit-term fix — same pattern as main.rs, user-visible error path.)

### src/enrich.rs (16 occurrences)
All user-visible "Failed to <verb> <thing>: {e}" strings changed to "Could not <verb> <thing>: {e}":
- create ONNX session builder, set optimization level, load model, load tokenizer,
  create model directory, run curl, create input_ids tensor, create attention_mask tensor,
  create token_type_ids tensor, extract tensor, parse JSON, read API response,
  parse API response as JSON, read {path}, run '{binary}', run '{binary}' with stdin.

## Gate results

- `cargo fmt --all`: clean (no output)
- `cargo fmt --all -- --check`: clean (no output)
- `cargo clippy --all-targets`: 0 new warnings (2 pre-existing in src/store.rs, unrelated to this change)
- `cargo test`: 584 passed, 0 failed, across 18 test suites

## Lockstep grep result

Zero matches on all old strings:
- "Extracting guardrails" — not found
- "No guardrail database found" — not found
- "Failed to" in src/enrich.rs — not found
- "Failed to read scenario" — not found

## Test assertions updated

None. No test in tests/ or src/ asserted any of the changed strings.
