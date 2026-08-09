# Grok Build hook invocation — re-verification on 1.0.0

**Date:** 2026-08-10  
**Host:** Grok Build **1.0.0** (`3cd0d0cbce`), WSL2  
**Related:** [#173](https://github.com/taniwhaai/arai/issues/173) (0.2.112 ACP path showed no hook invocation)

This note freezes a fresh host probe and the Arai-side defect that made
live Grok sessions look like a pure host failure when they were not.

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

ACP `grok agent stdio` was **not** fully exercised (JSON-RPC handshake not
completed in this session). Treat the TUI/headless path as verified; treat
ACP/agent as **unverified on 1.0.0**, not as still broken.

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

### Fixes landed with this verification

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

## What remains open (not closed by this note)

| Item | Status |
| --- | --- |
| ACP / `grok agent` entrypoint on 1.0.0 | Unverified (handshake incomplete) |
| Interactive TUI (non-headless) on 1.0.0 | Unverified here (headless proved host contract) |
| Upstream filing for 0.2.112 | May be obsolete if fixed in 1.0.0; confirm ACP before closing #173 as host-fixed |
| Full support-table audit by entrypoint | Still recommended follow-up |

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
