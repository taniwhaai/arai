# Taniwha Project State Layout

This document specifies the on-disk layout of a Taniwha project. The layout is the durable, canonical shape of the project — every artefact an agent (or human reviewer) needs to act correctly is here, and nothing the agent needs to know lives only in conversation history or in-memory state.

## Design principles

These principles govern every decision in this layout. When extending or modifying it, check changes against them.

**Cold readability.** Every artefact must make sense to an agent reading it for the first time, with no prior context, no surrounding conversation, and no other documents loaded. If understanding artefact A requires reading artefact B, A must explicitly reference B by path, not implicitly assume the reader knows about it.

**Decisions, not just outcomes.** When something changed — a contract was amended, a re-raise was resolved, a pairing was reconsidered — the state records both the new value and the reasoning that produced it. A future agent picking up the project must be able to answer "why is it this way?" without re-deriving the answer from scratch.

**Local completeness over DRY.** Where there's tension between not repeating yourself and giving an agent everything it needs locally, choose local completeness. An agent acting on one manifest should not need to read three other manifests to understand it. Cross-references are fine; cross-dependencies that hide information are not.

**Immutable contracts, append-only history.** Contracts and other authoritative artefacts are versioned and immutable once published. Changes produce new versions; old versions are retained. History is append-only — events are recorded as they happen and never edited.

**Optimised for agents, navigable for humans.** The format is structured (YAML/JSON for machine state, Markdown for content) and slightly verbose by human standards. Humans can still vet any single module in isolation, but the layout does not sacrifice agent-friendliness for human elegance.

**Filesystem is the only memory.** No part of the system relies on memory that survives only within a single conversation or process. If an agent needs to know it, it is on disk.

## Top-level layout

A Taniwha project lives in a single root directory. The user's source code lives at that root in whatever structure the project context specifies. Taniwha tools' state lives in `.taniwha/` at the project root.

`.taniwha/` is the company-level namespace. Each Taniwha tool owns its own subdirectory under `.taniwha/`. Cross-tool coordination (project ID, tool versions in use) lives in `.taniwha/project.yaml` at the company root. The skills covered by this document are backed by Kupu — the runtime state manager — which writes to `.taniwha/kupu/`.

```
<project-root>/
├── .taniwha/                       # Company-level: shared by all Taniwha tools
│   ├── project.yaml                # Project ID, name, tool versions in use
│   └── kupu/                       # Skills' runtime state — managed by Kupu
│       ├── project_context.yaml    # Language, toolchain, repo conventions
│       ├── brief/
│       ├── design/
│       ├── vocabulary/
│       ├── contracts/
│       ├── implementations/
│       ├── compositions/
│       ├── tree/
│       ├── re-raises/
│       ├── decisions/
│       ├── events/
│       └── orchestrator/
└── (user's source code laid out per project_context — at the repo root,
   not inside .taniwha/)
```

Other Taniwha tools, when they need per-project state, get their own subdirectory under `.taniwha/` (e.g. `.taniwha/arai/` if Arai is extracted to per-project state in the future). Tools never write outside their own subdirectory, with the single exception of `.taniwha/project.yaml` which is the agreed cross-tool coordination point.

**Critical separation: `.taniwha/` is agent and tool state, not a code store.** Source code, build files, configuration, and any artefact intended for the user to ship lives at the repo root in the layout the project context describes. `.taniwha/` holds only the durable record of *what was built and why* — manifests pointing at source paths, decision records, event logs, contracts. A user committing this project to git would expect their editor to open at the repo root, their build commands to run there, and their CI to operate on those files. Putting source inside `.taniwha/` would conflate tool history with shipping artefacts and break all of that.

This separation is enforced through the implementations directory layout (described below): manifests reference source files by repo-root paths, they do not contain copies.

Each subdirectory under `.taniwha/kupu/` is described in detail below. Every file inside follows the conventions in "File conventions" at the end of this document.

## `.taniwha/project.yaml`

