# Arai base-directory deprecation shim

## Structural tier

**Selected:** single_module

**Justification:** The brief restricts all changes to a single existing source file (`src/config.rs`) inside a single-package repository, adds no cross-module contract, introduces no new shared data shape that other modules consume, and exposes no new public surface. The brief's own tier rationale and the "Out of scope" list both reinforce this: any change beyond the one file is forbidden. Per the design-doc skill, a modification that fits entirely within one existing module's contract is the canonical `single_module` case.

**Module count:** 1.

## Purpose

Keep existing Arai installations working after the v0.2.15 base-directory and environment-variable rename, by transparently honouring the previous defaults while signalling — once, and only to interactive users — that those defaults are deprecated.

## External boundaries

- **Process environment**: inbound. The system reads two named environment variables to discover an operator-supplied base directory: the current name and a deprecated alias.
- **User home directory**: inbound. The system inspects two well-known locations under the user's home directory to detect whether prior state already lives at the new default path, the old default path, both, or neither.
- **Standard error stream**: outbound. The system emits at most one human-readable deprecation warning per resolution. Emission is gated on standard error being attached to an interactive terminal; non-interactive invocations (notably the hot path where this binary runs as a subprocess of another tool on every operation) produce no output on this channel.

## Modules

### base-directory-resolution

**Responsible for:** Deciding which filesystem path the rest of the system should treat as the Arai base directory, and reporting whether that decision warrants a one-shot deprecation notice.

**Not responsible for:** Reading the environment or filesystem itself, emitting the warning, moving or migrating any files, or persisting state. All world-touching behaviour is supplied to it by the caller; all warning emission is performed by the caller. Migration of data from the old default location to the new one belongs to a separate, future workstream and is explicitly out of scope here.

**Inputs:**

- A way to look up an environment variable by name and obtain either its value or an indication that it is unset. This is supplied to the resolver, not performed by it, so that tests can drive every branch without mutating the real process environment.
- A way to test whether a given path currently exists on the filesystem. This is supplied to the resolver, not performed by it, so that tests can drive every branch without creating real directories.
- A handle on the user's home directory (the conceptual root under which the two well-known default locations are constructed). This is supplied to the resolver because home-directory discovery itself is not the resolver's concern, and because tests must be able to vary it freely.

The exact mechanism by which these three capabilities are injected — whether each is a separately passed callable, whether they are bundled into a single grouped value, or whether they are expressed via a shared abstraction the caller satisfies — is delegated to the contract-derivation step. The structural commitment here is only that all three are parameters, not ambient access.

**Outputs:**

- A single value of shape `ResolvedBaseDir` (defined under "Data shapes") containing the chosen filesystem path together with an optional deprecation notice describing why a warning is warranted. The notice is absent in branches where no warning is required.

**Side effects:**

- None. The resolver performs no I/O of its own, mutates no global state, and emits nothing on any output stream. Its only observable behaviour is the value it returns. Warning emission is the caller's responsibility.

**Error semantics:**

- The resolver does not itself originate failures. It receives the results of environment lookups and existence probes as already-evaluated values and consumes them. If the injected dependencies signal a failure of their own (for example, the home-directory handle is unavailable), that failure is propagated to the caller unchanged; the resolver does not invent a fallback. The five-branch resolution order assumes the home-directory handle is available; behaviour when it is not is the caller's concern, not the resolver's.

**Behavioural guarantees:**

- Determinism: given identical inputs (same environment-lookup results, same existence-probe results, same home-directory handle), the resolver returns identical output. There is no hidden state, no time-dependent behaviour, no source of nondeterminism.
- Total ordering of the five branches as stated in the brief: the resolver evaluates them in the documented order and returns on the first match. In particular, an explicit `ARAI_BASE_DIR` value short-circuits all subsequent checks; the deprecated environment variable is consulted only if the current one is unset; existence of the new default path takes precedence over existence of the old default path.
- At most one deprecation notice per call. The notice variants are mutually exclusive: a call returns either no notice, or one notice describing exactly one deprecated condition. The resolver never returns a notice in the three branches that the brief marks as silent.
- Idempotency and purity: repeated calls with the same inputs produce identical results, and the resolver may be called from any context without observable side effects.

**Dependencies:** None. The resolver calls no other module in the system. It receives all capabilities it needs as inputs.

### Caller-site change in the existing configuration-loading module

The existing function in `src/config.rs` that today computes the base directory is modified to delegate that decision to `base-directory-resolution`. The change is mechanical and contained:

1. Construct the three injected dependencies from the live process environment, the live filesystem, and the live home-directory lookup.
2. Invoke the resolver and receive a `ResolvedBaseDir`.
3. If the returned value carries a deprecation notice, and standard error is currently attached to an interactive terminal, write the notice's message to standard error exactly once. If either condition is false, suppress emission silently.
4. Use the returned path as the base directory in the rest of the existing load procedure, exactly where the previous inline computation was used.

