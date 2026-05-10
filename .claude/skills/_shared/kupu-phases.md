# Kupu phases and tool detection

This document describes how the skills detect Kupu's installed phase and choose between MCP tools and bash-fallback scripts. It is the authoritative reference; individual skill text refers to it rather than describing detection logic per skill.

## What is Kupu?

Kupu is the optional Rust MCP server that owns durable state for Taniwha projects. Without Kupu installed, the skills use bash-fallback scripts (under `_shared/scripts/`) to do the equivalent work — directly writing files under `.taniwha/kupu/`. With Kupu installed, the skills prefer Kupu's MCP tools, which provide schema enforcement, atomic writes, and structured returns.

The skills work either way. Kupu makes them substantially leaner; without Kupu they remain functional.

## Kupu phases

Kupu ships in phases. Each phase adds tools without changing previously-shipped tool surfaces. Skills detect available tools per-operation and choose accordingly.

**Phase 1** — primitives and project lifecycle:
- `kupu.new_id` — generate a fresh ULID
- `kupu.now` — current UTC instant in `{iso, filename}` form
- `kupu.init` — initialise a Taniwha project's `.taniwha/` skeleton
- `kupu.get_project` — read project metadata from `.taniwha/project.yaml`

**Phase 2** — durable writes:
- `kupu.record_event` — atomic event-plus-index write
- `kupu.record_decision` — write a decision record under `.taniwha/kupu/decisions/`
- `kupu.register_re_raise` — open a re-raise AND atomically mark blocked tree nodes
- `kupu.resolve_re_raise` — close a re-raise AND atomically unblock tree nodes

**Phase 3** — reads for Phase 1+2 artefacts:
- `kupu.get_event(event_id)` — single event lookup
- `kupu.list_events(kind?, since?)` — events index with kind/since filters
- `kupu.get_decision(decision_id)` — single decision lookup
- `kupu.list_decisions()` — decisions index
- `kupu.get_re_raise(re_raise_id)` — single re-raise (open or resolved)
- `kupu.list_re_raises(status?)` — re-raises index, optional open/resolved filter
- `kupu.get_tree()` — current tree state
- `kupu.get_brief(version?)` — brief at version (defaults to current)
- `kupu.list_briefs()` — all brief versions
- `kupu.get_project_context(version?)` — project context at version

**Phase 4** — build metrics:
- `kupu.record_dispatch_metrics(handoff_id, metrics)` — record metrics for a dispatch
- `kupu.get_dispatch_metrics(handoff_id)` — single dispatch metrics
- `kupu.get_build_metrics()` — aggregate metrics across the build (by_role, by_phase, slowest, largest)
- `kupu.get_dispatch_trace(handoff_id)` — full trace for one dispatch
- `kupu.export_metrics(format)` — JSON, NDJSON, CSV export

**Phase 5** — artefact CRUD for versioned families:
- `kupu.write_brief(content, source?)` — append a new brief version
- `kupu.write_design(content, parent_brief_version?)` — append a new design version
- `kupu.write_vocabulary(entries)` — append a new vocabulary version
- `kupu.write_contract(module, content, parent_design_version?)` — append a new contract version for a module
- `kupu.write_project_context(content)` — append a new project_context version
- Plus paired reads: `kupu.get_design(version?)`, `kupu.list_designs()`, `kupu.get_vocabulary(version?)`, `kupu.list_vocabularies()`, `kupu.get_contract(module, version?)`, `kupu.list_contracts(module)`, `kupu.list_project_contexts()`. Writes are append-only with server-computed contiguous version numbers and atomic 4-way bundle (file + family meta.yaml + project.yaml current pointer + emitted event).

