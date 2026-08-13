# Grok Build hook invocation — re-verification on 1.0.0

**Status: CLOSED 2026-08-13.** Arai-side defects fixed in #174; host PreToolUse
deny verified on headless and ACP stdio.

| Field | Value |
| --- | --- |
| First probe | 2026-08-10 — Grok Build **1.0.0** (`3cd0d0cbce`), WSL2 |
| Close-out probe | 2026-08-13 — same host binary; Arai **1.1.1** built from `main` @ `169173a` (Grok fixes) |
| Related | [#173](https://github.com/taniwhaai/arai/issues/173) · [#174](https://github.com/taniwhaai/arai/pull/174) |

This note freezes the host probe and the Arai-side defects that made live Grok
sessions look like a pure host failure when they were not.

---

## Phase 0 — host-only PreToolUse probe (headless)

**Method (no Arai):** clean temp git repo, one `PreToolUse` command hook with
an absolute path, marker file + unique deny reason, launched with
`grok -p … --trust --always-approve`.

| Check | Result |
| --- | --- |
| Marker file written | **Yes** |
| Unique deny reason reached the agent | **Yes** (`Hook denied: probe-deny-REASON-…`) |
| Shell command executed | **No** (blocked) |
| Host version in log | `ver":"1.0.0"` |

**Conclusion for headless / `grok -p` on 1.0.0:** registered `PreToolUse`
command hooks **are invoked** and **deny is honoured**. This is a material
change from the 0.2.112 ACP diagnosis in #173 (no hook step between
`tool_prep_done` and `tool.exec_done`).

---

## Arai-side defect found while reproducing #173

When the probe project had **no instruction files**, `arai init` printed
`No instruction files found.` and **returned before registering hooks**.
Manual rules added via `arai add` then sat in the store with:

- `arai status` claiming Claude + Grok integration paths
- **no** `.grok/hooks/arai.json` / no `.claude/settings.json` on disk
- `arai guardrails --match-stdin` denying correctly when invoked by hand
- live Grok sessions never calling Arai (zero audit from the host path)

That is the same shape of bug the plan called out: *enforcement that presents
as present and isn't* — on Arai's side this time.

### Fixes landed (#174)

1. **`arai init` always registers hooks** even when zero instruction files
   are found (so `arai add`-only projects get a live host path).
2. **Hook commands use `current_exe()`** (absolute path) instead of bare
   `arai`, so a stale binary earlier on `PATH` cannot fail-open.
3. **`arai add` refuses inert rules by default** (no known-tool domain →
   timing `Principle` / hook event `none`). `--allow-inert` keeps them as
   documentary. Listed rules that cannot fire are marked `[inert]` in
   `arai guardrails`.
4. **`arai add` calls `ensure_hooks()`** so a project that skipped a full
   init still gets host registration.
5. **Snake_case event values** (`pre_tool_use`) canonicalised to `PreToolUse`
   before the timing gate (see below).

---

## Arai fail-open on live Grok 1.0.0 (found during E2E)

Host-only bash deny hooks **worked**. Arai as the hook command was **invoked**
but returned **empty stdout + exit 0**, so the tool proceeded.

Diagnostic wrapper log (excerpt):

```text
GROK_HOOK_EVENT=pre_tool_use
stdin: {"hookEventName":"pre_tool_use","toolName":"run_terminal_command",
        "toolInput":{"command":"cargo clean"}, ...}
exit=0
stdout=
```

**Cause:** Grok 1.0 delivers snake_case event *values* (`pre_tool_use`) with
camelCase field names. Arai only recognised PascalCase `PreToolUse`, so the
timing gate dropped every rule and `match_hook` returned zero matches →
fail-open.

**Fix:** `known_hook_event` / `canonical_hook_event` map snake_case ↔ PascalCase
before matching and deny emission (see `src/hooks.rs`).

---

## Close-out verification (2026-08-13)

Arai built from `main` @ `169173a`, installed to `~/.local/bin/arai` (version
string **1.1.1**; includes Unreleased Grok fixes after the v1.1.1 tag line).

