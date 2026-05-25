# Arai legacy-path migration prompt

## Structural tier

**Selected:** single_module

**Justification:** Brief v4 confines all changes to `src/config.rs` (the new migration-prompt logic) and `src/main.rs` (a single new call after `Config::load` in the `arai init` command). The migration-prompt logic introduces no new cross-module contract, no shared data shape that other modules consume, and no public surface that any module other than the `arai init` entry point needs to call. The brief's own tier rationale, the "Out of scope" list, and AC9 (no other tests change) all reinforce this. The new module sits beside `base-directory-resolution` (stable from cycle #73), consumes its `DeprecatedDefaultPath` notice, and exposes a single capability to the `arai init` entry point. Per the design-doc skill, this is the canonical `single_module` shape.

**Module count:** 1.

## Relationship to the stable contract from `design/v1.md`

Cycle #73's design defined `base-directory-resolution` as a pure resolver returning a `ResolvedBaseDir` value containing a path plus an optional, mutually-exclusive `DeprecationNotice`. That contract is stable, in main, and **not changed** by this cycle. In particular:

- The resolver is not modified.
- `ResolvedBaseDir` is not modified.
- `DeprecationNotice` and its two variants (`DeprecatedEnvVar`, `DeprecatedDefaultPath`) are not modified.
- The caller-site change in `Config::load` from cycle #73 — emit a one-shot stderr warning when a notice is present and stderr is interactive — is preserved unchanged.

This cycle's migration-prompt logic plugs in **after** `Config::load` returns, **only inside the `arai init` command**, and **only when the returned configuration carries a `DeprecatedDefaultPath` notice that survived `Config::load`'s warning emission**. It does not run inside `Config::load` (which is shared by every command, including the hot-path hook subprocess) and it does not run in any other command's entry point. The structural commitment: the migration check is downstream of `Config::load`, gated on the same notice the shim already exposes.

## Purpose

Offer existing Arai users a one-time, opt-in, default-no migration of `~/.arai/` to `~/.taniwha/arai/` the first time they run `arai init` after upgrading, and remember their decline so the offer does not re-fire on subsequent invocations.

## External boundaries

- **Standard input**: inbound. The system reads a single line of user input to capture the `y / N` migration decision. Reading is gated on standard input being attached to an interactive terminal; non-interactive `arai init` invocations short-circuit without prompting (treated as decline-without-marker, since no human is present to record a decision).
- **Standard output**: outbound. The prompt summary (source path, destination path, file count, total size, the question itself) and the post-migration confirmation are written here. Treated as user-facing UI, not as a structured event channel.
- **User home directory (filesystem, read)**: inbound. The system inspects the legacy path to compute the migration summary (file count, total size) and to check for the presence of the decline marker.
- **User home directory (filesystem, write)**: outbound. On accept, the legacy directory tree is moved into the new location. On decline, a single zero-content marker file is created inside the legacy directory.
- **Process environment**: not touched. No environment variable is read or written by this module.

## Modules

### legacy-path-migration

**Responsible for:** Deciding whether to offer a legacy-path migration on a given `arai init` invocation, conducting the prompt UX when the offer is warranted, performing the directory move on accept, and recording the decline state on decline.

**Not responsible for:** Resolving the base directory (that is `base-directory-resolution`), emitting the deprecation warning (that is `Config::load`'s caller-site behaviour preserved from cycle #73), gating itself on whether the calling command is `arai init` (that is the entry-point's responsibility — the entry point only calls this module from `cmd_init`), or providing a "force re-prompt" / "reset decline" mechanism (out of scope per the brief).

**Inputs:**

