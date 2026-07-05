//! Anonymous telemetry — tracks whether guardrails are useful.
//!
//! Only two events:
//! - "rule_fired": a guardrail matched and was injected
//! - "rule_followed": the LLM changed behavior after seeing the guardrail
//!
//! Opt-out: ARAI_TELEMETRY=off, DO_NOT_TRACK=1, or `[telemetry] enabled = false` in config.toml.
//! Opt-outs always win — including over a configured custom endpoint.
//! Never runs on the hot hook path — events are queued and flushed async.
//!
//! Self-hosted collectors: `[telemetry] endpoint = "https://…"` (plus
//! optional `bearer_env`) redirects the existing queue to your own
//! infrastructure.  Default behavior is unchanged when unset.  Payload
//! schema: docs/telemetry-payload.md.

use std::path::Path;

const POSTHOG_HOST: &str = "https://us.i.posthog.com";
const POSTHOG_KEY: &str = "phc_CZ9YDA5V5NZC4iJTaHR9YdYjSmGE6svUK4fDk3NLTaFC";

/// Hard cap on the on-disk telemetry queue.  When the queue file is already
/// at or above this size, `track` silently drops new events instead of
/// appending — bounds worst-case growth for users who only ever invoke
/// hooks (the flush path runs from CLI commands like `arai init`/`scan`/
/// `audit`/`stats`).  Two megabytes is roughly 10,000 events; well past the
/// point where the upstream sink would dedupe anyway.
const TELEMETRY_QUEUE_CAP_BYTES: u64 = 2 * 1024 * 1024;

/// Check if telemetry is enabled.
pub fn is_enabled() -> bool {
    if POSTHOG_KEY.is_empty() {
        return false;
    }

    // Standard opt-out env vars
    if std::env::var("ARAI_TELEMETRY")
        .map(|v| v == "off" || v == "0" || v == "false")
        .unwrap_or(false)
    {
        return false;
    }
    if std::env::var("DO_NOT_TRACK")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false)
    {
        return false;
    }

    true
}

/// Queue an event to be sent later (non-blocking).
/// Events are written to a file and flushed in the background.
pub fn track(arai_base: &Path, event: &str, properties: serde_json::Value) {
    if !is_enabled() {
        return;
    }

    let queue_path = arai_base.join("telemetry_queue.jsonl");

    // Bound the queue at TELEMETRY_QUEUE_CAP_BYTES so a long-running install
    // that only ever invokes hooks (never an `arai init`/`scan`/`audit` etc.
    // that would flush) can't grow this file without limit.  We pay one
    // metadata syscall here (~30 µs on a warm filesystem) — well inside the
    // hook budget.  Dropped events are acceptable: the upstream sink dedups
    // identical rule firings via the salted rule_hash anyway, so trimming
    // the tail just loses a count, not a category.
    if let Ok(meta) = std::fs::metadata(&queue_path) {
        if meta.len() >= TELEMETRY_QUEUE_CAP_BYTES {
            return;
        }
    }

    let event_entry = serde_json::json!({
        "event": event,
        "properties": properties,
        "timestamp": chrono_now(),
    });

    // Append to queue file (one JSON object per line)
    if let Ok(line) = serde_json::to_string(&event_entry) {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&queue_path)
        {
            writeln!(file, "{line}").ok();
        }
    }
}

/// Stable salt for telemetry rule hashing.  Changing this breaks server-side
/// dedup of rule firings across releases — leave it alone.  The 12-hex-char
/// truncation already gives ~48 bits of entropy, so collision risk is
/// negligible at the rule-firing volumes we actually see.
const TELEMETRY_RULE_SALT: &str = "arai-telemetry-stable-2026";