The project's root manifest. One file, present from the moment the project is initialised. Records the project's identity, the tooling versions in use, and the cross-tool coordination metadata that any Taniwha tool can read.

**This is a cross-tool file** (lives at `.taniwha/`, not `.taniwha/kupu/`). Its schema is canonical: the dispatcher's bash-fallback bootstrap must produce exactly this shape, and Kupu's parser must accept exactly this shape. Drift between bash-written and Kupu-written project.yaml files is a coordination bug, not a quirk to handle. Both sides conform to the canonical schema below.

```yaml
schema_version: 1
project_id: <ULID or stable slug>
name: <human-readable name>
created_at:
  iso: <ISO 8601 with millisecond precision, ending in 'Z'>
  filename: <YYYYMMDDTHHMMSSsssZ>
tooling_versions:
  kupu: "<semver>"           # absent if Kupu not installed
  arai: "<semver>"           # absent if Arai not installed
  taniwha_skills: "<semver>"  # always present; matches the skills suite version
tools_registered:
  - kupu          # entries here mirror tooling_versions keys
  - taniwha_skills
current_phase: <free-text phase descriptor, e.g. "kickoff", "building", "verifying", "phase-1-complete">
```

**Fields that are part of the canonical schema and MUST be present:**

- `schema_version` (integer; currently `1`)
- `project_id` (string; ULID or slug)
- `name` (string)
- `created_at` (Timestamp struct, see below)
- `tooling_versions` (mapping of tool-name → semver string)
- `tools_registered` (list of tool-name strings; should match `tooling_versions` keys)
- `current_phase` (free-text string)

**Optional convenience fields** (skills may include these for their own use; consumers MUST tolerate their absence):

- `brief: { current_version, path }` — the skills' pointer to the brief
- `project_context: { path }` — the skills' pointer to project context
- `design_doc: { current_version, path, history: [...] }`
- `vocabulary: { current_version, path }`
- `configuration: { model_routing: {...}, human_gates: [...] }`

These optional fields live alongside the canonical fields, not nested under a `project:` key. Putting any field under `project:` produces a file that Kupu's parser will reject; the bash-fallback bootstrap must use top-level fields to remain Kupu-readable.

### Timestamp shape (used here and elsewhere)

Every place a timestamp appears in a `.taniwha/` file — `created_at` here, `dispatched_at` and `returned_at` in handoff metadata, `timestamp` in event records, `created_at` in decision files — uses the same struct shape:

```yaml
<field_name>:
  iso: "2026-05-03T01:08:02.000Z"
  filename: "20260503T010802000Z"
```

NOT a bare ISO string. Both sub-fields describe the same instant to the millisecond. This is what Kupu's `kupu.now` produces and what its parsers expect; the bash-fallback `_shared/scripts/util/now.sh --both` produces both forms from a single clock read for the same purpose.

### When to write project.yaml

The dispatcher writes this file once at kickoff (with the canonical fields plus any optional fields the skills want to record). Subsequent updates happen when versioned artefacts advance — design doc v1 → v2, vocabulary v1 → v2, etc. — and the optional pointer fields update accordingly. The canonical fields rarely change after initial capture.

This file is small and rarely changes. Its purpose is to give a returning agent the project's basic shape in one read, AND to give cross-tool coordination (e.g. Kupu reading what Taniwha skills wrote, or vice versa) a reliable common schema.

## `.taniwha/kupu/project_context.yaml`

Records the project-level facts that every code-producing agent must honour: language, toolchain, repository style, directory conventions, build/test commands, code style. Populated at kickoff via a structured user-input round, before any agent that produces code runs. Never populated by inference — these are the user's decisions, not the agents'.