- The `ResolvedBaseDir` value previously produced by `base-directory-resolution` and consumed by `Config::load`. The module reads the optional deprecation notice on this value and acts only when the notice is present and is the `DeprecatedDefaultPath` variant. All other notice states (absent, `DeprecatedEnvVar`) cause a silent no-op return.
- An injectable capability for testing whether a given path currently exists on the filesystem. Supplied to the module, not performed directly, so tests can drive the marker-present and marker-absent branches without touching the real filesystem.
- An injectable capability for collecting summary statistics about a directory tree (file count and total byte size). Supplied to the module, not performed directly, so tests can drive the prompt-text branches without populating real fixtures.
- An injectable capability for performing the directory move from a source path to a destination path with the atomicity semantics described under "Behavioural guarantees". Supplied to the module, not performed directly, so tests can simulate both the same-filesystem rename success path and the cross-filesystem fallback path without arranging real cross-mount state.
- An injectable capability for creating the decline marker file at a given path. Supplied to the module, not performed directly, so tests can verify the decline branch without writing to the real home directory.
- An injectable capability for reading one line of user input. Supplied to the module, not performed directly, so tests can drive the `y`, `Y`, `N`, empty, and arbitrary-other-text branches without an interactive terminal.
- An injectable capability for writing prompt and confirmation text to the user-facing output channel. Supplied to the module, not performed directly, so tests can capture and assert the exact text presented.
- An injectable capability for asking whether the input channel is currently attached to an interactive terminal. Supplied to the module, not performed directly, so tests can drive both interactive and non-interactive branches deterministically.

The exact mechanism by which these capabilities are injected — separately passed callables, a single grouped value, or a shared abstraction — is delegated to contract derivation. The structural commitment here is only that all eight are parameters, not ambient access. This satisfies AC8.

**Outputs:**

- A single value of shape `MigrationOutcome` (defined under "Data shapes") describing what the module did: skipped (and why), prompted-and-declined, prompted-and-accepted (with summary of what moved), or short-circuited by an existing marker.

**Side effects:**

- When the offer is warranted and the user accepts: the legacy directory tree is moved to the new location via the injected move capability, and a confirmation line is written to the output channel via the injected write capability.
- When the offer is warranted and the user declines (or the prompt completes with default-decline input): a zero-content marker file is created inside the legacy directory via the injected marker-creation capability, and no further side effect occurs.
- When the offer is warranted but standard input is non-interactive: no prompt is conducted, no marker is written, no move is performed. The module returns a skipped outcome distinguishing this case from "marker already present".
- When the notice is absent or is `DeprecatedEnvVar`: no I/O of any kind. Return is a skipped outcome.
- When the marker file already exists at the documented location: no I/O beyond the existence probe. Return is a short-circuit outcome.

All side effects are mediated by injected capabilities. With test doubles, the module performs no real-world side effect.

**Error semantics:**

- If the injected directory-statistics capability signals failure while computing the summary, the module reports a skipped outcome carrying the underlying failure description and does not prompt. Rationale: presenting a prompt with missing or wrong figures would mislead the user into accepting a migration whose scope was misrepresented.
- If the injected move capability signals failure during accept, the module reports an accept-failed outcome carrying the underlying failure description, does NOT write the decline marker (the user did not decline), and does NOT print the success confirmation. Partial-move state on disk is the move capability's concern; the structural guarantee here is that the module reports the failure to the caller rather than swallowing it.
- If the injected marker-creation capability signals failure during decline, the module reports a decline-marker-failed outcome carrying the underlying failure description. The user is not re-prompted in this same invocation; whether the prompt re-fires next invocation depends on whether the marker actually got created, which is the marker-creation capability's concern.
- If the injected input-read capability signals failure (e.g. unexpected end-of-stream), the module treats it as an empty-input default-decline and proceeds with the decline branch. Rationale: brief AC2 mandates default-no, and a failed read is operationally indistinguishable from a user who pressed Return.
- The module never panics. All failure surfaces are reported in the returned `MigrationOutcome`.

**Behavioural guarantees:**

