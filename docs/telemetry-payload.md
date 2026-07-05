# Telemetry payload schema (self-hosted collectors)

When `[telemetry] endpoint` is set in `~/.taniwha/arai/config.toml`, Ārai
POSTs queued telemetry batches to your collector instead of the default
sink. This documents exactly what arrives, so you know what you're
storing.

## Configuration

```toml
# ~/.taniwha/arai/config.toml
[telemetry]
enabled = true                                # opt-outs always win (see below)
endpoint = "https://collector.example.com/arai"
bearer_env = "ARAI_TELEMETRY_TOKEN"           # optional; env var NAME, not the token
```

- `endpoint` must be `https://`; plain `http://` is allowed only for
  loopback (`127.0.0.1` / `localhost` / `[::1]`) so a local dev
  collector works without a certificate.
- `bearer_env` names an environment variable; when set and non-empty at
  flush time, the request carries `Authorization: Bearer <token>`. Only
  the variable name is ever stored.
- Opt-outs override everything, endpoint included: `ARAI_TELEMETRY=off`,
  `DO_NOT_TRACK=1`, or `enabled = false` all mean nothing leaves the
  machine.

## Transport

- `POST <endpoint>` with `Content-Type: application/json`, 5-second
  timeout, redirects disabled.
- Flushes happen from CLI commands (`arai init` / `scan` / `audit` /
  `stats`), never from the hook hot path.
- Any 2xx clears the local queue; anything else retains it, and the next
  flush retries the same batch. Design your collector to tolerate
  duplicate batches (idempotent ingest or dedupe on event content).

## Envelope

```json
{
  "batch": [
    {
      "event": "rule_fired",
      "properties": { "...": "see per-event tables below" },
      "timestamp": "1751692800"
    }
  ]
}
```

- `timestamp` is Unix seconds as a string, recorded at queue time.
- The default-sink `api_key` field is **not** sent to custom endpoints.

## Common properties (merged into every event at flush time)

| Field | Type | Meaning |
|-------|------|---------|
| `distinct_id` | string | Random anonymous machine id (`arai-<8 hex>`); no hardware or user derivation |
| `arai_version` | string | Crate version |
| `os` | string | `std::env::consts::OS` |
| `arch` | string | `std::env::consts::ARCH` |

## Events

### `rule_fired`

| Field | Type | Meaning |
|-------|------|---------|
| `rule_hash` | string | Salted SHA-256 prefix (12 hex chars) of subject+predicate — rule text never leaves the machine |
| `tool_name` | string | Tool the rule fired on (Bash, Edit, …) |
| `hook_event` | string | PreToolUse / PostToolUse / UserPromptSubmit |
| `match_pct` | number | Match confidence 0–100 |
| `severity` | string | block / warn / inform |

### `hook_latency`

| Field | Type | Meaning |
|-------|------|---------|
| `hook_event` | string | Hook event name |
| `latency_ms` | number | End-to-end hook handling time |
| `matched` | bool | Whether any rule matched |

### `arai_init`

| Field | Type | Meaning |
|-------|------|---------|
| `rule_count` | number | Rules extracted |
| `file_count` | number | Instruction files discovered |
| `code_graph_tools` | number | Tools detected by the code scan |
| `enrichment_tier` | string | taxonomy / onnx / llm |

No project paths, no rule text, no prompts, no code content — the same
anonymity constraints as the default sink. The audit log
(`arai audit`) is a separate, local-only channel and is never shipped
by telemetry regardless of endpoint (see `arai audit --ship` / #149 for
the audit-side story).
