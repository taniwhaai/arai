# Shipping the audit trail to your own collector

`arai audit --ship` centralizes the tamper-evident audit log on **your
infrastructure** — a compliance collector, a SIEM gateway, an internal
service. Local-first stays the default: nothing ships unless you invoke
it (or a cron/CI job does).

## Configuration

```toml
# ~/.taniwha/arai/config.toml
[ship]
url = "https://collector.example.com/arai/audit"
bearer_env = "ARAI_SHIP_TOKEN"   # optional; env var NAME, not the token
```

```bash
arai audit --ship                          # use [ship] url from config
arai audit --ship https://collector/...   # one-off URL
arai audit --ship --json                   # machine-readable report
```

- HTTPS required; plain HTTP allowed only for loopback
  (`127.0.0.1` / `localhost` / `[::1]`) dev collectors.
- Exits non-zero on any rejection — safe to gate a cron/CI job on.
- Never runs on the hook hot path; this is an explicit administrative
  command, same family as `--verify` and `--purge`.

## What arrives at the collector

One `POST` per pending day-bucket, `Content-Type: application/json`:

```json
{
  "project": "myproj-1a2b3c4d",
  "day": "20260705",
  "head": "9f2c…64-hex…e1",
  "jsonl": "{\"ts\":…,\"prev_hash\":…,\"hash\":…}\n{…}\n"
}
```

- `jsonl` is the **raw** day-bucket content, byte-for-byte. It is not
  re-serialized — the hash chain is computed over these exact canonical
  bytes, so the collector can verify integrity independently.
- `head` is the content of the `.head.YYYYMMDD` chain sidecar (`null`
  if the sidecar was absent — pre-chain installs).

## Verifying the chain server-side

Each JSONL line carries `prev_hash` and `hash`, where
`hash = SHA-256(prev_hash + "|" + canonical_line_bytes)` (hex). To
verify a shipped bucket:

1. Split `jsonl` on newlines; for each line, recompute the hash from
   the previous line's `hash` (the first line's `prev_hash` anchors the
   day) and compare with the line's own `hash` field.
2. The final line's `hash` must equal `head`.

Any tamper, reorder, or deletion in transit or at rest breaks the
recomputation — the same guarantee `arai audit --verify` gives locally,
now server-attested.

## Resume + idempotency

- A cursor (`.ship_cursor.json` next to the buckets) records the byte
  length and chain head each successful POST covered. Interrupted or
  failed runs resume exactly where they left off; unchanged buckets are
  skipped.
- Today's bucket is live and re-ships whole when it grows. **Dedupe on
  the per-entry `hash`** — it is unique per chain position, so replayed
  entries are exact duplicates you can drop.

## Minimal collector example

Any HTTPS endpoint accepting POSTed JSON works. A ten-line reference
(Python/Flask) that stores buckets and verifies chains:

```python
import hashlib, flask
app = flask.Flask(__name__)

@app.post("/arai/audit")
def ingest():
    b = flask.request.get_json()
    lines = [l for l in b["jsonl"].split("\n") if l]
    prev = None
    for line in lines:
        import json; entry = json.loads(line)
        if prev is not None and entry["prev_hash"] != prev:
            return "chain broken", 400
        prev = entry["hash"]
    if b["head"] and prev != b["head"]:
        return "head mismatch", 400
    open(f'{b["project"]}-{b["day"]}.jsonl', "w").write(b["jsonl"])
    return "", 204
```

(Production collectors should also recompute each line's `hash` from
the canonical bytes per the algorithm above, authenticate the bearer,
and dedupe on `hash`.)

## What never ships

The audit channel and the telemetry channel stay physically separate:
`--ship` sends only audit buckets, telemetry settings have no effect on
it, and the bearer token (resolved from `bearer_env` at run time) is
never written to disk, the cursor, error messages, or the audit log
itself.
