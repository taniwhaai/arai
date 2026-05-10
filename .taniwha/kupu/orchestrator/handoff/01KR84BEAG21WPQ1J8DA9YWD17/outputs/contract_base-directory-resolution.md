# Manifest: base-directory-resolution

## Responsibility

Selects the Arai base-directory path from a deterministic, five-branch precedence order over injected environment lookups and existence probes, and returns whether that selection warrants a deprecation notice.

## Not responsible for

Reading the process environment, probing the filesystem, discovering the user home directory, emitting any output, migrating any files, or persisting any state across calls. All world-touching capability is injected by the caller; all side effects (including warning emission) are the caller's responsibility.

## Inputs

- **env-lookup** (callable, required): A function the caller supplies. When invoked with an environment variable name (a text string), it returns either the variable's value as a non-empty text string, or an indication that the variable is unset/empty. The resolver invokes this callable with the two variable names described below; it does not access the process environment directly.

  The two names the resolver queries, in order:
  - `ARAI_BASE_DIR` — the current canonical environment variable.
  - `ARAI_DB_DIR` — the deprecated environment variable (legacy alias).

- **path-exists** (callable, required): A function the caller supplies. When invoked with a filesystem path string, it returns a boolean: true if the path currently exists on the filesystem, false otherwise. The resolver invokes this callable with the two well-known default path strings described below; it does not access the filesystem directly.

  The two path strings the resolver queries (constructed from the home-directory value):
  - **new-default**: `<home>/.taniwha/arai` — the current canonical default, where `<home>` is the home-directory value input.
  - **old-default**: `<home>/.arai` — the legacy default.

- **home-directory** (text string, required): The user's home directory, supplied by the caller. The resolver uses this value as the prefix when constructing the two well-known default path strings. The resolver does not discover the home directory itself. If the caller is unable to supply a valid home-directory value, that failure must be signalled before the resolver is invoked; the resolver's behaviour when the home-directory value is empty or absent is undefined and the caller must not invoke it in that state.

## Outputs

- **resolved** (`ResolvedBaseDir`, always present): A single structured value containing two fields:

  - **path** (text string, always present): The filesystem path the rest of the system should treat as the Arai base directory. One of: the value returned by env-lookup for `ARAI_BASE_DIR`, the value returned by env-lookup for `ARAI_DB_DIR`, the new-default path string, or the old-default path string. Never absent. Never empty.

  - **notice** (`DeprecationNotice`, optional — absent in three of the five branches): When present, signals that the chosen path was selected via a deprecated mechanism and that the caller should emit a human-readable warning to interactive users. When absent, no warning is warranted. The notice is one of exactly two variants (see "Referenced data shapes"); no call returns both variants simultaneously.

## Side effects

None. The resolver reads no global state, writes no global state, emits nothing to any output stream, and has no observable effect on anything outside the value it returns. Concurrent invocations from independent callers do not interfere.

## Error semantics

- **env-lookup signals an error**: propagated to the caller of the resolver unchanged. The resolver does not invent a fallback. The five-branch logic is not entered.
- **path-exists signals an error**: propagated to the caller of the resolver unchanged. The resolver does not invent a fallback. The five-branch logic is suspended at the point of failure.
- **home-directory is absent or empty**: the caller must not invoke the resolver in this state. Behaviour is undefined. The caller is responsible for handling missing home-directory before invoking the resolver, consistent with how the existing configuration-load function handles that condition today.
- **No other failure mode**: the resolver does not originate any failure of its own. Given valid inputs (a callable env-lookup, a callable path-exists, and a non-empty home-directory string), the resolver always returns a `ResolvedBaseDir`.

## Behavioural guarantees

- **Determinism**: Given identical inputs — the same responses from env-lookup, the same responses from path-exists, and the same home-directory string — the resolver returns an identical `ResolvedBaseDir`. There is no hidden mutable state, no time-dependent behaviour, no random element, and no source of nondeterminism.