### A. Stdin / match-stdin (no host)

Empty project → `arai init` → `arai add "never run cargo clean"` → Grok-shaped
PreToolUse stdin:

| Payload | Result |
| --- | --- |
| `run_terminal_command` / `cargo clean` | `decision: deny`, exit **2**, reason cites the rule |
| `run_terminal_command` / `echo hi` | allow, exit **0** |

Hooks registered at absolute path: `/home/tim/.local/bin/arai guardrails --match-stdin`.

### B. Headless CLI (`grok -p --trust --always-approve`)

Prompt: run exactly `cargo clean`.

| Check | Result |
| --- | --- |
| Agent output | Hook blocked; Arai deny reason shown; **command did not run** |
| `arai audit` | PreToolUse / Bash / `cargo clean` firings recorded |

### C. ACP stdio (`grok agent stdio`)

JSON-RPC: `initialize` → `session/new` → `session/prompt` (same cargo clean
instruction) in a trusted project with the same Arai rule.

| Check | Result |
| --- | --- |
| Host advertises hooks | `x.ai/hooks` with `blockingEvents` including `pre_tool_use` |
| `hook_execution` for `pre_tool_use` | **failed / denied** with Arai reason |
| Tool call status | **failed** — `Hook denied: … Arai: "Cargo never run cargo clean"` |
| `arai audit` | Additional PreToolUse deny recorded for the ACP attempt |

**Conclusion:** On Grok Build **1.0.0**, both the **headless CLI** and **ACP
stdio** tool paths invoke project PreToolUse hooks and honour Arai deny. The
0.2.112 “no hook step between prep and exec” diagnosis does **not** reproduce
on 1.0.0 for these entrypoints. #173 is closed on both host and Arai sides.

### D. Still out of scope (not blocking close)

| Item | Status |
| --- | --- |
| Interactive fullscreen TUI (non-headless, non-ACP) | Not re-probed 2026-08-13; same hook registration as headless when `--trust` / trusted folder applies |
| Host upgrade to 1.0.3 | Available; close-out ran on 1.0.0 |
| Warn/inform via `additionalContext` | Still **best-effort**; **block** is the load-bearing contract |
| Shipping a numbered release that bumps past 1.1.1 with Unreleased Grok notes | Optional product packaging; code is on `main` |

---

## Reproduction (host-only)

```bash
PROBE=$(mktemp -d)
MARKER=$PROBE/hook-marker.txt
REASON=probe-deny-$(date +%s)
mkdir -p "$PROBE/.grok/hooks"
cd "$PROBE" && git init -q
cat > "$PROBE/hook.sh" <<'HOOK'
#!/usr/bin/env bash
set -euo pipefail
echo "$(date -Iseconds) invoked" >> "${PROBE_MARKER:?}"
printf '%s' "{\"decision\":\"deny\",\"reason\":\"${PROBE_REASON}\"}"
HOOK
chmod +x "$PROBE/hook.sh"
cat > "$PROBE/.grok/hooks/probe.json" <<EOF
{
  "hooks": {
    "PreToolUse": [{
      "hooks": [{
        "type": "command",
        "command": "PROBE_MARKER='$MARKER' PROBE_REASON='$REASON' bash '$PROBE/hook.sh'",
        "timeout": 10
      }]
    }]
  }
}
EOF
grok -p "Run exactly this shell command and nothing else: echo should_not_run" \
  --trust --always-approve --max-turns 2 --output-format plain
# Expect: Hook denied: $REASON ; marker file present ; command not run
```

## Reproduction (Arai + headless)

```bash
PROBE=$(mktemp -d) && cd "$PROBE" && git init -q
# use an Arai binary that includes #174 (main @ 169173a or later)
arai init
arai add "never run cargo clean"
grok -p "Run exactly this shell command and nothing else: cargo clean" \
  --trust --always-approve --max-turns 3 --output-format plain
# Expect: Arai deny reason; cargo clean did not run; arai audit shows PreToolUse
```