```yaml
project_context:
  language:
    name: <language name>           # e.g. python, go, typescript, rust
    version: <version constraint>   # e.g. ">=3.11", "1.24", "node 20"
  toolchain:
    binary_path: <absolute path>    # detected at kickoff, may be null if not detected
    version: <version string>       # detected at kickoff, may be null
    commands:
      test: <shell command string>      # e.g. "cargo test", "pytest", "go test ./..."
      build: <shell command string>     # may be empty string for languages with no build step
      format: <shell command string>    # e.g. "cargo fmt --all", "ruff format ."
      lint: <shell command string>      # e.g. "cargo clippy", "ruff check ."
  repo_style:
    kind: monorepo | single_package | workspace
    module_layout: <path template>  # e.g. "internal/{module}", "src/{module}"
    test_layout: alongside_source | separate_tests_dir | <path template>
  shared_types:
    package_path: <path>            # only present for multi-module tiers with shared sharing markers
  conventions:
    naming: <kebab|snake|camel>      # how module names map to filesystem names
    code_style_notes: |
      Free-text notes the user wants every code-producing agent to honour.
      May be empty.

provenance:
  authored_by: user
  authored_at:
    iso: <ISO 8601 with milliseconds, ending in Z>
    filename: <YYYYMMDDTHHMMSSsssZ>
  amended:
    - amended_at:
        iso: <...>
        filename: <...>
      reason: <short>
      decision_ref: decisions/<id>.md
```

The `toolchain.commands` block is the **single source of truth** for how to invoke language-specific tools in this project. Captured once at kickoff via user confirmation (with defaults from `_shared/registries/toolchain-defaults.yaml`), reused for the lifetime of the build. Every dispatch — verifier, leaf, composition — reads commands by name from here. No skill should re-derive language-specific commands; doing so is a discipline gap to surface as a finding.