/// Hash a rule's subject + predicate into an opaque, anonymous identifier.
/// Subject and predicate come straight from the user's CLAUDE.md and may
/// include codenames, vendor names, or internal service names — sending
/// them verbatim contradicts the README's "anonymous telemetry" claim.
/// The salted SHA-256 prefix lets the upstream sink dedup rule firings
/// across runs (same rule → same hash) without ever seeing the rule text.
fn rule_hash(subject: &str, predicate: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(TELEMETRY_RULE_SALT.as_bytes());
    h.update(b"|");
    h.update(subject.to_lowercase().as_bytes());
    h.update(b"|");
    h.update(predicate.to_lowercase().as_bytes());
    let bytes = h.finalize();
    bytes.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// Track that a guardrail rule fired.  Reports an anonymous `rule_hash`
/// instead of the raw subject/predicate so the rule text never leaves the
/// machine.  `severity` is included so the upstream sink can roll up
/// block-vs-warn-vs-inform mix without inferring it from predicate text.
pub fn track_rule_fired(
    arai_base: &Path,
    subject: &str,
    predicate: &str,
    tool_name: &str,
    hook_event: &str,
    match_pct: u8,
    severity: &str,
) {
    track(
        arai_base,
        "rule_fired",
        serde_json::json!({
            "rule_hash": rule_hash(subject, predicate),
            "tool_name": tool_name,
            "hook_event": hook_event,
            "match_pct": match_pct,
            "severity": severity,
        }),
    );
}

/// Track an arai init event.
pub fn track_init(
    arai_base: &Path,
    rule_count: usize,
    file_count: usize,
    tool_count: i64,
    enrichment: &str,
) {
    track(
        arai_base,
        "arai_init",
        serde_json::json!({
            "rule_count": rule_count,
            "file_count": file_count,
            "code_graph_tools": tool_count,
            "enrichment_tier": enrichment,
        }),
    );
}

/// Track hook latency.
pub fn track_hook_latency(arai_base: &Path, hook_event: &str, latency_ms: u128, matched: bool) {
    track(
        arai_base,
        "hook_latency",
        serde_json::json!({
            "hook_event": hook_event,
            "latency_ms": latency_ms,
            "matched": matched,
        }),
    );
}

/// `[telemetry]` settings from `{arai_base}/config.toml`.
///
/// - `enabled = false` — config-level opt-out.  Checked at the egress point
///   (`flush`), so nothing ever leaves the machine; the local queue file may
///   still accumulate up to its 2 MB cap but is never shipped.  The env
///   opt-outs (`ARAI_TELEMETRY=off`, `DO_NOT_TRACK=1`) are checked on the
///   hook path as before and always win.
/// - `endpoint = "https://…"` — self-hosted collector.  When set, batches
///   POST there instead of the default sink; the payload envelope is
///   `{"batch": [...]}` (no PostHog `api_key`).  See
///   `docs/telemetry-payload.md` for the schema.  HTTPS required, except
///   plain HTTP to loopback (`127.0.0.1` / `::1` / `localhost`) for local
///   collectors.
/// - `bearer_env = "VAR"` — name of an environment variable holding a
///   bearer token for the custom endpoint.  Same discipline as
///   `arai trust --bearer-env`: only the name is configured, the token is
///   resolved at flush time and never logged.
struct TelemetryConfig {
    enabled: bool,
    endpoint: Option<String>,
    bearer_env: Option<String>,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        TelemetryConfig {
            enabled: true,
            endpoint: None,
            bearer_env: None,
        }
    }
}

/// Parse the `[telemetry]` section out of a config.toml body.  Unknown keys
/// and malformed files fall back to defaults — telemetry must never break a
/// CLI command.
fn parse_telemetry_config(content: &str) -> TelemetryConfig {
    // `toml::Table`, not `toml::Value`: in toml v1 a document (with
    // `[section]` headers) only parses as a table.
    let Ok(table) = content.parse::<toml::Table>() else {
        return TelemetryConfig::default();
    };
    let Some(section) = table.get("telemetry") else {
        return TelemetryConfig::default();
    };
    TelemetryConfig {
        enabled: section
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        endpoint: section
            .get("endpoint")
            .and_then(|v| v.as_str())
            .map(String::from),
        bearer_env: section
            .get("bearer_env")
            .and_then(|v| v.as_str())
            .map(String::from),
    }
}

fn load_telemetry_config(arai_base: &Path) -> TelemetryConfig {
    let path = arai_base.join("config.toml");
    match std::fs::read_to_string(&path) {
        Ok(content) => parse_telemetry_config(&content),
        Err(_) => TelemetryConfig::default(),
    }
}