- **Trigger predicate**: the module performs any user-visible action only when ALL of the following hold simultaneously: (a) the input `ResolvedBaseDir` carries a `DeprecatedDefaultPath` notice; (b) the decline-marker file does not exist at the documented location; (c) standard input is interactive. If any predicate fails, the module returns a skipped or short-circuit outcome with no I/O beyond the existence probe for the marker.
- **Default-decline**: the prompt accepts only the exact strings `y` and `Y` as accept; any other input — including empty input, `N`, `n`, whitespace, or arbitrary text — is treated as decline. This satisfies AC2.
- **Atomicity of the move**: the module's contract requires the injected move capability to attempt a single-syscall same-filesystem rename first, and on failure-due-to-cross-filesystem-condition (and only that condition) fall back to a copy-then-delete sequence. Same-filesystem failures of any other kind propagate as accept-failed without copy-then-delete. The structural commitment is that the move is observable from outside the process as either "fully at the source" or "fully at the destination" by the time the module returns success; intermediate visibility is permitted only during the copy-then-delete fallback path. (The Rust standard library's same-filesystem rename is the file-move primitive intended; cross-device condition detection is the operating system's `EXDEV` error or its standard-library equivalent. Naming the exact stdlib symbol is contract-derivation's job, not this design's.)
- **Decline-state durability**: once the decline marker exists, subsequent invocations short-circuit silently — no prompt, no probe of file count or size, no read of user input. This holds across processes and across reboots; the marker lives on disk inside the legacy directory.
- **Decline-state user-removable**: the marker is a single file at a documented path inside the legacy directory. If the user deletes it manually, the next `arai init` re-fires the prompt. The module does not "remember" decline state by any other means (no separate config, no environment variable, no entry inside `~/.taniwha/arai/`).
- **Decline-state self-disposal**: the marker lives inside the legacy directory. If the user later deletes `~/.arai/` themselves (without going through this module), the marker disappears with it; on the next invocation the resolver will not return `DeprecatedDefaultPath` (because the legacy path no longer exists) and the migration trigger predicate fails. This eliminates orphan-marker drift without explicit cleanup logic.
- **Idempotency of decline**: invoking the decline branch twice in a row is equivalent to invoking it once. The marker-creation capability is contracted to be idempotent — creating a marker that already exists is a no-op success. This is also what makes the short-circuit branch correct: the second invocation sees the marker and short-circuits before reaching the prompt at all.
- **No-op safety**: invoking the module on a system where the migration is not warranted (no notice, or `DeprecatedEnvVar`, or marker present, or non-interactive stdin) produces no user-visible output and no filesystem mutation. This matters for AC7 — non-`arai init` commands never invoke this module, but even if they did by accident, the module's own predicates would prevent unwanted prompting. Defence in depth.
- **No re-emission of the deprecation warning**: the cycle-#73 caller-site warning has already been emitted by `Config::load` before this module runs. The module does not re-emit it. The prompt summary text stands on its own as a migration offer, not as a re-statement of the deprecation.

**Dependencies:** None on other modules in the system. The module consumes a `ResolvedBaseDir` value as an input (the value flows in from the caller; the module does not call `base-directory-resolution` itself). All world-touching capabilities are injected.

## Caller-site change in `arai init`

The `arai init` command's entry point in `src/main.rs` is modified to call this module **once**, **after** `Config::load` returns successfully and **after** any other init-flow steps that must run regardless of migration outcome. The change is mechanical:

1. Receive the `Config` (or equivalent) value from `Config::load`. This value already carries — or has access to — the `ResolvedBaseDir` whose notice the cycle-#73 shim consulted. Whether this surface needs a small additive plumbing change to thread the `ResolvedBaseDir` (or just its notice) into the migration call is contract-derivation's concern; the structural commitment is that the call site is `cmd_init` and the input is "the same notice the shim just warned about, if any".
2. Construct the eight injected dependencies from the live filesystem, the live standard input, the live standard output, and the live tty-detection facility. The decline-marker path is the legacy base directory joined with `.migration_declined` (the documented filename per AC4).
3. Invoke the module. Receive a `MigrationOutcome`.
4. The `MigrationOutcome` is informational at the entry-point level: `arai init` does not branch its subsequent behaviour on it. (If the move succeeded, the next steps of `arai init` will naturally operate against the new path — but `arai init` reads the path from the `ResolvedBaseDir` it already has, which still points at the legacy path for this invocation. Whether `arai init` re-resolves after a successful move, or simply lets the next invocation pick up the new path, is a question for contract derivation. The brief does not require re-resolution within the same invocation, and the cleanest reading is that `arai init` documents "run me again" on a successful migration. See "Open questions" below.)

No other command's entry point is changed. No other call site invokes this module. This is what enforces AC7 structurally: the only way the module gets called is through `cmd_init`.

## Open questions

The following question is genuinely peripheral — it does not affect module shape, contract, or data shape, only the user-visible step ordering inside `arai init` after a successful migration. Surfacing it here rather than burying a silent decision.

- **Re-resolution within the same invocation after a successful move.** After the user accepts and the legacy directory has been moved to the new location, should `arai init` re-run `Config::load` (so the rest of the init flow operates against the new path in the same invocation) or should it print a "re-run `arai init`" hint and exit? The brief is silent. The minimal-surprise reading is to print the hint and exit cleanly, since the rest of the init flow may already have made decisions against the legacy path before the prompt fired. requires_user_decision: true.

## Data shapes