If a command field is empty string (for languages without a conventional invocation in that role, e.g. Python's lack of a single canonical build command), the corresponding skill operations skip — no build dispatch is made if `commands.build` is empty.

This file is authoritative for code-producing agents. Whenever a contract or design clause could be expressed in a way that depends on language or toolchain (concurrency primitives, error handling style, package layout, file extensions), the agents must defer to project context, not pick something themselves. Contracts and design docs remain language-neutral; project context is where language-specific choices are recorded.

The file is amended only by user action (via a structured user-input round). Agents may not edit it. If an agent finds that its work cannot be done within the current project context (e.g. the chosen language genuinely cannot satisfy a contract clause), the correct response is to re-raise with category `out_of_scope` and source `project_context`, asking the user to amend.

## `.taniwha/kupu/brief/`

The original user brief, versioned. The brief is the prompt the user gave at kickoff; subsequent versions exist when the user has answered re-raises or amended scope, and the answers are folded into the brief as authoritative text rather than scattered across decision records.

```
brief/
├── v1.md
├── v2.md
└── ...
```

`v1.md` is the verbatim original prompt with a metadata header (`captured_at`, `source: user_kickoff`). Each subsequent version's header records what changed from the previous version and which decision record explains it. The current version is named in `project.yaml` under `brief.current_version` and `brief.path`.

Versioning the brief brings amendments into the same versioned-immutable model as design and vocabulary, making them discoverable to cold readers.

## `.taniwha/kupu/design/`

The design document, versioned. Each version is a separate Markdown file. The current version is named in `project.yaml`.

```
design/
├── v1.md
├── v2.md
└── ...
```

The design doc itself is produced by the `design-doc` skill and follows that skill's output format. New versions are produced when a re-raise bubbles to the root and the user approves an amendment. Old versions are retained — they are part of the project's permanent history.

Each design doc version begins with a header recording its version number, the date it was approved, and a short note describing what changed from the previous version (or "initial version" for v1). This note is critical for cold-reading agents: it tells them whether they need to read older versions to understand current decisions.

## `.taniwha/kupu/vocabulary/`

The shared vocabulary file (data shapes, external systems, cross-cutting concerns) referenced by all manifests. Versioned in the same way as the design doc.

```
vocabulary/
├── v1.md
├── v2.md
└── ...
```

The current version is referenced by name from `project.yaml` (add a `vocabulary.current_version` and `vocabulary.path` block alongside `design_doc`). Vocabulary versions track design doc versions but do not have to bump in lockstep — the vocabulary changes when shapes or shared concerns change, which may or may not be the same moment the design doc changes.

## `.taniwha/kupu/contracts/`

The per-module manifests. Each manifest is a directory named for the module, containing one file per version.

```
contracts/
├── <module-name>/
│   ├── v1.md
│   ├── v2.md
│   ├── meta.yaml
│   └── ...
└── <another-module>/
    └── ...
```

`<module-name>/v<N>.md` is the manifest content as produced by the `contract-derivation` skill. It is immutable once published.

`<module-name>/meta.yaml` records:

```yaml
module: <module-name>
current_version: <integer>
versions:
  - version: 1
    created_at: <ISO 8601>
    derived_from:
      design_doc_version: <integer>
      vocabulary_version: <integer>
    supersedes: null
    superseded_by: 2
    decision_ref: decisions/<id>.md  # null for initial version
  - version: 2
    created_at: <ISO 8601>
    derived_from:
      design_doc_version: <integer>
      vocabulary_version: <integer>
    supersedes: 1
    superseded_by: null
    decision_ref: decisions/<id>.md
```

`derived_from` is critical: it pins each contract version to the design and vocabulary versions it was derived against. A returning agent can detect drift (current design version does not match what this contract was derived from) and flag it.

`decision_ref` points to a record in `.taniwha/kupu/decisions/` explaining why this version exists. The initial version's decision is allowed to be the bare statement "derived from design v1"; subsequent versions must reference a substantive decision record.

## `.taniwha/kupu/implementations/`

Implementation manifests for leaf modules. Each implementation is a directory named for the module, with versioned subdirectories. **The actual source code does not live here** — it lives at the repo root, in the layout the project context specifies. This directory holds only the manifests that record which source files satisfied which version of which contract.

```
implementations/
├── <module-name>/
│   ├── v1/
│   │   ├── manifest.yaml
│   │   └── notes.md
│   ├── v2/
│   │   └── ...
│   └── meta.yaml
```

`v<N>/manifest.yaml` records:

```yaml
implementation: <module-name>
version: <integer>
targets_contract:
  module: <module-name>
  version: <integer>
created_at: <ISO 8601>
status: current | superseded | stale
project_context_version: <integer>     # which project_context this targeted
source_paths:                          # repo-root-relative paths
  - kind: code | test | config
    path: internal/codegeneration/code_generation.go
  - kind: test
    path: internal/codegeneration/code_generation_test.go
  # ...
verification:
  acceptance_criteria_met: <true|false|partial>
  verified_at: <ISO 8601 or null>
  verifier_record: events/<id>.yaml
```

`status` matters for returning agents:
- `current`: this implementation targets the current contract version.
- `superseded`: a newer implementation exists.
- `stale`: the contract this targets has been superseded but no new implementation exists yet. The manifest is still on disk but should not be trusted; the orchestrator (or a returning agent) should re-dispatch implementation work for the current contract version.

**Source paths reference current files, not historical snapshots.** When a manifest has `status: superseded`, the paths it lists are the same paths the newer manifest references — the old manifest tells you "this version targeted contract version N" and points at where the code lives, but reading those files shows whatever is *currently* there. To reconstruct what the code looked like at the time of an older manifest version, use git history. The state layout does not duplicate source code across versions.

This is a deliberate tradeoff: source-code history belongs in the user's version-control system, not in `.taniwha/`. Duplicating code across implementation versions inside `.taniwha/` would conflate agent history with shipping artefacts and be confusing to anyone navigating the repo with normal tools.

`v<N>/notes.md` is the implementor's record of acceptance-criterion satisfaction (per the leaf-implementation skill's output format). It is the implementation's self-documentation for cold readers and does belong inside `.taniwha/` — it is agent-produced metadata about the implementation, not the implementation itself.

`<module-name>/meta.yaml` records the version history at module level, mirroring the contracts pattern.