- **Ordering — total, five-branch, short-circuit**: The resolver evaluates branches in the following fixed order and returns on the first branch that matches. Later branches are not evaluated once an earlier branch matches.

  1. If env-lookup(`ARAI_BASE_DIR`) returns a value: return that value as path, no notice.
  2. Else if env-lookup(`ARAI_DB_DIR`) returns a value: return that value as path, notice = `deprecated-env-var`.
  3. Else if path-exists(new-default) is true: return new-default as path, no notice.
  4. Else if path-exists(old-default) is true: return old-default as path, notice = `deprecated-default-path`.
  5. Else: return new-default as path, no notice.

  "First match" means: branch 1 short-circuits branches 2–5; branch 2 short-circuits branches 3–5; and so on. In particular:
  - When `ARAI_BASE_DIR` is set, `ARAI_DB_DIR` is never consulted, and path-exists is never called.
  - When both env vars are unset and both default paths exist, branch 3 matches before branch 4 is reached, so new-default is returned with no notice (not old-default).

- **At most one notice per call**: A `ResolvedBaseDir` carries either no notice or exactly one notice variant. The two notice variants are mutually exclusive. Branches 1, 3, and 5 produce no notice. Branches 2 and 4 each produce exactly one notice. No combination of inputs can produce both variants simultaneously.

- **Idempotency and purity**: Repeated calls with the same inputs return the same result. The resolver has no mechanism by which one call can affect the result of a subsequent call. "Idempotent" holds unconditionally, not conditionally.

- **Concurrency**: Concurrent invocations with independent input sets do not interfere. The resolver holds no shared mutable state. Whether concurrent invocations with the same input set are safe depends on whether the caller's env-lookup and path-exists callables are safe under concurrent invocation; the resolver imposes no additional constraint.

- **Resource bounds**: The resolver invokes env-lookup at most twice and path-exists at most twice per call (fewer when an earlier branch short-circuits). It allocates memory bounded by the size of its inputs (path strings, notice variant). It performs no I/O, no network access, and no unbounded computation.

## Dependencies

None. The resolver calls no other module in the system. Every capability it needs is injected as an input by the caller.

## Referenced data shapes

### `ResolvedBaseDir`

The single value returned by the resolver.

Fields:
- **path** (text string): The chosen filesystem path. Always present, never empty.
- **notice** (`DeprecationNotice` or absent): The deprecation notice, if any. Absent when no warning is warranted (branches 1, 3, 5). Present when a deprecated mechanism was used (branches 2, 4).

### `DeprecationNotice`

A discriminated value with exactly two variants. No other variants exist.

- **`deprecated-env-var`**: Produced when branch 2 matched (the deprecated `ARAI_DB_DIR` environment variable supplied the path). Carries a human-readable message instructing the user to rename the environment variable from `ARAI_DB_DIR` to `ARAI_BASE_DIR`. The exact wording of the message is the implementor's choice; the structural requirement is that the variant exists, is distinguishable from `deprecated-default-path`, and carries a message string the caller can emit verbatim to standard error.

- **`deprecated-default-path`**: Produced when branch 4 matched (the legacy default path `~/.arai` supplied the path because it exists and the new default `~/.taniwha/arai` does not). Carries a human-readable message noting that the `~/.arai` location is deprecated and that a forthcoming `arai migrate` command will assist with moving data to the new location. The exact wording is the implementor's choice, subject to the same structural requirement as above.

Both variants carry a message string. The caller emits the message string to standard error when the notice is present and standard error is attached to an interactive terminal; the caller suppresses it otherwise. The resolver does not inspect or gate on TTY state — that gate is entirely the caller's concern.

## Acceptance criteria

The following criteria are objective and verifiable without access to the implementation. A verifier holding only this manifest and the project context can write tests directly against them.

All tests against the resolver must use injected callables for env-lookup and path-exists. Tests must not set process environment variables, must not create directories on the real filesystem, and must not alter the running process's home directory. These constraints follow from the resolver's pure-function contract and from AC7.

### AC1 — Current env var wins unconditionally

Given:
- env-lookup(`ARAI_BASE_DIR`) returns a non-empty path string P
- env-lookup(`ARAI_DB_DIR`) returns any value (set or unset — the test should verify with both)
- path-exists(new-default) returns any value (true or false)
- path-exists(old-default) returns any value (true or false)

