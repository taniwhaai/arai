# Manifest: directive-tokenisation

## Responsibility

Classifies a single raw `arai:extends` directive line into its URL, optional content-pin, and optional tier components by whitespace-token shape — producing either a `ParsedDirective` or a `MalformedDirective` outcome — and performs no I/O.

## Not responsible for

Fetching content, comparing content hashes, reading or writing the trust file, verifying signatures, or applying tier semantics to any rule. This module produces a parsed value and nothing else.

---

## Settled decisions (BINDING — do not reopen or hedge)

**AC11_drop_syntax:** tier=override drop syntax is triple-equality (implicit). A local rule whose subject-predicate-object exactly matches an upstream rule implicitly drops that upstream rule when tier=override is in effect. No new directive keyword. Owned by tier-provenance. (This decision closes Open question 1 in the design; directive-tokenisation is not affected except that it must correctly classify the tier=override token as a valid tier value producing the Override variant.)

**AC12_duplicate_token:** Duplicate directive tokens (two @<pin> or two tier=) = fail-closed (skip + warn). A directive carrying more than one @<pin> OR more than one tier= token is malformed. The directive is skipped (local content preserved) and a stderr warning is emitted naming the duplicate. Owned by directive-tokenisation (AC12 family).

---

## Inputs

- **directive_line** (`non-empty string`, required): The raw directive text after the `arai:extends` marker has been stripped from the line. Accepted in either the `#` form (plain text) or the `<!-- ... -->` HTML-comment form (with comment delimiters already stripped by the caller before this module is invoked, or this module must handle stripping — see Behavioural guarantees). Contains zero or more whitespace-separated tokens following the URL.

## Outputs

- **success outcome** (`ParsedDirective`): Produced when all tokens are successfully classified. Fields: `url` (non-empty string, the first token), `pin` (optional 64-character lowercase hex string, absent when no `@<pin>` token present), `tier` (optional `Tier` variant, absent when no `tier=` token present). Defined in shared vocabulary.
- **failure outcome** (`MalformedDirective`): Produced when classification fails. Fields: `offending_token` (the token that caused the failure), `reason` (human-readable description for the warning). Defined in shared vocabulary. The caller is responsible for emitting the stderr warning and skipping the directive.

The module produces exactly one of these two outcomes for every valid input.

## Side effects

None. This module is pure computation with no I/O, no file access, no network calls, and no mutable shared state.

## Error semantics

- **Unknown trailing token** — a whitespace-separated token after the URL that does not begin with `@` and is not of the form `tier=<value>`: signalled as `MalformedDirective` with the offending token named; caller is contracted to emit a stderr warning and skip the directive, preserving local content. [AC12]
- **Unknown `tier=` value** — a `tier=` token whose value is not one of the three valid strings (`strict`, `advisory`, `override`): signalled as `MalformedDirective`. [AC12]
- **Malformed pin token** — an `@`-prefixed token whose remainder (after stripping the `@`) is not exactly 64 lowercase hex characters (i.e. not matching `[0-9a-f]{64}` after lowercase normalisation): signalled as `MalformedDirective`. Content comparison (whether the pin matches the fetched bytes) is the fetch-verification module's concern; this module only checks syntactic validity of the pin token. [AC3 — tokeniser half]
- **Duplicate `@<pin>` token** — two or more `@`-prefixed tokens present in the same directive: signalled as `MalformedDirective` with the second occurrence named as the offending token. [AC12_duplicate_token, BINDING]
- **Duplicate `tier=` token** — two or more `tier=` tokens present in the same directive: signalled as `MalformedDirective` with the second occurrence named as the offending token. [AC12_duplicate_token, BINDING]
- **No trailing tokens** — the bare `arai:extends <url>` case with no tokens after the URL: this is the success path. Produces a `ParsedDirective` with `pin` absent and `tier` absent. No new branch is taken. This is the backward-compatibility invariant for the directive path. [AC1]

There are no partial-success outcomes. The result is always either a fully-classified `ParsedDirective` or a `MalformedDirective`.

## Behavioural guarantees