## `.taniwha/kupu/compositions/`

Composition manifests for interior tree nodes. As with implementations, source code lives at the repo root in the layout the project context specifies; this directory holds only the manifests.

```
compositions/
├── <composition-id>/
│   ├── v1/
│   │   ├── manifest.yaml
│   │   └── notes.md
│   └── meta.yaml
```

`<composition-id>` is a stable identifier assigned by the orchestrator (e.g. derived from the parent contract name). `v<N>/manifest.yaml` records:

```yaml
composition: <composition-id>
version: <integer>
parent_contract:
  module: <module-name>
  version: <integer>
children:
  a:
    kind: implementation | composition
    id: <module-or-composition-id>
    version: <integer>
  b:
    kind: implementation | composition
    id: <module-or-composition-id>
    version: <integer>
created_at: <ISO 8601>
status: current | superseded | stale
project_context_version: <integer>
source_paths:                          # repo-root-relative paths
  - kind: code | test | config
    path: internal/api/post_shorten.go
verification:
  acceptance_criteria_met: <true|false|partial>
  verified_at: <ISO 8601 or null>
  verifier_record: events/<id>.yaml
```

The `children` block is the wiring record — it pins this composition to specific versions of its inputs. If either child is later superseded, this composition's status flips to `stale`.

The same source-code-history principle applies as for implementations: paths reference current files; reconstructing historical content uses git, not `.taniwha/`.

## `.taniwha/kupu/tree/`

The composition tree structure. One file, `tree.yaml`, plus a versioned history.

```
tree/
├── current.yaml
└── history/
    ├── v1.yaml
    ├── v2.yaml
    └── ...
```

`current.yaml` records the current shape of the tree:

```yaml
tree_version: <integer>
root:
  parent_contract:
    module: <module-name>
    version: <integer>
  node:
    kind: composition | leaf
    id: <id>
    version: <integer>
    # if composition:
    children:
      a:
        # recursive node
      b:
        # recursive node
```

History files capture the tree at each significant change. Tree versions bump when nodes are added, removed, or re-paired (not when implementations within nodes change).

A returning agent reads `current.yaml` first to understand the project's overall shape, then descends into individual contracts and implementations as needed.

## `.taniwha/kupu/re-raises/`

The re-raise queue and history.

```
re-raises/
├── open/
│   └── <re-raise-id>.yaml
├── resolved/
│   └── <re-raise-id>.yaml
└── index.yaml
```

Each `<re-raise-id>.yaml` follows the format defined in the re-raise protocol (see `_shared/re-raise-protocol.md`), plus framework metadata:

```yaml
id: <ulid or similar>
created_at: <ISO 8601>
emitted_by:
  agent_role: <role>
  acting_on:
    kind: contract | implementation | composition
    id: <id>
    version: <integer>
re_raise:
  # the structured re-raise as defined in the protocol
routing:
  destination:
    kind: contract_author | user | composer
    target_id: <id or "user">
  hop_count: <integer>  # how many levels it has bubbled
status: open | resolved | superseded
resolution:  # null while open
  resolved_at: <ISO 8601>
  resolved_by:
    kind: agent | user
    role: <role or null>
  outcome:
    kind: contract_amended | re_dispatched | re_raised_higher | rejected
    decision_ref: decisions/<id>.md
    new_versions:
      - kind: contract | composition | etc
        id: <id>
        version: <integer>
```

Open re-raises are the orchestrator's worklist. Resolved re-raises are project history — a returning agent uses them to understand why current decisions are the way they are. `index.yaml` is a flat summary listing all re-raises for fast scanning.

## `.taniwha/kupu/decisions/`

The reasoning record. Every substantive decision the system makes — contract amendments, re-pairings, escalations, user resolutions — has a decision record here.

```
decisions/
├── <decision-id>.md
└── index.yaml
```

Each decision record is Markdown with a fixed front-matter:

```markdown
---
id: <ulid>
created_at: <ISO 8601>
kind: contract_amendment | composition_repair | re_raise_resolution | scope_change | user_intervention
triggered_by:
  kind: re_raise | user | verifier_failure | manual
  ref: <re-raise id, event id, etc>
affects:
  - kind: contract | composition | tree | design_doc | vocabulary
    id: <id>
    from_version: <integer or null>
    to_version: <integer>
---

# Decision: <short title>

## Context
[What state of affairs prompted this decision.]

## Options considered
[Each option, with what it would have meant and why it was kept or rejected.]

## Resolution
[What was actually decided.]

## Rationale
[Why this option, in terms a cold-reading agent can act on later.]

## Consequences
[What the decision invalidated, what new work it produced, what it foreclosed.]
```

This is the single most important artefact for returning-agent readability. A future agent looking at a contract that seems oddly shaped reads its `decision_ref` and finds a complete record of why it has that shape. There is no "ask the team" — the team is a sequence of ephemeral agents who left these records.

`index.yaml` is a chronological flat list of all decisions for fast scanning.

## `.taniwha/kupu/events/`

The append-only event log. Records every action the orchestrator takes and every result it receives. Used for audit, debugging, and recovery.

```
events/
├── <year>/
│   ├── <month>/
│   │   └── <day>/
│   │       └── <timestamp>-<event-id>.yaml
└── index.yaml
```

**Date bucketing convention: UTC, always.** The `<year>/<month>/<day>/` directory is determined by the UTC date of the event's timestamp, not the local date of whoever is running the build. This matters because builds may be paused and resumed across timezones (or by agents running in different regions), and local-date bucketing would scatter related events across directories or — worse — cause the same calendar date to mean different things to different readers. UTC is unambiguous and matches the ISO 8601 `Z` suffix that timestamps already carry.

The `<timestamp>` in the filename is the UTC timestamp formatted as `YYYYMMDDTHHMMSSZ` (e.g. `20260501T220833Z`), and it must match the `timestamp` field inside the file. The directory (`2026/05/01/`) must match the date portion of that timestamp.

Each event:

```yaml
id: <ulid>
timestamp: <ISO 8601>
kind: orchestrator_decision | subagent_dispatched | subagent_returned | re_raise_emitted | re_raise_resolved | verification_run | user_input_requested | user_input_received | user_input_timed_out | build_started | build_completed | build_paused | build_resumed
actor:
  kind: dispatcher | orchestrator | <role> | user | verifier
  instance_id: <ephemeral id, useful for correlating>
payload:
  # event-specific structured data
correlation:
  decision_ref: <decision id or null>
  re_raise_ref: <re-raise id or null>
  contract_ref: <module-name@version or null>
```

Events are append-only and never edited. They are the system's ground-truth log; if a state file disagrees with events, events win and state must be reconstructed.