### `ResolvedBaseDir` (consumed, not defined here)

Defined in cycle #73's `design/v1.md` and the `base-directory-resolution/v1` contract. This module consumes it as an input. Of particular relevance: the optional `deprecation_notice` field, whose `DeprecatedDefaultPath` variant is the trigger for this module's behaviour. No other field of `ResolvedBaseDir` is read by this module beyond what is needed to compute the source and destination paths for the move (the source path is the chosen path itself when the notice is `DeprecatedDefaultPath`; the destination path is the new default location, which is a constant known to `Config::load` and either passed in or recomputed at the call site).

### `MigrationOutcome`

The single value returned by the module. A discriminated value with these mutually-exclusive variants:

- **skipped-no-notice**: the input notice was absent. No I/O performed.
- **skipped-env-var-notice**: the input notice was `DeprecatedEnvVar`. No I/O performed.
- **skipped-marker-present**: the decline marker was found at the documented location. The existence probe was the only I/O performed.
- **skipped-non-interactive**: the trigger predicate's other conditions held but standard input was not interactive. No prompt, no marker write, no move.
- **skipped-summary-failed**: directory-statistics computation signalled failure; the module did not prompt. Carries the underlying failure description.
- **prompted-declined**: prompt was conducted, user input was empty or any non-`y`/`Y` text, decline marker was successfully created.
- **prompted-decline-marker-failed**: prompt was conducted, user declined, but the marker write failed. Carries the underlying failure description.
- **prompted-accepted**: prompt was conducted, user input was `y` or `Y`, move succeeded. Carries the file count and total byte size that were moved (the same figures shown in the prompt) for the entry point to use in any post-migration messaging.
- **prompted-accept-failed**: prompt was conducted, user accepted, move signalled failure. Carries the underlying failure description. No marker is written in this case.

Variants are mutually exclusive. No variant carries an unbounded payload; the worst case is a small failure-description string borrowed from the relevant injected capability.

### `MigrationSummary`

The figures shown in the prompt and echoed in the `prompted-accepted` outcome:

- **source path**: the legacy directory being offered for move.
- **destination path**: the new default location.
- **file count**: total number of regular files under the legacy directory.
- **total size**: total byte size of those files.

A pre-formatted human-readable string is not part of this shape; the prompt-rendering belongs inside the module and uses these fields to compose its output. This keeps the summary independently inspectable in tests.

## Out of scope

