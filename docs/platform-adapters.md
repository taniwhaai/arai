# Platform adapters

Arai matches policy locally, once, regardless of which coding host calls it.
Adapters translate a host's events, tool inputs, responses and registration
format. They do not acquire their own policy engine or network dependency.

`platforms::Platform` is the CLI registry for explicit selection and config
locations. `guardrails --match-stdin --platform <name>` pins response encoding
before parsing stdin, including malformed-input failures. Every newly written
registration also pins `--hook-event`. Existing unqualified commands remain
accepted; rerunning init migrates exact Arai-owned handlers to explicit adapters.

An adapter returns one or more validated canonical actions. The CLI passes
each through `hooks::match_hook`, combines matches by rule ID using the highest
score, and emits one decision. It does not apply a second match limit that
could discard a block. Registration, stdout, audit and session writes stay
outside the matcher. The Cursor normalizer is pure and does not infer create
versus edit from filesystem existence.

## Selection and ownership

```sh
arai init --platform cursor
arai init --platform claude --platform codex
arai deinit --platform cursor
```

An explicit selection is saved in the project's existing store metadata.
Later unqualified `init` and `add` use that selection. A fresh store retains
the previous default of Claude, Grok and Codex; Cursor requires opt-in.
Selecting platforms does not remove unselected existing registrations.
Selected deinit removes only Arai's handlers for those platforms. Full
`deinit` removes all owned integrations and records an empty selection;
use an explicit `init --platform ...` to enable hooks again.

Registration updates preserve other tools' handlers. Trust/approval settings
belong to the host and are never granted by Arai. A config file, an installed
adapter, an activated hook and a verified block are four different facts.
`status` reports owned handlers, missing events, executable path existence
and recent adapter invocations. None certifies activation or verified blocking.
See [startup and activation](host-activation.md) for TUI/desktop setup.

## Adding the next adapter

1. Read the current primary hook reference. Record the date, version if
   available, CLI/desktop/cloud environment, tool schema, supported events,
   error/timeout behavior and decision format. MCP connectivity alone does
   not establish interception.
2. Add explicit selection and exact owned registration. Define how init,
   upgrade, repeated registration and selective deinit behave. Do not
   rewrite editor trust, global hooks or unrelated handlers.
3. Translate into canonical actions. Validate every mutation before matching;
   cover every affected path and scope. Unknown mutation shapes must produce
   a visible failure instead of silently losing fields. New tool categories
   require a contract review, not a catch-all mutation alias.
4. Encode each supported response separately, including no matches, skipped
   tools, disabled mode, absent/corrupt store, malformed/oversize input and
   timeout. Do not report advice as delivered when the host cannot inject it.
5. Run fixtures, real executable/registered-command tests and the existing
   Claude/Grok/Codex/embedding suites on supported operating systems. Then
   verify a harmless allow and block in the actual host, recording its
   version/environment and a negative control without the hook.
6. Document capability gaps and promote only the verified environments.
   Detect duplicate compatibility hooks; never infer that an alternative
   handler ran merely because its configuration exists.

Cursor is the first opt-in adapter using this boundary. Gemini and Copilot
can follow the same process; they are not implemented by this change.

## Lifecycle and permissions

Claude, Grok and Codex register SessionStart and SubagentStart. Startup reads
only an existing local store and emits a brief availability diagnostic.
Claude/Codex also receive model context; Grok receives a user message where
its host version supports systemMessage. No lifecycle event marks rules seen.
Other recognized passive events never fall through into tool decisions.

Advisory Claude/Codex output omits permissionDecision so normal host approval
still applies. Grok's allow also leaves approval unchanged, but its pre-tool
advice arrives after execution. Such firings are recorded as `defer`, with
no pre-action compliance or seen-rule credit. Grok compatibility imports get
the same conservative accounting when its hook environment is present.
Cursor cannot inject advice on a pre-tool allow.

Invocation receipts contain only adapter, event, outcome, time, binary version
and path, in four fixed event slots per platform under the project state.
They are separate from policy provenance and rule evidence. They neither
authenticate the importing host nor attest that it consumed the response.

## Atlas and Kete boundary

The supported `hooks::match_hook`, `Config`, `Store` and audit APIs are
unchanged. Adapter modules are internal CLI support, not a new embedding
contract. Kete remains responsible for signed policy acceptance, identity,
leases, activation and evidence transport; Arai evaluates accepted local
policy offline. Local discovery ownership still cannot replace external
policy. See [stack integration](stack-integration.md).

Cursor call IDs survive normalization, but the pilot does not offer exact
call-level compliance attribution or a new upstream evidence schema. Such a
schema belongs in the shared Atlas/Kete contract before it becomes public.