`index.yaml` is the most recent N events flat-listed for fast access by returning agents and by the orchestrator (the orchestrator typically only needs to know the recent events to decide what's next, not the entire history).

## `.taniwha/kupu/orchestrator/`

The orchestrator's working area. Distinct from the rest of the layout because it changes rapidly and its content is consumed by the dispatcher and the next orchestrator subagent.

```
orchestrator/
├── current_state.yaml
├── next_action.yaml
└── handoff/
    └── <handoff-id>/
        ├── inputs/
        ├── outputs/
        └── meta.yaml
```

`current_state.yaml` is the orchestrator's distilled view of where the build is — pointers into the rest of the layout, summary of open re-raises, current focus. Written by each orchestrator subagent before it exits, read by the next one when it starts.

`next_action.yaml` is the most recent orchestrator subagent's instruction to the dispatcher: spawn this role with these inputs, write outputs there, then re-invoke me. See the dispatcher skill for its exact format.

`handoff/<handoff-id>/` is the working area for one subagent dispatch. `inputs/` contains the documents the subagent will be given (copies, not references — the subagent receives them in its prompt). `outputs/` is where the dispatcher places the subagent's results when it returns. `meta.yaml` records the dispatch parameters (role, model, timing, status).

Handoff directories are not strictly history — once their results are integrated into the rest of the state, the handoff can be archived or deleted. They are kept for at least the duration of the current build for debugging; long-term archival is a configuration choice.

## File conventions

These apply to every file under `.taniwha/`:

**Encoding.** UTF-8.

**Structured data format.** YAML for state, configuration, and metadata. JSON acceptable where another tool requires it. The choice is per-directory and is fixed once chosen.

**Content format.** Markdown for content artefacts (design doc, vocabulary, contracts, decision records, notes). Markdown files have YAML front-matter where structured metadata is needed.

**Identifiers.** ULIDs for ids that need to sort by creation time (events, re-raises, decisions). Stable slugs for ids that name persistent things (modules, compositions). Slugs use kebab-case, are immutable once assigned, and may not collide.

**Timestamps.** ISO 8601 with the `Z` suffix indicating UTC. UTC is required, not preferred — this is the only way for cold-reading agents in arbitrary locations to interpret the log unambiguously. Local timestamps with offsets are not allowed.

**Versions.** Monotonically increasing integers starting at 1. No version 0. Versions never reused, even after deletion.

**Paths.** Relative to `.taniwha/` root within state files (so the state is portable). Absolute paths only in transient orchestrator working files where the dispatcher needs them.

**Cross-references.** Always include kind, id, and version when referencing another artefact. Never reference by name alone.

**No silent edits.** Files in versioned directories (contracts, implementations, compositions, design, vocabulary) are immutable once written. Edits create new versions. The only files that are edited in place are `meta.yaml` files (to update status fields), `current.yaml` files (to update pointers), and `index.yaml` files (to append).

## What this layout does not contain

Some things deliberately do not live in `.taniwha/`:

**Source code.** All source code, tests, build files, configuration, and any artefact intended for the user to ship lives at the repo root in the layout the project context specifies. `.taniwha/` is for agent state — manifests, decisions, history. Implementation manifests reference source files by repo-root paths but never contain copies. This separation lets the user treat the repo root as a normal codebase (open it in an editor, build it, commit it) without `.taniwha/` interfering.

**Source-code history.** Earlier versions of source files are not preserved under `.taniwha/`. Source-code history lives in the user's version-control system (typically git). Implementation manifests under older versions reference *current* paths; reconstructing what those paths contained at the time of an older manifest version is git's job.

**Build outputs.** Compiled binaries, packaged artefacts, deploy targets — not Taniwha's concern. Source lives at the repo root; what happens downstream is up to the user.

**Secrets and credentials.** Never. Manifests reference external systems by name; credentials to talk to those systems live in the user's standard secret-management mechanism.

**Conversation history with the user.** The decision records capture what was decided and why. If the user wants a verbatim record of what they said, that's a chat log concern, not a project state concern.

## Cold-reading checklist

When designing or extending this layout, ask: could a returning agent six months from now, with no context but this directory, do each of the following?

1. Understand what the project is. (`project.yaml`, current brief, current design doc.)
2. Understand the language and toolchain choices. (`project_context.yaml`.)
3. Understand the project's current shape. (`tree/current.yaml`.)
4. Pick any one module and understand its contract in isolation. (Manifest plus referenced vocabulary version.)
5. Pick any one implementation and find its source files. (Implementation manifest's `source_paths`, then the actual files at the repo root.)
6. Verify an implementation satisfies its contract. (Implementation manifest plus its `notes.md` plus the contract version it targets.)
7. Understand why anything is the way it is. (Decision records, reachable from contracts/implementations/compositions via `decision_ref`.)
8. Detect staleness. (Status fields plus `derived_from`, `targets_contract`, and `project_context_version` pinning.)
9. Resume an interrupted build. (`orchestrator/current_state.yaml`, open re-raises, recent events.)

If any of these is not possible from the layout alone, the layout is incomplete. Fix the layout, not the agent.