**Phase 6** — tree operations:
- `kupu.next_dispatchable_node()` — list of unblocked nodes (structural query, agnostic to Taniwha discipline)
- `kupu.create_handoff(role, model, target_node, inputs)` — atomically create handoff directory + meta.yaml + tree update; inputs as `{filename, content}` pairs
- `kupu.update_handoff_status(handoff_id, new_status, payload?)` — FSM-enforced status transitions
- `kupu.promote_implementation(node_id, new_version)` — flip tree node to current atomically
- `kupu.mark_subtree_stale(root_node_id, reason)` — recursively mark subtree's implementations stale
- `kupu.get_handoff(handoff_id)` — single handoff lookup
- `kupu.list_handoffs(role?, status?, since?)` — handoffs index with filters

**Phase 7** — toolchain and validation:
- `kupu.detect_toolchain(project_root?)` — inspect project tree for language signals, return suggested toolchain commands
- `kupu.validate_contract(module, version?)` — structural validation of a contract artefact
- `kupu.validate_vocabulary(version?)` — structural validation of a vocabulary artefact

**Future phases** — see `kupu-tool-surface.md` in the kupu repository for any further roadmap.

## Detection model

**Per-operation detection.** Each time a skill needs to perform an operation that has both an MCP-tool form and a bash-fallback form, the skill checks at the moment of need whether the corresponding `kupu.<tool>` is available.

Available means: the host (Claude Code) reports the tool in its tool list when the skill is dispatched. The skill does not poll, query, or maintain its own registry — it simply attempts to use the tool by name and falls back if not present.

The check is essentially free in tokens and latency. Per-operation detection means the skills automatically work across all Kupu phases: a Phase 1-only Kupu installation gets `kupu.new_id` and `kupu.now` from MCP and bash for everything else; a Phase 2 installation also uses MCP for event/decision/re-raise writes; a future Phase 3+ installation will use MCP for read tools too.

## Schema agreement

Both backends — Kupu and bash-fallback — must produce identical on-disk artefacts. The canonical schema for every file under `.taniwha/` is described in `state-layout.md`.

Kupu's Rust serde structs are the **source of truth** for these schemas. Bash-fallback templates in skill text are documentation that must match exactly. If they drift, that's a skill-text bug, not a Kupu bug.

This means: when state-layout.md describes a record's shape, that shape is what Kupu's serde structs serialise/deserialise. The bash-fallback templates are reverse-engineered from Kupu's output, not the other way around.

## Per-operation mapping

The table below shows how each skill operation maps to backends. When Kupu's MCP tool is available, use it; otherwise fall back to the bash form.