No other behaviour of the load function changes. No public field of any returned configuration value changes. No new exported function, type, or environment-variable name is introduced beyond the existing deprecated alias being read.

## Data shapes

### `ResolvedBaseDir`

The single value returned by the resolver. Two conceptual fields:

- **path**: the filesystem path the rest of the system should use as the Arai base directory. Always present.
- **deprecation_notice**: an optional value. When present, it carries enough information for the caller to emit a single human-readable line on standard error explaining why the chosen path is deprecated and, in the case where the chosen path is the legacy default home-directory location, that a forthcoming `arai migrate` command will move it. When absent, no warning is to be emitted.

The notice's internal representation (a discriminated variant identifying which deprecation condition fired, a pre-formatted string, or some other carrier) is delegated to contract-derivation. The structural commitment here is only that the resolver returns exactly one value combining the chosen path with an optional, mutually-exclusive deprecation signal — not a tuple plus a side channel, not two parallel optional fields the caller has to disambiguate.

### Deprecation-notice variants

Two and only two notice variants exist, corresponding to the two warning-emitting branches in the brief's resolution order:

- **deprecated-env-var**: emitted when the deprecated environment variable supplied the path. Message instructs the user to rename the variable.
- **deprecated-default-path**: emitted when the legacy home-directory default supplied the path. Message notes that the location is deprecated and that a migration command is forthcoming.

The other three branches (current env var; new default exists; neither default exists) produce no notice.

## Out of scope

- Moving, copying, or otherwise migrating any files from the legacy default location to the new default location. That is a separate workstream (the `arai migrate` command).
- Any change to documentation, README content, help text outside the warning string itself, or release notes.
- Any change to any source file other than `src/config.rs`.
- Any change to public API surface: no new exported function names beyond the resolver itself, no new struct fields on the existing configuration value, no rename or removal of the deprecated environment variable, no change to the names of returned paths.
- Any refactor, cleanup, or tidy-up of `src/config.rs` beyond the minimum required to introduce the resolver and route the existing load function through it.
- Any change to how the warning is presented other than "single line on standard error, only when standard error is an interactive terminal". No log-framework integration, no severity level, no structured-event emission, no suppression cache across invocations beyond the natural single-emission-per-call behaviour.
- Any handling for the case where the home-directory handle is unavailable beyond what the existing load function already does today; the brief does not request a change to that behaviour and it is preserved unchanged.

## Test surface

The test surface lives entirely against the pure resolver and exercises it through the injected dependencies — never against the live environment or live filesystem. The following coverage is required to satisfy the brief's acceptance criteria.

- **Branch 1 (AC1)**: with the current environment variable set to a path, the deprecated environment variable also set, the new default path probed as existing, and the old default path probed as existing, the resolver returns the path supplied by the current environment variable and no notice. This case must hold regardless of the values of the other inputs, demonstrating that the first branch short-circuits the rest.
- **Branch 2 (AC2)**: with the current environment variable unset and the deprecated environment variable set to a path, the resolver returns that path together with the deprecated-env-var notice. Verified independently of whether the default paths exist.
- **Branch 3 (AC3)**: with both environment variables unset, the new default path probed as existing, and the old default path probed as not existing, the resolver returns the new default path and no notice.
- **Branch 4 (AC4)**: with both environment variables unset, the new default path probed as not existing, and the old default path probed as existing, the resolver returns the old default path together with the deprecated-default-path notice.
- **Branch 5 (AC5)**: with both environment variables unset and neither default path probed as existing, the resolver returns the new default path and no notice.
- **Precedence (AC6)**: with both environment variables unset, the new default path probed as existing, and the old default path also probed as existing, the resolver returns the new default path and no notice. This is the disambiguator between branches 3 and 4 and is called out separately in the brief.
- **Determinism**: repeated invocations with the same inputs yield identical outputs.
- **No ambient access**: tests run in arbitrary process-environment and filesystem states without affecting their outcomes. This is satisfied structurally by the resolver taking all capabilities as parameters; the test bodies must not set environment variables, must not create directories, and must not alter the home-directory of the running process. Satisfaction of this property is what AC7 codifies.

The caller-site TTY-gating behaviour is exercised indirectly: because the resolver is what decides whether a notice exists, and the caller's only added logic is "if notice present and stderr is a TTY, emit it", the structural commitment is that the TTY check sits at the caller site and the notice-presence decision sits in the resolver. Tests against the resolver alone fully cover the notice-presence half; the TTY-gate half is a one-line guard at the caller and does not require a dedicated test harness beyond what the existing module already has.

AC8 (`cargo test` continues to pass with additional tests added) is a build-level guarantee satisfied by the combination of: (a) all new tests being unit tests against the pure resolver, which cannot regress the existing 277, and (b) the caller-site change being mechanically minimal and preserving the existing function's external behaviour in every non-deprecated branch.