The resolver returns `ResolvedBaseDir` with path = P and no notice.

Verification note: confirm that env-lookup is not called for `ARAI_DB_DIR` and path-exists is not called at all (or that the outcome is identical regardless of those callables' responses). The short-circuit guarantee applies.

### AC2 — Deprecated env var used when current env var absent

Given:
- env-lookup(`ARAI_BASE_DIR`) returns unset/empty
- env-lookup(`ARAI_DB_DIR`) returns a non-empty path string Q
- path-exists(new-default) returns any value
- path-exists(old-default) returns any value

The resolver returns `ResolvedBaseDir` with:
- path = Q
- notice present, variant = `deprecated-env-var`
- the notice carries a non-empty message string

Verification note: confirm the notice variant is specifically `deprecated-env-var`, not `deprecated-default-path`.

### AC3 — New default used silently when it exists

Given:
- env-lookup(`ARAI_BASE_DIR`) returns unset/empty
- env-lookup(`ARAI_DB_DIR`) returns unset/empty
- path-exists(new-default) returns true
- path-exists(old-default) returns false

The resolver returns `ResolvedBaseDir` with:
- path = new-default path string (constructed from the supplied home-directory)
- no notice

### AC4 — Old default used with notice when only it exists

Given:
- env-lookup(`ARAI_BASE_DIR`) returns unset/empty
- env-lookup(`ARAI_DB_DIR`) returns unset/empty
- path-exists(new-default) returns false
- path-exists(old-default) returns true

The resolver returns `ResolvedBaseDir` with:
- path = old-default path string (constructed from the supplied home-directory)
- notice present, variant = `deprecated-default-path`
- the notice carries a non-empty message string mentioning `arai migrate`

Verification note: confirm the notice variant is specifically `deprecated-default-path`, not `deprecated-env-var`.

### AC5 — Fresh-install fallback to new default with no notice

Given:
- env-lookup(`ARAI_BASE_DIR`) returns unset/empty
- env-lookup(`ARAI_DB_DIR`) returns unset/empty
- path-exists(new-default) returns false
- path-exists(old-default) returns false

The resolver returns `ResolvedBaseDir` with:
- path = new-default path string (constructed from the supplied home-directory)
- no notice

### AC6 — New default takes precedence over old default when both exist

Given:
- env-lookup(`ARAI_BASE_DIR`) returns unset/empty
- env-lookup(`ARAI_DB_DIR`) returns unset/empty
- path-exists(new-default) returns true
- path-exists(old-default) returns true

The resolver returns `ResolvedBaseDir` with:
- path = new-default path string
- no notice

Verification note: this is the disambiguator between AC3 and AC4. The result must be identical to AC3, not AC4.

### AC7 — No ambient environment or filesystem access

The resolver's implementation must be callable with arbitrary injected callables without touching the real process environment or filesystem. This is verified structurally: if the implementation accesses the process environment or filesystem directly (rather than through the injected callables), it fails this criterion regardless of whether the tests happen to pass in a controlled environment. The verifier should confirm that the test bodies themselves set no environment variables and create no directories, and that the resolver under test still produces correct results.

### AC8 — Existing test suite continues to pass

After introducing the resolver and the caller-site change in the configuration-loading function, `cargo test` passes with all previously passing tests (no fewer than 277) still passing, plus at least six new tests covering AC1 through AC6.

### Additional: notice mutual exclusivity

No combination of inputs causes the resolver to return a value with both `deprecated-env-var` and `deprecated-default-path` present simultaneously. The `notice` field in `ResolvedBaseDir` is either absent or holds exactly one variant.

### Additional: path field always present and non-empty

For every valid combination of inputs (any combination of env-lookup responses and path-exists responses, with a non-empty home-directory string), the `path` field of the returned `ResolvedBaseDir` is present and non-empty.

### Additional: determinism

For any fixed set of inputs, two successive calls to the resolver return structurally identical `ResolvedBaseDir` values (same path string, same notice presence and variant).