| Operation | Kupu MCP tool | Bash fallback |
|---|---|---|
| Generate ULID | `kupu.new_id` | `bash _shared/scripts/util/new_ulid.sh` |
| Get current time | `kupu.now` | `bash _shared/scripts/util/now.sh --both` |
| Compute event path | (none — derived in skill) | `bash _shared/scripts/util/event_path.sh` |
| Initialise project | `kupu.init` | manually create `.taniwha/` skeleton + write `project.yaml` |
| Read project metadata | `kupu.get_project` | manually parse `.taniwha/project.yaml` |
| Record event | `kupu.record_event` | manually write event file + update events index |
| Record decision | `kupu.record_decision` | manually write decision file + update decisions index |
| Register re-raise | `kupu.register_re_raise` | manually write re-raise file + mutate `tree/current.yaml` |
| Resolve re-raise | `kupu.resolve_re_raise` | manually move re-raise file + mutate `tree/current.yaml` |
| Read event | `kupu.get_event` | manually parse event file |
| List events | `kupu.list_events` | manually parse `events/index.yaml` |
| Read decision | `kupu.get_decision` | manually parse decision file |
| List decisions | `kupu.list_decisions` | manually parse `decisions/index.yaml` |
| Read re-raise | `kupu.get_re_raise` | manually search both open and resolved sub-trees |
| List re-raises | `kupu.list_re_raises` | manually `ls re-raises/open/` and `re-raises/resolved/` |
| Read tree | `kupu.get_tree` | manually parse `tree/current.yaml` |
| Read brief | `kupu.get_brief` | manually parse `brief/v<N>.md` |
| List briefs | `kupu.list_briefs` | manually `ls brief/*.md` |
| Read project context | `kupu.get_project_context` | manually parse `project_context.yaml` |
| Record dispatch metrics | `kupu.record_dispatch_metrics` | (no fallback — metrics are an optional capability; skip when absent) |
| Read dispatch metrics | `kupu.get_dispatch_metrics` | manually parse handoff `meta.yaml` metrics block |
| Aggregate build metrics | `kupu.get_build_metrics` | (no fallback — would require walking all handoffs) |
| Export metrics | `kupu.export_metrics` | (no fallback — format-specific serialisation) |
| Write brief | `kupu.write_brief` | manually write `brief/v<N+1>.md` + update `project.yaml` current_version |
| Write design | `kupu.write_design` | manually write `design/v<N+1>.md` + update meta + project.yaml |
| Write vocabulary | `kupu.write_vocabulary` | manually write `vocabulary/v<N+1>.md` + update meta + project.yaml |
| Write contract | `kupu.write_contract` | manually write `contracts/<module>/v<N+1>.md` + update meta + project.yaml |
| Write project_context | `kupu.write_project_context` | manually write `project_context.yaml` (versioned) + update project.yaml |
| Read design | `kupu.get_design` | manually parse `design/v<N>.md` |
| List designs | `kupu.list_designs` | manually `ls design/*.md` |
| Read vocabulary | `kupu.get_vocabulary` | manually parse `vocabulary/v<N>.md` |
| Read contract | `kupu.get_contract` | manually parse `contracts/<module>/v<N>.md` |
| List contracts | `kupu.list_contracts` | manually `ls contracts/<module>/*.md` |
| Find unblocked nodes | `kupu.next_dispatchable_node` | manually parse tree, walk dependency graph |
| Create handoff | `kupu.create_handoff` | manually create handoff directory + meta.yaml + tree update |
| Update handoff status | `kupu.update_handoff_status` | manually edit handoff `meta.yaml` |
| Promote implementation | `kupu.promote_implementation` | manually edit `tree/current.yaml` + implementations meta |
| Mark subtree stale | `kupu.mark_subtree_stale` | manually walk tree, edit each affected node's status |
| Read handoff | `kupu.get_handoff` | manually parse handoff `meta.yaml` |
| List handoffs | `kupu.list_handoffs` | manually `ls orchestrator/handoff/` and parse meta.yaml each |
| Detect toolchain | `kupu.detect_toolchain` | manually inspect project tree for language signals |
| Validate contract structure | `kupu.validate_contract` | (no fallback — validation logic complex; skip when absent) |
| Validate vocabulary structure | `kupu.validate_vocabulary` | (no fallback — validation logic complex; skip when absent) |

For operations marked "manually" in the bash-fallback column, the skill text describes the canonical YAML/Markdown shape. The shape must match what Kupu's serde structs produce — see state-layout.md for the authoritative schemas.

## Why this matters

The bash-fallback path involves real ceremony: subprocess calls for primitives, hand-written YAML following the canonical schema, manual atomic-write patterns (write-temp-then-rename), explicit verify-after-write steps to catch the missing-decision-file class of bug.

When Kupu is present, all of that compresses. `kupu.record_event` is one MCP call. Schema validation happens server-side. Atomicity is enforced by Kupu's implementation, not by skill text discipline. Verification is implicit in the tool's success return.

The skills' job is to call the right thing at the right moment. The detection logic above keeps that simple: try MCP first, fall back if needed.

## Future direction

In a future skills release (probably v3.0), the bash-fallback path may be retired entirely once Kupu adoption is established. v2.0 keeps both paths because requiring Kupu installation as a hard dependency raises the bar for new users; better to ship "works without Kupu, dramatically better with it."

The Kupu phases model is the staging path: each Kupu phase shipped lets a corresponding chunk of bash-fallback code be retired in a later skills release.
