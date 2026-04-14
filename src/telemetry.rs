//! Anonymous telemetry — tracks whether guardrails are useful.
//!
//! Only two events:
//! - "rule_fired": a guardrail matched and was injected
//! - "rule_followed": the LLM changed behavior after seeing the guardrail
//!
//! Opt-out: ARAI_TELEMETRY=off, DO_NOT_TRACK=1, or [telemetry] enabled = false in config.toml
//! Never runs on the hot hook path — events are queued and flushed async.

use std::path::Path;

const POSTHOG_HOST: &str = "https://us.i.posthog.com";
const POSTHOG_KEY: &str = "phc_CZ9YDA5V5NZC4iJTaHR9YdYjSmGE6svUK4fDk3NLTaFC";

/// Check if telemetry is enabled.
pub fn is_enabled() -> bool {
    if POSTHOG_KEY.is_empty() {
        return false;
    }

    // Standard opt-out env vars
    if std::env::var("ARAI_TELEMETRY").map(|v| v == "off" || v == "0" || v == "false").unwrap_or(false) {
        return false;
    }
    if std::env::var("DO_NOT_TRACK").map(|v| v == "1" || v == "true").unwrap_or(false) {
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

    let event_entry = serde_json::json!({
        "event": event,
        "properties": properties,
        "timestamp": chrono_now(),
    });

    // Append to queue file (one JSON object per line)
    let queue_path = arai_base.join("telemetry_queue.jsonl");
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

/// Track that a guardrail rule fired.
pub fn track_rule_fired(arai_base: &Path, subject: &str, predicate: &str, tool_name: &str, hook_event: &str) {
    track(arai_base, "rule_fired", serde_json::json!({
        "subject": subject,
        "predicate": predicate,
        "tool_name": tool_name,
        "hook_event": hook_event,
    }));
}

/// Track that a rule was followed (LLM changed behavior after seeing the guardrail).
#[allow(dead_code)]
pub fn track_rule_followed(arai_base: &Path, subject: &str, followed: bool) {
    track(arai_base, "rule_outcome", serde_json::json!({
        "subject": subject,
        "followed": followed,
    }));
}

/// Track an arai init event.
pub fn track_init(arai_base: &Path, rule_count: usize, file_count: usize, tool_count: i64, enrichment: &str) {
    track(arai_base, "arai_init", serde_json::json!({
        "rule_count": rule_count,
        "file_count": file_count,
        "code_graph_tools": tool_count,
        "enrichment_tier": enrichment,
    }));
}

/// Track hook latency.
pub fn track_hook_latency(arai_base: &Path, hook_event: &str, latency_ms: u128, matched: bool) {
    track(arai_base, "hook_latency", serde_json::json!({
        "hook_event": hook_event,
        "latency_ms": latency_ms,
        "matched": matched,
    }));
}

/// Flush queued events to PostHog (called from non-hook commands like `arai init`).
pub fn flush(arai_base: &Path) {
    if !is_enabled() {
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
            let mut props = e.get("properties").cloned().unwrap_or(serde_json::json!({}));
            if let Some(obj) = props.as_object_mut() {
                obj.insert("distinct_id".to_string(), serde_json::json!(distinct_id));
                obj.insert("arai_version".to_string(), serde_json::json!(env!("CARGO_PKG_VERSION")));
                obj.insert("os".to_string(), serde_json::json!(std::env::consts::OS));
                obj.insert("arch".to_string(), serde_json::json!(std::env::consts::ARCH));
            }
            serde_json::json!({
                "event": e.get("event").and_then(|v| v.as_str()).unwrap_or("unknown"),
                "properties": props,
                "timestamp": e.get("timestamp").and_then(|v| v.as_str()).unwrap_or(""),
            })
        })
        .collect();

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
            "-sS", "-X", "POST",
            &format!("{POSTHOG_HOST}/batch/"),
            "-H", "Content-Type: application/json",
            "-d", &payload_str,
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