- ANY change to `resolve_base_dir`, `ResolvedBaseDir`, or `DeprecationNotice`. These are stable from cycle #73 and reused verbatim.
- ANY change to other commands (`arai status`, `arai why`, `arai guardrails`, `arai hooks`, etc.). Only `arai init` calls this module.
- ANY change to the cycle-#73 caller-site warning emission inside `Config::load`. The warning continues to fire on every invocation regardless of migration choice or decline-marker state.
- A "force re-prompt" or "reset decline" CLI subcommand. Manually deleting the marker file is the documented v1 mechanism.
- Repo-local `.arai/` migration. The resolver only looks at user-global; this module mirrors that scope.
- Any change to the internal layout of `~/.taniwha/arai/`.
- Any change to documentation outside the prompt and confirmation strings the module itself produces.
- Persisting the decline state anywhere other than the marker file. No config-file entry, no environment variable, no entry inside the new base directory.
- Telemetry, analytics, or audit-log emission for migration events. The module produces no events on either of Arai's two existing observability channels (`telemetry.rs`, `audit.rs`). If such emission is wanted later, it is additive.
- Running migration on any command other than `arai init`, including a future `arai migrate` subcommand. (If a user later requests an explicit `arai migrate` command, this module's capability is reusable, but defining that command is a separate ticket.)
- Re-resolving `Config::load` within the same invocation after a successful move. See "Open questions".
- Any handling for partial-state recovery if the move fails midway. The injected move capability owns those semantics; this module reports the failure verbatim.

## Test surface

The test surface lives entirely against the module's pure entry point and exercises it through the eight injected dependencies — never against the live environment, live filesystem, live stdin, or live stdout. AC8 requires precisely this. The following coverage is required to satisfy the brief's acceptance criteria.

- **AC1 (prompt fires on `DeprecatedDefaultPath` + `arai init`)**: with the input notice set to `DeprecatedDefaultPath`, the marker-existence probe returning false, the input channel reported as interactive, and a synthetic directory-statistics result of (file_count = N, total_size = S), the module emits a prompt to the captured output channel containing the source path, destination path, N, and S, then reads from the input channel. Verified by inspecting the captured output text and by observing the input read.
- **AC2 (default-no)**: with all trigger predicates true, drive the input channel with each of the strings (empty, `N`, `n`, `no`, `garbage`, ` y` with leading space, `yes`, single-newline). For each, the resulting outcome is `prompted-declined`, the move capability is never invoked, and the marker-creation capability is invoked exactly once.
- **AC2 accept paths**: drive the input channel with each of `y` and `Y`. For each, the resulting outcome is `prompted-accepted`, the move capability is invoked exactly once with (source = legacy path, destination = new default path), the marker-creation capability is never invoked, and the captured output channel contains the post-migration confirmation line.
- **AC3 (move on accept)**: with the move capability instrumented to record its invocation arguments and to return success, the outcome is `prompted-accepted` carrying the file count and total size that were shown in the prompt. With the move capability instrumented to return failure, the outcome is `prompted-accept-failed` carrying the underlying failure description; the marker-creation capability is never invoked; no confirmation line is written.
- **AC4 (decline writes marker)**: with the decline branch reached, the marker-creation capability is invoked exactly once with the path `<legacy_base_dir>/.migration_declined` and the documented zero-content payload. Verified by inspecting the recorded invocation arguments.
- **AC5 (marker short-circuits)**: with the marker-existence probe returning true, regardless of the values of every other input, the outcome is `skipped-marker-present`, and none of {directory-statistics, input-read, output-write, marker-creation, move} is invoked. Verified by asserting the corresponding test doubles recorded zero invocations.
- **AC6 (no notice / `DeprecatedEnvVar` does not prompt)**: with the input notice absent, the outcome is `skipped-no-notice` and no capability beyond — at most — the marker-existence probe is invoked. With the input notice set to `DeprecatedEnvVar`, the outcome is `skipped-env-var-notice` and likewise no further capability is invoked. (Whether the marker-existence probe is consulted in these branches is an implementation choice for contract derivation; the structural commitment is that no prompt is conducted and no migration occurs.)
- **AC7 (non-`arai init` commands never prompt)**: structurally satisfied by the entry-point change being scoped to `cmd_init`. A unit test of the module itself cannot exercise this — the predicate "the calling command is `arai init`" lives in the entry point, not in the module. A small integration test or a CLI-level test asserting that `arai status` and `arai why` do not invoke any of the migration module's injected capabilities (e.g. by running them in a context where invoking those capabilities would be observable) is sufficient. Alternatively, a code-level test asserting that the only call site of the module's entry point is `cmd_init` covers the same ground.
- **AC8 (testable without real fs)**: structurally satisfied by all eight world-touching capabilities being injected parameters. Test bodies must not write to the real filesystem, must not read from the real standard input, must not write to the real standard output, and must not interrogate the real terminal. Capturing this property is a quality-check on the test code itself, not a separate test case.
- **AC9 (`cargo test` passes; existing 287 tests preserved)**: build-level guarantee satisfied by (a) all new tests being unit tests against the pure module entry point, which cannot regress existing tests; (b) the entry-point change in `cmd_init` being mechanically minimal and additive; (c) no public API surface of any other module changing.
- **Non-interactive stdin**: with all other trigger predicates true but the interactive-terminal probe reporting non-interactive, the outcome is `skipped-non-interactive` and none of {directory-statistics, input-read, output-write, marker-creation, move} is invoked. Distinct from `skipped-marker-present` so the entry point can distinguish "user has answered before" from "no human is here to answer".
- **Statistics-failure short-circuit**: with the directory-statistics capability returning failure, the outcome is `skipped-summary-failed` and none of {input-read, output-write, marker-creation, move} is invoked. The user is never shown a prompt with missing or zero figures.
- **Determinism**: repeated invocations with the same injected results yield identical outcomes and identical sequences of capability invocations.
- **No ambient access**: tests run in arbitrary process-environment, filesystem, stdin, and stdout states without affecting their outcomes. This is satisfied structurally by the module taking all capabilities as parameters; the test bodies must not set environment variables, must not create directories, must not pipe to stdin, and must not redirect stdout.

The cycle-#73 caller-site behaviour (the deprecation warning emission inside `Config::load`) is unchanged and continues to be covered by the existing tests from that cycle.