/// Validate a custom collector endpoint.  HTTPS anywhere; plain HTTP only to
/// loopback so a local dev collector works without a cert while off-host
/// traffic can never carry events (or a bearer token) unencrypted.
fn validate_endpoint(url: &str) -> Result<(), String> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if let Some(rest) = url.strip_prefix("http://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        // Reject userinfo outright: `http://localhost@evil.com/` connects to
        // evil.com — a naive host check would read "localhost" and wave the
        // loopback exception through.
        if authority.contains('@') {
            return Err("userinfo ('@') is not allowed in the telemetry endpoint".to_string());
        }
        let host = if authority.starts_with('[') {
            // IPv6 literal: everything up to and including the bracket.
            match authority.split_once(']') {
                Some((h, _)) => format!("{h}]"),
                None => return Err("malformed IPv6 literal in endpoint".to_string()),
            }
        } else {
            authority
                .rsplit_once(':')
                .map(|(h, _)| h.to_string())
                .unwrap_or_else(|| authority.to_string())
        };
        if host == "127.0.0.1" || host == "localhost" || host == "[::1]" {
            return Ok(());
        }
        return Err("plain http is only allowed for loopback collectors \
                    (127.0.0.1 / localhost / [::1]); use https"
            .to_string());
    }
    Err("telemetry endpoint must be an https:// URL".to_string())
}

/// POST a batch payload to a custom collector.  Returns Ok on any 2xx.
/// The bearer token travels only in the request header — never in argv
/// (visible in the process list) and never in the returned error text.
fn post_batch_custom(
    endpoint: &str,
    payload: &serde_json::Value,
    bearer: Option<&str>,
) -> Result<(), String> {
    let scrub = |msg: String| match bearer {
        Some(t) if !t.is_empty() => msg.replace(t, "[redacted]"),
        _ => msg,
    };
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(5)))
        .max_redirects(0)
        .build()
        .new_agent();
    let mut request = agent
        .post(endpoint)
        .header("Content-Type", "application/json");
    if let Some(token) = bearer {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let response = request
        .send(payload.to_string().as_bytes())
        .map_err(|e| scrub(format!("HTTP error: {e}")))?;
    if !response.status().is_success() {
        return Err(format!("HTTP {} from collector", response.status()));
    }
    Ok(())
}

