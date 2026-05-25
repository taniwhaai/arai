# Manifest: legacy-path-migration

## Responsibility

Decides whether a migration offer is warranted on a given `arai init` invocation,
conducts the prompt UX when warranted, performs the directory move on accept, and
records the decline state on decline — all through injected world-touching
capabilities so the module itself has no ambient I/O access.

## Not responsible for

Resolving the base directory (that is `base-directory-resolution`), emitting the
deprecation warning that fires inside `Config::load` (preserved unchanged from
cycle #73), gating itself on whether the calling command is `arai init` (that is the
entry point's responsibility), providing a force-re-prompt or reset-decline mechanism
(out of scope), handling partial-move recovery (the injected move capability owns those
semantics), or emitting telemetry, audit-log entries, or analytics for migration events.

## Inputs

### Value input

- **`resolved`** (`ResolvedBaseDir`, required): the value previously produced by
  `base-directory-resolution` and threaded through `Config::load` into the `cmd_init`
  call site. The module reads only two fields of this value:

  - `resolved.path` — used as the migration source path when the notice is
    `DeprecatedDefaultPath`. This is `~/.arai` in the common case.
  - `resolved.notice` — the optional `DeprecationNotice`. The module triggers only
    when this field is present and holds the `DeprecatedDefaultPath` variant.

  When `resolved.notice` is absent, or holds `DeprecatedEnvVar`, the module returns
  immediately without consulting any injected capability.

  `ResolvedBaseDir` is defined in full in the stable `base-directory-resolution/v1`
  contract (cycle #73). This module consumes it as a read-only value; it does not
  call `base-directory-resolution`.

### Injected capability inputs

All eight capabilities below are required parameters of the module's public entry
point. No capability is accessed by any means other than the supplied parameter. A
test double may be substituted for any or all of them without restriction.

The grouped type for all eight capabilities is named `MigrationCapabilities` (see
"Referenced data shapes"). The entry point accepts a single `MigrationCapabilities`
value rather than eight separate parameters; this is the only grouping choice; no
other grouping is permitted.

---

**`path_exists`** — path-existence probe.

Rust type: `fn(&str) -> bool`

The module invokes this with a filesystem path string. The callable returns `true`
if the path currently exists (as any filesystem object), `false` otherwise. The
module uses this to check for the decline-marker file. It is invoked at most once per
`offer_migration` call, only when the notice is `DeprecatedDefaultPath`.

---

**`dir_stats`** — directory-statistics probe.

Rust type: `fn(&str) -> Result<MigrationSummaryStats, String>`

The module invokes this with the source directory path. The callable returns
`Ok(MigrationSummaryStats)` on success or `Err(description)` on failure.
`MigrationSummaryStats` is defined under "Referenced data shapes". On failure, the
module returns `MigrationOutcome::SkippedSummaryFailed` and performs no further I/O.

---

**`move_dir`** — directory move.

Rust type: `fn(&str, &str) -> Result<(), String>`

The module invokes this with `(source_path, destination_path)`. The callable is
contracted to attempt a single-syscall same-filesystem rename first (Rust standard
library's `std::fs::rename`). On failure with error kind
`std::io::ErrorKind::CrossesDevices` (available in Rust >= 1.85, present in this
project's toolchain at cargo 1.94.1 / rustc >= 1.75+1.85 constraint), the callable
falls back to a recursive copy-then-delete sequence. On same-filesystem failure of any
other kind, the callable propagates the failure as `Err(description)` without
attempting copy-then-delete.

The cross-device condition is detected via `std::io::ErrorKind::CrossesDevices`.
Checking raw `errno` values is not required and must not be done in the callable
implementation; the standard library variant is the canonical detection point.

On success, the full directory tree that was at `source_path` is now at
`destination_path` and `source_path` no longer exists. The observable state is either
"fully at source" or "fully at destination" from the module's perspective; intermediate
visibility during copy-then-delete is an acceptable transient state the callable does
not need to mask.

On failure from the module's perspective: the module returns
`MigrationOutcome::PromptedAcceptFailed` carrying the `Err` string. The module does
not write the decline marker in this case. The module does not re-attempt.

---

**`create_marker`** — decline-marker creation.

Rust type: `fn(&str) -> Result<(), String>`

The module invokes this with the decline-marker path (always
`<legacy_base_dir>/.migration_declined`, where `<legacy_base_dir>` is
`resolved.path`). The callable creates a zero-content file at that path. If the file
already exists, the callable returns `Ok(())` (idempotent). On failure, the callable
returns `Err(description)`.

On success: the module returns `MigrationOutcome::PromptedDeclined`.
On failure: the module returns `MigrationOutcome::PromptedDeclineMarkerFailed`
carrying the `Err` string.

---

**`read_line`** — one-line user-input read.

Rust type: `fn() -> Result<String, String>`

The module invokes this at most once per `offer_migration` call, only after the full
prompt text has been written to the output channel. The callable returns `Ok(line)`
where `line` is the raw text the user entered (may include a trailing newline, which
the module strips before comparison). On failure (e.g. unexpected end-of-stream), the
callable returns `Err(_)`; the module treats this as empty input and proceeds with
the decline branch. Rationale: AC2 mandates default-no, and a failed read is
operationally indistinguishable from a user who pressed Return with no input.

---

**`write_output`** — user-facing output write.

Rust type: `fn(&str)`

The module invokes this to emit the prompt text (source path, destination path, file
count, total byte size, and the decision question) and, on a successful accept, the
post-migration confirmation line. The callable receives a complete string each time;
whether it appends a trailing newline is the implementor's choice as long as the
output is human-readable. The callable is not used for any purpose other than
user-facing prompt and confirmation text. It is never invoked on the skip or
marker-short-circuit paths.

The prompt summary format is: the module must include source path, destination path,
file count, and total byte size in whatever natural prose form makes the user-facing
question legible. The exact wording is the implementor's choice.

The post-migration confirmation line must include the phrase `"Run 'arai init' again"`
(or an equivalent instruction that tells the user to re-invoke `arai init`) so users
understand the current invocation ends here.

---

**`is_interactive`** — interactive-terminal probe.

Rust type: `fn() -> bool`

The module invokes this at most once per `offer_migration` call. It returns `true` if
the input channel is currently attached to an interactive terminal, `false` otherwise.
The module only reaches this callable when `resolved.notice` is `DeprecatedDefaultPath`
and the decline marker does not exist. If this returns `false`, the module returns
`MigrationOutcome::SkippedNonInteractive` with no further I/O.

---

### Decline-marker path

The decline-marker path is always:

```
<resolved.path>/.migration_declined
```

where `<resolved.path>` is the `path` field of the `ResolvedBaseDir` input when the
notice is `DeprecatedDefaultPath` (i.e. the legacy `~/.arai` directory). The filename
is `.migration_declined`. No other path, no other filename, no other parent directory.
This path is used both for the existence probe (via `path_exists`) and for marker
creation (via `create_marker`).

### Migration destination path

The destination path for the move is always the Arai new-default location:
`~/.taniwha/arai`. This is a constant known at the call site in `src/main.rs` and
passed into `offer_migration` (or recomputed inline from the home directory). It is
NOT a parameter of `MigrationCapabilities`; it is a plain text-string parameter of
`offer_migration` alongside `resolved` and `capabilities`.

Specifically, the public entry point signature is (expressed in language-neutral terms,
with the Rust realisation below):

- `resolved`: the `ResolvedBaseDir` value
- `dest_path`: the new canonical base directory path string
- `capabilities`: the `MigrationCapabilities` grouped value

Rust entry-point signature:

```rust
pub fn offer_migration(
    resolved: &ResolvedBaseDir,
    dest_path: &str,
    capabilities: MigrationCapabilities,
) -> MigrationOutcome
```

This function is the module's sole public entry point. All other functions in
`src/legacy_path_migration.rs` are private (`pub(crate)` or `pub` is not permitted
on helpers; the module's public surface is exactly `offer_migration`,
`MigrationOutcome`, `MigrationSummaryStats`, and `MigrationCapabilities`).

## Outputs

- **`MigrationOutcome`** (`MigrationOutcome`, always present): a discriminated value
  describing what the module did. Every call returns exactly one variant. Defined in
  full under "Referenced data shapes".

## Side effects

All side effects are mediated by injected capabilities. With test doubles substituted,
the module has zero real-world side effects.

**On accept (user input is `"y"` or `"Y"` after stripping trailing whitespace/newline):**
- `move_dir(resolved.path, dest_path)` is called exactly once.
- On `move_dir` success: `write_output(confirmation_line)` is called exactly once.
- On `move_dir` failure: no further capability is invoked; no marker is written.

**On decline (any other input, including empty, failed read):**
- `create_marker(decline_marker_path)` is called exactly once.

**On skipped-marker-present:**
- `path_exists(decline_marker_path)` was called (returning true); no other
  capability is invoked.

**On skipped-non-interactive:**
- `path_exists(decline_marker_path)` returned false; `is_interactive()` returned
  false; no other capability is invoked.

**On skipped-no-notice or skipped-env-var-notice:**
- No capability of any kind is invoked.

**On skipped-summary-failed:**
- `path_exists` returned false; `is_interactive()` returned true; `dir_stats` was
  called and failed; no further capability is invoked.

## Error semantics

- **`dir_stats` signals failure**: the module returns `MigrationOutcome::SkippedSummaryFailed(description)`. No prompt is written, no input is read, no marker is created, no move is attempted. Caller obligation: log or surface the description; it is informational.

- **`move_dir` signals failure**: the module returns `MigrationOutcome::PromptedAcceptFailed(description)`. The decline marker is NOT written. The post-migration confirmation is NOT written. Partial-move state on disk is the `move_dir` callable's concern; this module reports the failure string verbatim and does not retry. Caller obligation: surface the failure to the user; do not treat this as a decline.

- **`create_marker` signals failure**: the module returns `MigrationOutcome::PromptedDeclineMarkerFailed(description)`. The user is not re-prompted in this invocation. Whether the prompt re-fires on the next invocation depends on whether the marker was actually created on disk; this module does not know and does not retry. Caller obligation: the failure is informational; no mandatory action.

- **`read_line` signals failure**: the module treats this as empty input and proceeds with the decline branch (calls `create_marker`, returns `MigrationOutcome::PromptedDeclined` or `MigrationOutcome::PromptedDeclineMarkerFailed`). The `read_line` failure string is not surfaced in the outcome. Rationale: AC2 mandates default-no; a failed read is operationally indistinguishable from an empty-input decline.

- **`path_exists` signals failure**: not applicable. The callable's Rust type is `fn(&str) -> bool`, which cannot signal failure. An existence-probe that cannot determine presence returns `false`, and the module proceeds as if the marker is absent. If the project context ever demands a fallible existence probe, the callable's type must be amended and this contract updated.

- **`write_output` signals failure**: not applicable. The callable's Rust type is `fn(&str)`, which cannot signal failure. Output errors are silently swallowed at the capability boundary. This is intentional: a prompt-output failure is not a reason to abort the migration operation itself.

- **`is_interactive` signals failure**: not applicable. The callable's Rust type is `fn() -> bool`. A callable that cannot determine TTY state returns `false`, causing the module to short-circuit to `SkippedNonInteractive`. This is the safe default (no prompt without confirmed interactivity).

- **The module never terminates abnormally.** All failure surfaces are reported in the returned `MigrationOutcome`. The module contains no unconditional termination, no unconditional abort, and no unrecoverable assertion in library code. The `#[cfg(test)]` test module may use assertion macros freely.

## Behavioural guarantees

- **Trigger predicate — all three must hold for any user-visible action:**
  1. `resolved.notice` is `DeprecationNotice::DeprecatedDefaultPath`.
  2. `path_exists(decline_marker_path)` returns `false`.
  3. `is_interactive()` returns `true`.
  
  If predicate 1 fails: return `SkippedNoNotice` or `SkippedEnvVarNotice` immediately;
  no capability is invoked.
  If predicate 2 fails: return `SkippedMarkerPresent`; only `path_exists` was invoked.
  If predicate 3 fails: return `SkippedNonInteractive`; `path_exists` and
  `is_interactive` were invoked.

- **Evaluation order — short-circuit, fixed:**
  1. Check notice variant (no I/O).
  2. Probe marker existence via `path_exists`.
  3. Probe TTY via `is_interactive`.
  4. Collect directory statistics via `dir_stats`.
  5. Write prompt text via `write_output`.
  6. Read user input via `read_line`.
  7. Act on input (move or create marker), write confirmation if applicable.
  
  Later steps are not reached if an earlier step causes a short-circuit return.

- **Default-decline:** the module accepts only the exact strings `"y"` and `"Y"` (after
  stripping a single trailing newline `'\n'` and a single trailing carriage-return
  `'\r'` from the raw `read_line` result) as accept. Any other result — including
  empty string, `"N"`, `"n"`, `"no"`, `"yes"`, whitespace-prefixed variants, or
  arbitrary text — is treated as decline. This satisfies AC2.

- **Idempotency:** the module's behaviour given identical inputs (same `resolved`,
  same `dest_path`, same capability responses) is identical on every call. The module
  holds no mutable state between calls. Whether a second call short-circuits or
  re-prompts depends entirely on the capability responses, not on any internal state.

- **Atomicity:** the module's atomicity guarantee extends only as far as its sequencing
  of capability calls. It does not guarantee filesystem atomicity; that is the move
  callable's concern. From the module's perspective: if `move_dir` returns `Ok(())`,
  the module reports `PromptedAccepted`; if `move_dir` returns `Err`, the module
  reports `PromptedAcceptFailed` and does not write the marker. There is no partial
  outcome at the `MigrationOutcome` level.

- **Ordering — capability invocation order is deterministic:** given the same inputs,
  the sequence of capability invocations is always the same. Specifically: the module
  never invokes `dir_stats`, `read_line`, `write_output`, `move_dir`, or
  `create_marker` before completing the trigger predicate check; and it never invokes
  `move_dir` and `create_marker` in the same call (they are branches of a conditional).

- **Concurrency:** the module holds no shared mutable state. Concurrent invocations
  of `offer_migration` with independent inputs do not interfere with each other via
  the module's own state. Whether concurrent invocations are safe in practice depends
  on whether the supplied capability callables are safe under concurrent invocation;
  the module imposes no additional constraint.

- **Resource bounds:** the module invokes each capability at most once per call
  (except `write_output`, which may be invoked up to twice: once for the prompt, once
  for the confirmation). It allocates memory bounded by the size of the path strings,
  the `MigrationSummaryStats` value, and the line read from `read_line`. It performs
  no unbounded iteration or accumulation of its own.

- **No re-emission of the deprecation warning:** the deprecation warning emitted by
  `Config::load`'s caller-site change (cycle #73) fires before `offer_migration` is
  called. This module does not re-emit that warning. The prompt text is a migration
  offer, not a restatement of the deprecation.

- **Decline-state durability:** the decline marker lives on disk at
  `<resolved.path>/.migration_declined`. Its presence persists across process
  boundaries and reboots. The module does not maintain decline state by any other
  means.

- **Decline-state user-removable:** the marker is a single file at the documented
  path. If the user deletes it manually, the next `arai init` invocation re-fires the
  prompt (assuming the legacy path still exists and triggers `DeprecatedDefaultPath`).

- **Decline-state self-disposal:** the marker lives inside the legacy directory. If
  the user deletes `~/.arai/` manually (without going through this module), the marker
  disappears with it; on the next invocation, the resolver will not return
  `DeprecatedDefaultPath` (because the legacy path no longer exists), so predicate 1
  of the trigger check fails and the module short-circuits to `SkippedNoNotice`.

## Dependencies

None on other modules in the system. `base-directory-resolution` is NOT called by
this module. The `ResolvedBaseDir` value is supplied by the caller (`cmd_init` in
`src/main.rs`) as a plain value input. All world-touching capabilities are injected
as parameters.

## Caller-site change in `src/main.rs` (`cmd_init`)

This section is normative for the `src/main.rs` implementor. It specifies the exact
mechanical change required at the `cmd_init` entry point.

### Plumbing: threading `ResolvedBaseDir` into `cmd_init`

`Config::load` (as modified in cycle #73) already consumes a `ResolvedBaseDir` value
internally. The `Config` value returned by `Config::load` must expose (or provide
access to) the `ResolvedBaseDir` that was used during load, so `cmd_init` can pass it
to `offer_migration`.

If `Config` already carries the `ResolvedBaseDir` as a field (or a reference to it),
no further plumbing is required. If it does not, a small additive change to `Config`
is required: add a `resolved_base_dir: ResolvedBaseDir` field (or equivalent) that
`Config::load` populates before returning. This change is additive and does not alter
any existing field or public function signature.

The destination path for the migration (`dest_path` parameter of `offer_migration`)
is the new canonical default: `~/.taniwha/arai`. At the call site, this string is
constructed from the user's home directory using the same logic `base-directory-
resolution` already uses (i.e., `format!("{}/.taniwha/arai", home_dir)`). This value
is not stored in `Config`; it is constructed inline at the call site.

### Control flow at `cmd_init` per `MigrationOutcome` variant

After `Config::load` returns and after any init-flow steps that must run regardless of
migration outcome, `cmd_init` calls `offer_migration` with:
- the `ResolvedBaseDir` threaded from `Config`
- the constructed `dest_path` string
- a `MigrationCapabilities` built from live filesystem, live standard input, live
  standard output, and live TTY-detection (see "Live capability construction" below)

`cmd_init` then branches on `MigrationOutcome` exactly as follows:

| Variant | `cmd_init` action |
|---|---|
| `SkippedNoNotice` | Continue normal `arai init` flow; no output from migration. |
| `SkippedEnvVarNotice` | Continue normal `arai init` flow; no output from migration. |
| `SkippedMarkerPresent` | Continue normal `arai init` flow; no output from migration. |
| `SkippedNonInteractive` | Continue normal `arai init` flow; no output from migration. |
| `SkippedSummaryFailed(desc)` | Continue normal `arai init` flow; optionally emit a non-fatal warning to stderr with `desc`. The exact policy (warn vs. silent) is the implementor's choice; `cmd_init` must not abort. |
| `PromptedDeclined` | Continue normal `arai init` flow; no additional output from `cmd_init`. |
| `PromptedDeclineMarkerFailed(desc)` | Continue normal `arai init` flow; optionally emit a non-fatal warning to stderr with `desc`. Must not abort. |
| `PromptedAccepted { .. }` | Print the confirmation hint (see below) and exit with code 0. Do NOT call `Config::load` again. Do NOT continue the normal `arai init` flow. |
| `PromptedAcceptFailed(desc)` | Print an error message to stderr including `desc` and exit with a non-zero code. Do NOT write the decline marker (the module already did not). |

The confirmation hint text for `PromptedAccepted` must be substantially:

> `"Migration complete. Run 'arai init' again to finish initialisation."`

The exact phrasing may vary slightly; the structural requirement is that:
1. It confirms migration completed.
2. It instructs the user to run `arai init` again.

Exit code for `PromptedAccepted` is 0. Exit code for `PromptedAcceptFailed` is
non-zero (the existing convention for `cmd_init` failure applies).

### Live capability construction

At the `cmd_init` call site, `MigrationCapabilities` is constructed with:
- `path_exists`: wraps `std::path::Path::new(p).exists()`
- `dir_stats`: walks the directory tree with standard library directory iteration,
  counting regular files and summing their sizes; returns `Ok` or `Err(description)`.
  The exact implementation is the implementor's choice; no new crate dependency is
  introduced.
- `move_dir`: wraps `std::fs::rename` with `CrossesDevices` fallback as specified
  above. The exact copy-then-delete implementation is the implementor's choice; no
  new crate dependency is introduced.
- `create_marker`: wraps `std::fs::File::create(path).map(|_| ())` or equivalent.
  Creating an already-existing file is `Ok(())`.
- `read_line`: wraps `std::io::stdin().lock().lines().next()` or equivalent, returning
  `Ok(line)` or `Err(description)`.
- `write_output`: wraps `println!` or `print!` to standard output.
- `is_interactive`: wraps `atty::is(atty::Stream::Stdin)` — however, `atty` is a
  dependency already available in the crate. If `atty` is not already a dependency,
  the implementor must use an alternative that does not introduce a new crate
  dependency (e.g. check `std::io::stdin().is_terminal()` available in Rust 1.70+
  via `IsTerminal`, which is in scope given the >= 1.75 constraint).

## Referenced data shapes

All shapes below are defined in this module (not imported from elsewhere). Because
this is a `single_module` build, no shared vocabulary file is produced; all types are
defined here inline.

### `MigrationOutcome`

Rust realisation:

```rust
pub enum MigrationOutcome {
    /// resolved.notice was absent.
    SkippedNoNotice,
    /// resolved.notice was DeprecatedEnvVar.
    SkippedEnvVarNotice,
    /// Decline marker already exists at documented path.
    SkippedMarkerPresent,
    /// Trigger predicates 1 and 2 held but stdin was non-interactive.
    SkippedNonInteractive,
    /// dir_stats callable failed; description is the Err string.
    SkippedSummaryFailed(String),
    /// User declined (any input other than "y"/"Y"); marker created successfully.
    PromptedDeclined,
    /// User declined; marker creation failed; description is the Err string.
    PromptedDeclineMarkerFailed(String),
    /// User accepted ("y" or "Y"); move succeeded.
    PromptedAccepted {
        file_count: u64,
        total_bytes: u64,
    },
    /// User accepted; move signalled failure; description is the Err string.
    PromptedAcceptFailed(String),
}
```

All variants are mutually exclusive. `PromptedAccepted` carries the file count and
total byte size that were shown in the prompt (sourced from `MigrationSummaryStats`).
`String` payloads are the raw `Err` strings from the failing capability; they are
not re-formatted by this module.

### `MigrationSummaryStats`

Rust realisation:

```rust
pub struct MigrationSummaryStats {
    pub file_count: u64,
    pub total_bytes: u64,
}
```

Returned by the `dir_stats` callable on success. `file_count` is the count of regular
files (not directories, not symlinks) under the source directory tree. `total_bytes`
is the sum of the byte sizes of those files as reported by their metadata. Both fields
are `u64` (sufficient for any realistic home-directory tree; overflow is not a
realistic concern but if it occurs, saturating arithmetic may be used without contract
violation).

### `MigrationCapabilities`

Rust realisation:

```rust
pub struct MigrationCapabilities {
    pub path_exists:    Box<dyn Fn(&str) -> bool>,
    pub dir_stats:      Box<dyn Fn(&str) -> Result<MigrationSummaryStats, String>>,
    pub move_dir:       Box<dyn Fn(&str, &str) -> Result<(), String>>,
    pub create_marker:  Box<dyn Fn(&str) -> Result<(), String>>,
    pub read_line:      Box<dyn Fn() -> Result<String, String>>,
    pub write_output:   Box<dyn Fn(&str)>,
    pub is_interactive: Box<dyn Fn() -> bool>,
}
```

`Box<dyn Fn(...)>` is used throughout so that test doubles (closures capturing test
state) can be substituted without requiring the callables to satisfy `'static` or
`Copy`. The module owns the capabilities value; it is not shared across concurrent
invocations. Implementors must not use `Arc`, `Mutex`, or other shared-ownership
wrappers for these fields; each `offer_migration` call receives its own
`MigrationCapabilities` by value.

The seven fields correspond one-to-one to the seven injected capabilities described
under "Inputs". The field names are the canonical names used throughout this contract.

### `ResolvedBaseDir` and `DeprecationNotice`

Defined in the stable `base-directory-resolution/v1` contract (cycle #73, read-only
reference). Consumed here by value. Not redefined. The `DeprecationNotice` variants
relevant to this module:
- `DeprecatedDefaultPath` — triggers this module's behaviour.
- `DeprecatedEnvVar` — causes `SkippedEnvVarNotice` return.
- Absent — causes `SkippedNoNotice` return.

## Acceptance criteria

The following criteria are objectively verifiable by a verifier holding only this
manifest and the project context. Tests live in `#[cfg(test)] mod tests` inside
`src/legacy_path_migration.rs`. Tests must not access the real filesystem, real
standard input, real standard output, or real TTY. All capability arguments must be
test doubles (closures or function pointers).

### AC1 — Prompt fires on `DeprecatedDefaultPath` + interactive + no marker

Given:
- `resolved.notice` = `DeprecatedDefaultPath`
- `path_exists` returns `false`
- `is_interactive` returns `true`
- `dir_stats` returns `Ok(MigrationSummaryStats { file_count: N, total_bytes: S })`
- `read_line` returns `Ok("N".to_string())`
- `write_output` captures its argument

The `write_output` callable is invoked at least once before `read_line` is invoked.
The captured prompt text contains: the source path (`resolved.path`), the destination
path (`dest_path`), the file count N, and the total byte size S. The outcome is
`PromptedDeclined`.

### AC2 — Default-no: any non-`y`/`Y` input is decline

Given all trigger predicates hold (notice = `DeprecatedDefaultPath`, marker absent,
interactive = true), `dir_stats` returns `Ok`, and `create_marker` returns `Ok`:

For each of the following `read_line` return values:
- `Ok("".to_string())`
- `Ok("N".to_string())`
- `Ok("n".to_string())`
- `Ok("no".to_string())`
- `Ok("garbage".to_string())`
- `Ok(" y".to_string())` (leading space)
- `Ok("yes".to_string())`
- `Ok("\n".to_string())`
- `Err("read error".to_string())`

The outcome is `PromptedDeclined` (or `PromptedDeclineMarkerFailed` if `create_marker`
were configured to fail, but here it succeeds). `move_dir` is never invoked.
`create_marker` is invoked exactly once.

### AC2-accept — Accept paths: only `y` and `Y`

Given the same trigger predicates hold, `dir_stats` returns `Ok`, and `move_dir`
returns `Ok`:

For each of the following `read_line` return values:
- `Ok("y".to_string())`
- `Ok("Y".to_string())`
- `Ok("y\n".to_string())` (trailing newline, stripped before comparison)
- `Ok("Y\n".to_string())`

The outcome is `PromptedAccepted { file_count: N, total_bytes: S }` where N and S
match the `dir_stats` return. `move_dir` is invoked exactly once with
`(resolved.path, dest_path)`. `create_marker` is never invoked. `write_output` is
invoked at least once after `move_dir` returns (the confirmation line).

### AC3 — Move failure produces accept-failed; no marker written

Given all trigger predicates hold, `dir_stats` returns `Ok`, `read_line` returns
`Ok("y".to_string())`, and `move_dir` returns `Err("disk full".to_string())`:

The outcome is `PromptedAcceptFailed("disk full".to_string())`.
`create_marker` is never invoked.
The confirmation write is never emitted (no `write_output` call after `move_dir`'s
failure, though the prompt-text write before `read_line` is still emitted).

### AC4 — Decline writes marker at exact path

Given all trigger predicates hold, `dir_stats` returns `Ok`, `read_line` returns
`Ok("N".to_string())`, and `create_marker` records its invocation arguments:

`create_marker` is invoked exactly once with the path:
`format!("{}/.migration_declined", resolved.path)`

The content written to the marker is zero bytes (the `create_marker` callable
receives only the path; it is the callable's responsibility to write zero content,
not this module's — the module passes the path and expects the callable to create a
zero-content file).

### AC5 — Marker presence short-circuits all other capabilities

Given:
- `resolved.notice` = `DeprecatedDefaultPath`
- `path_exists` returns `true`
- All other capabilities are instrumented to record any invocation

The outcome is `SkippedMarkerPresent`.
`dir_stats`, `is_interactive`, `read_line`, `write_output`, `create_marker`, and
`move_dir` are each invoked zero times.
`path_exists` is invoked exactly once.

### AC6 — No notice / `DeprecatedEnvVar` notice: no capabilities invoked

Sub-case A: `resolved.notice` is absent.
- Outcome is `SkippedNoNotice`.
- No capability is invoked (including `path_exists`).

Sub-case B: `resolved.notice` = `DeprecatedEnvVar`.
- Outcome is `SkippedEnvVarNotice`.
- No capability is invoked (including `path_exists`).

### AC7 — Non-`arai init` commands do not invoke the module

Verified structurally: `offer_migration` is called only from `cmd_init` in
`src/main.rs`. A code-level assertion (grep or module-import check) that no other
function in `src/main.rs` or any other `src/*.rs` file calls `offer_migration` (or
imports `legacy_path_migration`) satisfies this criterion.

### AC8 — All capabilities are injected; no ambient access in tests

Each test body:
- Supplies all seven capabilities as closures or function-pointer values.
- Does not write to the real filesystem.
- Does not read from real standard input.
- Does not write to real standard output.
- Does not set or read process environment variables.
- Does not interrogate the real terminal.

This is a quality check on the test code itself. The verifier confirms compliance by
inspecting the test bodies; no separate test case is needed.

### AC9 — `cargo test` passes; existing 287 tests preserved

After the new module and caller-site change are introduced:
- `cargo test` exits with code 0.
- The count of passing tests is not less than 287 + (count of new tests in
  `src/legacy_path_migration.rs`).
- No previously passing test is removed or skipped.
- No public function in `Config`, `resolve_base_dir`, `ResolvedBaseDir`,
  `DeprecationNotice`, or any other existing module has its signature changed.

### AC-noninteractive — Non-interactive stdin: short-circuit without marker

Given all other trigger predicates hold (notice = `DeprecatedDefaultPath`, marker
absent), but `is_interactive` returns `false`:
- Outcome is `SkippedNonInteractive`.
- `dir_stats`, `read_line`, `write_output`, `create_marker`, and `move_dir` are each
  invoked zero times.
- `path_exists` is invoked exactly once (returning false).
- `is_interactive` is invoked exactly once.

### AC-statsfail — Statistics failure prevents prompt

Given all trigger predicates hold (notice = `DeprecatedDefaultPath`, marker absent,
interactive = true), but `dir_stats` returns `Err("permission denied".to_string())`:
- Outcome is `SkippedSummaryFailed("permission denied".to_string())`.
- `read_line`, `write_output`, `create_marker`, and `move_dir` are each invoked zero
  times.

### AC-determinism — Repeated invocations with identical inputs yield identical results

Given any fixed set of inputs (same `resolved`, same `dest_path`, same capability
responses), calling `offer_migration` twice yields the same `MigrationOutcome` variant
and the same sequence of capability invocations both times. No internal state persists
between calls.

### AC-noambient — Module holds no ambient access

Verified structurally: `src/legacy_path_migration.rs` contains no direct calls to:
- `std::env::*` (environment variable access)
- `std::fs::*` (filesystem operations)
- `std::io::stdin()`, `std::io::stdout()` (I/O streams)
- `atty::*` or `std::io::IsTerminal` (TTY detection)

All such operations occur only in the live capability closures constructed at the
`cmd_init` call site in `src/main.rs`, not inside `src/legacy_path_migration.rs`
itself.