- **Backward-compatibility invariant (tokeniser half):** Given a directive line whose only token after the `arai:extends` marker is a URL (no trailing tokens), the output is a `ParsedDirective` with `url` equal to that URL, `pin` absent, and `tier` absent (Peer). This is observably identical to the pre-slice parse result. No new code path is entered. The new pin-comparison and signature-check branches are entered only when a `@<pin>` or `tier=` token is present. [AC1]
- **Token classification is by shape, order-independent among optional tokens:** A directive listing `@<pin>` then `tier=` and a directive listing `tier=` then `@<pin>` produce identical `ParsedDirective` values (fields match, regardless of token order).
- **Absent `tier=` defaults to Peer at the caller boundary:** When `tier` is absent in the `ParsedDirective`, callers interpret this as the `Peer` tier. This module does not substitute the default — it leaves `tier` absent. The `Peer` default is applied by callers reading the optional field.
- **In-URL `@` is not a pin token:** An `@` character appearing inside the URL token (e.g. in userinfo syntax, `user@host`) is part of the URL and is not classified as a pin token, because it is not preceded by surrounding whitespace (it is not a separate whitespace-delimited token).
- **HTML-comment form:** When the directive appears in `<!-- arai:extends ... -->` form, the comment delimiters are stripped and the inner content is processed identically to the `#` form. The module either accepts already-stripped input or performs the stripping itself — the contract requires that both surface forms produce identical results for equal semantic content.
- **Idempotency:** Calling this module with the same input any number of times produces the same output. No state is accumulated.
- **Determinism:** Output is fully determined by input. No randomness, no time-dependency.
- **Concurrent invocation safety:** The module holds no shared mutable state; concurrent invocations are safe without coordination.
- **Resource bounds:** Memory allocated is proportional to the length of the input string. No per-call allocation beyond input size. No external calls.

## Dependencies

None. This module has no dependencies on other modules in this system and no external I/O dependencies.

## Referenced data shapes

All defined in the shared vocabulary file:
- `ParsedDirective` — the success output shape
- `MalformedDirective` — the failure output shape
- `Tier` — the enumeration carried in `ParsedDirective.tier`

## Acceptance criteria

All criteria are verifiable from string inputs and outputs alone, without any I/O.

**AC1 — Bare directive backward-compatibility (tokeniser half):**
Given a directive line containing only a URL token (e.g. `https://example.com/policy.md`) with no trailing tokens, when the module classifies the line, then the output is a `ParsedDirective` with `url` equal to the URL, `pin` absent, and `tier` absent. No `MalformedDirective` is produced.

**AC3 — Malformed pin token rejected (tokeniser half):**
Given a directive line with a trailing `@`-prefixed token whose remainder is not exactly 64 lowercase hex characters (e.g. `@abc123`, `@ABCDEF...` with uppercase, `@` alone, `@` followed by 63 hex chars), when the module classifies the line, then the output is a `MalformedDirective` naming the offending token.

**AC12a — Unknown trailing token rejected:**
Given a directive line containing a trailing token that does not begin with `@` and is not of the form `tier=<value>` (e.g. `foo`, `bar=baz`, `strict` without `tier=`), when the module classifies the line, then the output is a `MalformedDirective` naming the offending token.

**AC12b — Unknown tier= value rejected:**
Given a directive line containing `tier=unknown` (or any value other than `strict`, `advisory`, `override`), when the module classifies the line, then the output is a `MalformedDirective` naming the offending token.

**AC12c — Known tier= values accepted:**
Given a directive line containing `tier=strict`, `tier=advisory`, or `tier=override`, when the module classifies the line, then the output is a `ParsedDirective` carrying the corresponding `Tier` variant (`Strict`, `Advisory`, or `Override` respectively).

**AC12d — Duplicate `@<pin>` token fail-closed [BINDING: AC12_duplicate_token]:**
Given a directive line containing two or more `@`-prefixed tokens both syntactically valid as pin tokens, when the module classifies the line, then the output is a `MalformedDirective` naming the second (duplicate) pin token and indicating it is a duplicate.

**AC12e — Duplicate `tier=` token fail-closed [BINDING: AC12_duplicate_token]:**
Given a directive line containing two `tier=` tokens (even if both carry the same valid value), when the module classifies the line, then the output is a `MalformedDirective` naming the duplicate.

**AC12f — Order-independence of optional tokens:**
Given two directive lines that are identical except that `@<pin>` and `tier=<value>` appear in reversed order, when both are classified, then both produce `ParsedDirective` values with equal `url`, equal `pin`, and equal `tier` fields.

**AC12g — Valid 64-char hex pin accepted:**
Given a directive line with a trailing token `@` followed by exactly 64 lowercase hex characters, when the module classifies the line, then the output is a `ParsedDirective` with `pin` set to those 64 characters.

**AC12h — In-URL `@` not misclassified:**
Given a directive line whose URL token contains an `@` character (e.g. `https://user@host/path`) with no separate whitespace-delimited `@<pin>` token, when the module classifies the line, then the output is a `ParsedDirective` with the full URL preserved and `pin` absent.

**AC14 — Full gate:**
The full local gate passes: `cargo fmt --all --check` reports no formatting issues, `cargo clippy --all-targets` reports no new warnings, and `cargo test` passes all tests including the existing extends suite and any new unit tests for this module.