/// Flush queued events to the configured sink (called from non-hook commands
/// like `arai init` — never from the hook hot path).  Default sink unless a
/// `[telemetry] endpoint` is configured; all opt-outs win regardless.
pub fn flush(arai_base: &Path) {
    if !is_enabled() {
        return;
    }
    let tcfg = load_telemetry_config(arai_base);
    if !tcfg.enabled {
        // Config-level opt-out: nothing leaves the machine, regardless of
        // endpoint.  The queue file stays local and capped.
        return;
    }

    let queue_path = arai_base.join("telemetry_queue.jsonl");
    if !queue_path.exists() {
        return;
    }

    let content = match std::fs::read_to_string(&queue_path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let events: Vec<serde_json::Value> = content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();

    if events.is_empty() {
        return;
    }

    // Get or create anonymous distinct_id
    let distinct_id = get_or_create_id(arai_base);

    let batch: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            let mut props = e
                .get("properties")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            if let Some(obj) = props.as_object_mut() {
                obj.insert("distinct_id".to_string(), serde_json::json!(distinct_id));
                obj.insert(
                    "arai_version".to_string(),
                    serde_json::json!(env!("CARGO_PKG_VERSION")),
                );
                obj.insert("os".to_string(), serde_json::json!(std::env::consts::OS));
                obj.insert(
                    "arch".to_string(),
                    serde_json::json!(std::env::consts::ARCH),
                );
            }
            serde_json::json!({
                "event": e.get("event").and_then(|v| v.as_str()).unwrap_or("unknown"),
                "properties": props,
                "timestamp": e.get("timestamp").and_then(|v| v.as_str()).unwrap_or(""),
            })
        })
        .collect();

    // Self-hosted collector: same events, their infrastructure.  Synchronous
    // with a hard 5 s timeout (flush runs from CLI commands, never hooks);
    // the queue is only cleared on success so a down collector loses nothing.
    if let Some(endpoint) = &tcfg.endpoint {
        if let Err(e) = validate_endpoint(endpoint) {
            eprintln!("arai: telemetry endpoint rejected: {e}");
            return;
        }
        let bearer = tcfg
            .bearer_env
            .as_deref()
            .and_then(|name| std::env::var(name).ok())
            .filter(|v| !v.is_empty());
        // No PostHog api_key on the custom path — the envelope is just the
        // batch.  Schema: docs/telemetry-payload.md.
        let payload = serde_json::json!({ "batch": batch });
        match post_batch_custom(endpoint, &payload, bearer.as_deref()) {
            Ok(()) => {
                std::fs::remove_file(&queue_path).ok();
            }
            Err(e) => {
                // Queue retained — next flush retries.  Error is already
                // token-scrubbed.
                eprintln!("arai: telemetry flush to custom endpoint failed: {e}");
            }
        }
        return;
    }

    // Default sink — unchanged behavior when no endpoint is configured.
    let payload = serde_json::json!({
        "api_key": POSTHOG_KEY,
        "batch": batch,
    });

    // Fire and forget — don't block on network
    let payload_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(_) => return,
    };

    // Use curl in background to avoid adding HTTP deps to non-enrich builds
    std::process::Command::new("curl")
        .args([
            "-sS",
            "-X",
            "POST",
            &format!("{POSTHOG_HOST}/batch/"),
            "-H",
            "Content-Type: application/json",
            "-d",
            &payload_str,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok();

    // Clear the queue
    std::fs::remove_file(&queue_path).ok();
}

/// Get or create a stable anonymous ID for this machine.
fn get_or_create_id(arai_base: &Path) -> String {
    let id_path = arai_base.join(".telemetry_id");
    if let Ok(id) = std::fs::read_to_string(&id_path) {
        let id = id.trim().to_string();
        if !id.is_empty() {
            return id;
        }
    }

    // Generate a random anonymous ID
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(format!("{:?}", std::time::SystemTime::now()).as_bytes());
    hasher.update(format!("{}", std::process::id()).as_bytes());
    let hash = hasher.finalize();
    let id: String = hash.iter().take(8).map(|b| format!("{b:02x}")).collect();
    let id = format!("arai-{id}");

    std::fs::create_dir_all(arai_base).ok();
    std::fs::write(&id_path, &id).ok();
    id
}

fn chrono_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", now.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_hash_is_stable_and_case_insensitive() {
        // Same input → same hash (deterministic, not seeded by time / pid).
        let a = rule_hash("git", "never");
        let b = rule_hash("git", "never");
        assert_eq!(a, b);
        // Hash is 12 hex chars (6 bytes truncated).
        assert_eq!(a.len(), 12);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        // Case-insensitive on subject and predicate so case-variant rule
        // text from CLAUDE.md collapses to one bucket upstream.
        assert_eq!(rule_hash("Git", "Never"), rule_hash("git", "never"));
    }

    #[test]
    fn rule_hash_differs_across_rules() {
        let a = rule_hash("git", "never");
        let b = rule_hash("cargo", "never");
        let c = rule_hash("git", "always");
        assert_ne!(a, b, "different subject → different hash");
        assert_ne!(a, c, "different predicate → different hash");
    }

    // ── #151: configurable telemetry endpoint ────────────────────────────────

    #[test]
    fn telemetry_config_parses_and_defaults() {
        // No section → defaults.
        let c = parse_telemetry_config("extra_sources = []");
        assert!(c.enabled && c.endpoint.is_none() && c.bearer_env.is_none());
        // Malformed file → defaults (telemetry must never break a command).
        let c = parse_telemetry_config("this is [ not toml");
        assert!(c.enabled && c.endpoint.is_none());
        // Full section.
        let c = parse_telemetry_config(
            "[telemetry]\nenabled = true\nendpoint = \"https://c.example.com/arai\"\nbearer_env = \"ARAI_TELEMETRY_TOKEN\"\n",
        );
        assert!(c.enabled);
        assert_eq!(c.endpoint.as_deref(), Some("https://c.example.com/arai"));
        assert_eq!(c.bearer_env.as_deref(), Some("ARAI_TELEMETRY_TOKEN"));
        // Config opt-out.
        let c = parse_telemetry_config("[telemetry]\nenabled = false\n");
        assert!(!c.enabled);
    }

    #[test]
    fn endpoint_validation_https_or_loopback_only() {
        assert!(validate_endpoint("https://collector.example.com/v1").is_ok());
        assert!(validate_endpoint("http://127.0.0.1:8080/ingest").is_ok());
        assert!(validate_endpoint("http://localhost:9999").is_ok());
        assert!(validate_endpoint("http://[::1]:8080/x").is_ok());
        assert!(validate_endpoint("http://[::1]").is_ok());
        assert!(validate_endpoint("http://collector.example.com/v1").is_err());
        assert!(validate_endpoint("http://127.0.0.2/x").is_err());
        assert!(validate_endpoint("ftp://x").is_err());
        // Userinfo trick: the real host is evil.com, not loopback.
        assert!(validate_endpoint("http://localhost@evil.com/x").is_err());
        assert!(validate_endpoint("http://localhost:8080@evil.com/x").is_err());
        assert!(validate_endpoint("http://127.0.0.1@evil.com").is_err());
    }

    /// Live loopback test: the batch reaches a custom collector with the
    /// bearer header, without the PostHog api_key, and the queue is only
    /// cleared on success.
    #[test]
    fn custom_endpoint_receives_batch_and_clears_queue_on_success() {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            // Headers and body arrive in separate writes — keep reading
            // (bounded by a read timeout) until the stream pauses.
            s.set_read_timeout(Some(std::time::Duration::from_millis(500)))
                .unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                match s.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        let text = String::from_utf8_lossy(&buf);
                        // Stop once the full declared body has arrived.
                        if let Some((head, body)) = text.split_once("\r\n\r\n") {
                            let expected = head
                                .lines()
                                .find_map(|l| {
                                    l.to_lowercase()
                                        .strip_prefix("content-length:")
                                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                                })
                                .unwrap_or(0);
                            if body.len() >= expected {
                                break;
                            }
                        }
                    }
                    Err(_) => break, // timeout: whatever arrived is the request
                }
            }
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
            String::from_utf8_lossy(&buf).to_string()
        });

        let base = std::env::temp_dir().join(format!("arai_tel_flush_{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(
            base.join("config.toml"),
            format!("[telemetry]\nendpoint = \"http://127.0.0.1:{port}/ingest\"\n"),
        )
        .unwrap();
        let queue = base.join("telemetry_queue.jsonl");
        std::fs::write(
            &queue,
            "{\"event\":\"rule_fired\",\"properties\":{\"rule_hash\":\"abc\"},\"timestamp\":\"1\"}\n{\"event\":\"hook_latency\",\"properties\":{\"latency_ms\":3},\"timestamp\":\"2\"}\n",
        )
        .unwrap();

        flush(&base);

        let request = handle.join().unwrap();
        assert!(request.contains("POST /ingest"));
        assert!(request.contains("rule_fired") && request.contains("hook_latency"));
        assert!(
            !request.contains(POSTHOG_KEY),
            "custom collectors must not receive the default sink api_key"
        );
        assert!(!queue.exists(), "queue must be cleared after a 2xx");
        std::fs::remove_dir_all(&base).ok();
    }

    /// A failing collector must retain the queue for the next flush, and the
    /// bearer token must reach the wire but never the error text.
    #[test]
    fn custom_endpoint_failure_keeps_queue_and_scrubs_token() {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut tmp = [0u8; 4096];
            let n = s.read(&mut tmp).unwrap_or(0);
            let req = String::from_utf8_lossy(&tmp[..n]).to_string();
            s.write_all(
                b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
            req
        });

        let payload = serde_json::json!({"batch": []});
        let err = post_batch_custom(
            &format!("http://127.0.0.1:{port}/ingest"),
            &payload,
            Some("tel-sekrit"),
        )
        .expect_err("503 must be an error");
        let request = handle.join().unwrap();
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer tel-sekrit"),
            "bearer must reach the collector; got:\n{request}"
        );
        assert!(
            !err.contains("tel-sekrit"),
            "error text leaked the token: {err}"
        );
    }

    /// `[telemetry] enabled = false` must stop egress even with an endpoint
    /// configured — no connection is attempted, the queue stays local.
    #[test]
    fn config_optout_blocks_custom_endpoint_egress() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();

        let base = std::env::temp_dir().join(format!("arai_tel_optout_{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(
            base.join("config.toml"),
            format!(
                "[telemetry]\nenabled = false\nendpoint = \"http://127.0.0.1:{port}/ingest\"\n"
            ),
        )
        .unwrap();
        let queue = base.join("telemetry_queue.jsonl");
        std::fs::write(
            &queue,
            "{\"event\":\"rule_fired\",\"properties\":{},\"timestamp\":\"1\"}\n",
        )
        .unwrap();

        flush(&base);

        assert!(
            matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "flush must not connect when enabled = false"
        );
        assert!(queue.exists(), "queue must be retained (local, capped)");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn rule_hash_does_not_leak_input_substring() {
        // The hash must not contain a recognisable suffix of the input —
        // catches accidental "send the first 12 chars of the subject"
        // regressions.  The salt makes this overwhelmingly unlikely
        // anyway, but pin it.
        let h = rule_hash("alembic-internal-codename", "never");
        assert!(!h.contains("alembic"));
        assert!(!h.contains("never"));
    }
}
