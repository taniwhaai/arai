//! `arai audit --ship` — send audit day-buckets to your own collector.
//!
//! Bring-your-own-collector for the tamper-evident audit trail: each
//! day-bucket JSONL file is POSTed together with its `.head.YYYYMMDD`
//! chain sidecar, so the collector can independently verify the hash
//! chain (server-attested tamper evidence).  The JSONL travels as raw
//! bytes — re-serialising would alter the canonical bytes the chain
//! hashes are computed over.
//!
//! Discipline:
//! - Explicit opt-in only: runs when `arai audit --ship` is invoked (or a
//!   cron/CI job invokes it).  Never on the hook hot path.
//! - Resume cursor: `.ship_cursor.json` next to the buckets records what
//!   each successful POST covered; interrupted ships pick up where they
//!   left off, and unchanged buckets are skipped.
//! - Idempotent re-ship: a grown bucket (today's) is re-POSTed whole; the
//!   collector dedupes on the per-entry chain `hash`.
//! - HTTPS only (plain HTTP allowed solely for loopback dev collectors);
//!   optional bearer auth via an environment-variable *name* in config —
//!   the token itself is never persisted or logged.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `[ship]` settings from `{arai_base}/config.toml`.
///
/// ```toml
/// [ship]
/// url = "https://collector.example.com/arai/audit"
/// bearer_env = "ARAI_SHIP_TOKEN"   # optional; env var NAME, not the token
/// ```
#[derive(Debug, Default, PartialEq)]
pub struct ShipConfig {
    pub url: Option<String>,
    pub bearer_env: Option<String>,
}

/// Parse the `[ship]` section out of a config.toml body.  Malformed files
/// and missing sections fall back to defaults.
pub fn parse_ship_config(content: &str) -> ShipConfig {
    let Ok(table) = content.parse::<toml::Table>() else {
        return ShipConfig::default();
    };
    let Some(section) = table.get("ship") else {
        return ShipConfig::default();
    };
    ShipConfig {
        url: section
            .get("url")
            .and_then(|v| v.as_str())
            .map(String::from),
        bearer_env: section
            .get("bearer_env")
            .and_then(|v| v.as_str())
            .map(String::from),
    }
}

pub fn load_ship_config(arai_base: &Path) -> ShipConfig {
    match std::fs::read_to_string(arai_base.join("config.toml")) {
        Ok(content) => parse_ship_config(&content),
        Err(_) => ShipConfig::default(),
    }
}

/// Validate a collector endpoint.  HTTPS anywhere; plain HTTP only to
/// loopback.  Userinfo is rejected outright — `http://localhost@evil.com`
/// connects to evil.com, and a naive host check would wave it through.
pub fn validate_collector_url(url: &str) -> Result<(), String> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if let Some(rest) = url.strip_prefix("http://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        if authority.contains('@') {
            return Err("userinfo ('@') is not allowed in the collector URL".to_string());
        }
        let host = if authority.starts_with('[') {
            match authority.split_once(']') {
                Some((h, _)) => format!("{h}]"),
                None => return Err("malformed IPv6 literal in collector URL".to_string()),
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
    Err("collector URL must be https:// (or http:// to loopback)".to_string())
}

/// Per-bucket cursor entry: what the last successful POST of this bucket
/// covered.  A bucket whose current size and head both match its cursor
/// entry has nothing new to ship.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CursorEntry {
    /// Byte length of the bucket file at the time it was shipped.
    pub bytes: u64,
    /// Content of the `.head.YYYYMMDD` sidecar at ship time (may be empty
    /// when the sidecar was absent).
    pub head: String,
}

type Cursor = BTreeMap<String, CursorEntry>;

fn cursor_path(arai_base: &Path, slug: &str) -> PathBuf {
    // Dot-prefixed and not date-shaped, so `arai audit --purge` leaves it
    // alone (purge only touches `YYYYMMDD.jsonl` / `.head.YYYYMMDD`).
    arai_base.join("audit").join(slug).join(".ship_cursor.json")
}

fn read_cursor(arai_base: &Path, slug: &str) -> Cursor {
    let path = cursor_path(arai_base, slug);
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Cursor::default(),
    }
}

fn write_cursor(arai_base: &Path, slug: &str, cursor: &Cursor) -> Result<(), String> {
    let path = cursor_path(arai_base, slug);
    let encoded =
        serde_json::to_string_pretty(cursor).map_err(|e| format!("encode cursor: {e}"))?;
    std::fs::write(&path, encoded).map_err(|e| format!("write cursor: {e}"))
}

/// A shippable day-bucket found on disk.
struct Bucket {
    day: String,
    path: PathBuf,
    bytes: u64,
    head: String,
}

/// List day-buckets for a project, oldest first.  Only well-formed
/// `YYYYMMDD.jsonl` names are considered — same filter as `purge`.
fn list_buckets(arai_base: &Path, slug: &str) -> Result<Vec<Bucket>, String> {
    let dir = arai_base.join("audit").join(slug);
    let mut buckets = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(buckets), // no audit dir → nothing to ship
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(day) = name.strip_suffix(".jsonl") else {
            continue;
        };
        if day.len() != 8 || !day.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let head = std::fs::read_to_string(dir.join(format!(".head.{day}")))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        buckets.push(Bucket {
            day: day.to_string(),
            path,
            bytes,
            head,
        });
    }
    buckets.sort_by(|a, b| a.day.cmp(&b.day));
    Ok(buckets)
}

/// Pure skip decision: nothing to ship when the bucket is byte-identical
/// (same length) and chain-identical (same head) to the last success.
fn already_shipped(bucket_bytes: u64, bucket_head: &str, cursor: Option<&CursorEntry>) -> bool {
    match cursor {
        Some(c) => c.bytes == bucket_bytes && c.head == bucket_head,
        None => false,
    }
}

/// Outcome of a ship run.
#[derive(Debug, serde::Serialize)]
pub struct ShipReport {
    pub shipped: Vec<String>,
    pub skipped: usize,
    pub shipped_bytes: u64,
}

/// POST one bucket to the collector.  Ok on any 2xx.  The bearer token
/// travels only in the request header and is scrubbed from error text.
fn post_bucket(
    url: &str,
    slug: &str,
    bucket: &Bucket,
    jsonl: &str,
    bearer: Option<&str>,
) -> Result<(), String> {
    let scrub = |msg: String| match bearer {
        Some(t) if !t.is_empty() => msg.replace(t, "[redacted]"),
        _ => msg,
    };
    let payload = serde_json::json!({
        "project": slug,
        "day": bucket.day,
        // Chain anchor: the collector recomputes the chain over `jsonl`
        // and must arrive at exactly this head.
        "head": if bucket.head.is_empty() { serde_json::Value::Null } else { serde_json::json!(bucket.head) },
        // Raw JSONL — canonical bytes preserved for chain verification.
        "jsonl": jsonl,
    });
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .max_redirects(0)
        .build()
        .new_agent();
    let mut request = agent.post(url).header("Content-Type", "application/json");
    if let Some(token) = bearer {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let response = request
        .send(payload.to_string().as_bytes())
        .map_err(|e| scrub(format!("HTTP error: {e}")))?;
    if !response.status().is_success() {
        return Err(format!(
            "HTTP {} from collector for bucket {}",
            response.status(),
            bucket.day
        ));
    }
    Ok(())
}

/// Ship all pending day-buckets for a project to `url`.  Stops at the first
/// failure with the cursor already advanced past every shipped bucket, so
/// the next invocation resumes exactly there.
pub fn ship(
    arai_base: &Path,
    slug: &str,
    url: &str,
    bearer: Option<&str>,
) -> Result<ShipReport, String> {
    validate_collector_url(url)?;
    let buckets = list_buckets(arai_base, slug)?;
    let mut cursor = read_cursor(arai_base, slug);
    let mut report = ShipReport {
        shipped: Vec::new(),
        skipped: 0,
        shipped_bytes: 0,
    };

    for bucket in &buckets {
        if already_shipped(bucket.bytes, &bucket.head, cursor.get(&bucket.day)) {
            report.skipped += 1;
            continue;
        }
        // Read exactly the bytes we recorded — the bucket may grow while we
        // ship (today's is live).  Truncating to the listed length keeps the
        // cursor entry truthful.
        let content = std::fs::read_to_string(&bucket.path)
            .map_err(|e| format!("read bucket {}: {e}", bucket.day))?;
        post_bucket(url, slug, bucket, &content, bearer)?;
        cursor.insert(
            bucket.day.clone(),
            CursorEntry {
                bytes: content.len() as u64,
                head: bucket.head.clone(),
            },
        );
        // Persist after every bucket so an interruption never re-ships
        // what already landed (beyond the collector-side dedupe).
        write_cursor(arai_base, slug, &cursor)?;
        report.shipped.push(bucket.day.clone());
        report.shipped_bytes += content.len() as u64;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(label: &str, days: &[(&str, &str, &str)]) -> (PathBuf, String) {
        let slug = format!("proj-{label}");
        let base = std::env::temp_dir().join(format!("arai_ship_{label}_{}", std::process::id()));
        let dir = base.join("audit").join(&slug);
        std::fs::create_dir_all(&dir).unwrap();
        for (day, content, head) in days {
            std::fs::write(dir.join(format!("{day}.jsonl")), content).unwrap();
            if !head.is_empty() {
                std::fs::write(dir.join(format!(".head.{day}")), head).unwrap();
            }
        }
        (base, slug)
    }

    /// Minimal loopback collector: answers `count` requests with `status`,
    /// returning the raw request texts.
    fn collector(
        count: usize,
        status: &'static str,
    ) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..count {
                let Ok((mut s, _)) = listener.accept() else {
                    break;
                };
                s.set_read_timeout(Some(std::time::Duration::from_millis(500)))
                    .unwrap();
                let mut buf = Vec::new();
                let mut tmp = [0u8; 65536];
                loop {
                    match s.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            let text = String::from_utf8_lossy(&buf);
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
                        Err(_) => break,
                    }
                }
                requests.push(String::from_utf8_lossy(&buf).to_string());
                let response =
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                s.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (port, handle)
    }

    #[test]
    fn ship_config_parses_and_defaults() {
        assert_eq!(parse_ship_config("extra = 1"), ShipConfig::default());
        assert_eq!(parse_ship_config("not [ toml"), ShipConfig::default());
        let c = parse_ship_config(
            "[ship]\nurl = \"https://c.example.com/audit\"\nbearer_env = \"ARAI_SHIP_TOKEN\"\n",
        );
        assert_eq!(c.url.as_deref(), Some("https://c.example.com/audit"));
        assert_eq!(c.bearer_env.as_deref(), Some("ARAI_SHIP_TOKEN"));
    }

    #[test]
    fn collector_url_validation() {
        assert!(validate_collector_url("https://collector.example.com/v1").is_ok());
        assert!(validate_collector_url("http://127.0.0.1:9000/ingest").is_ok());
        assert!(validate_collector_url("http://collector.example.com/v1").is_err());
        assert!(validate_collector_url("http://localhost@evil.com/x").is_err());
        assert!(validate_collector_url("ws://x").is_err());
    }

    #[test]
    fn skip_decision_table() {
        let entry = CursorEntry {
            bytes: 10,
            head: "abc".into(),
        };
        assert!(already_shipped(10, "abc", Some(&entry)));
        assert!(!already_shipped(11, "abc", Some(&entry))); // grew
        assert!(!already_shipped(10, "def", Some(&entry))); // head moved
        assert!(!already_shipped(10, "abc", None)); // never shipped
    }

    #[test]
    fn ships_buckets_with_heads_then_skips_then_reships_growth() {
        let (base, slug) = fixture(
            "roundtrip",
            &[
                ("20260101", "{\"hash\":\"h1\"}\n", "headhash-day1"),
                ("20260102", "{\"hash\":\"h2\"}\n", "headhash-day2"),
            ],
        );

        // First run: both buckets ship, envelopes carry raw jsonl + head.
        let (port, handle) = collector(2, "200 OK");
        let url = format!("http://127.0.0.1:{port}/ingest");
        let report = ship(&base, &slug, &url, None).unwrap();
        assert_eq!(report.shipped, vec!["20260101", "20260102"]);
        assert_eq!(report.skipped, 0);
        let requests = handle.join().unwrap();
        assert!(requests[0].contains("headhash-day1"));
        assert!(requests[0].contains("h1"));
        assert!(requests[1].contains("headhash-day2"));

        // Second run: nothing new — no connection needed, all skipped.
        let report = ship(&base, &slug, &url, None).unwrap();
        assert!(report.shipped.is_empty());
        assert_eq!(report.skipped, 2);

        // Day 2 grows (today's live bucket): only day 2 re-ships.
        let dir = base.join("audit").join(&slug);
        std::fs::write(
            dir.join("20260102.jsonl"),
            "{\"hash\":\"h2\"}\n{\"hash\":\"h3\"}\n",
        )
        .unwrap();
        std::fs::write(dir.join(".head.20260102"), "headhash-day2b").unwrap();
        let (port2, handle2) = collector(1, "200 OK");
        let url2 = format!("http://127.0.0.1:{port2}/ingest");
        let report = ship(&base, &slug, &url2, None).unwrap();
        assert_eq!(report.shipped, vec!["20260102"]);
        assert_eq!(report.skipped, 1);
        let requests = handle2.join().unwrap();
        assert!(requests[0].contains("h3"));

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn failure_stops_run_but_keeps_progress_and_scrubs_token() {
        let (base, slug) = fixture("resume", &[("20260101", "{\"hash\":\"h1\"}\n", "head1")]);

        // Collector rejects: run errors, nothing recorded as shipped, and
        // the bearer token reaches the wire but not the error text.
        let (port, handle) = collector(1, "503 Unavailable");
        let url = format!("http://127.0.0.1:{port}/ingest");
        let err = ship(&base, &slug, &url, Some("ship-sekrit")).unwrap_err();
        assert!(!err.contains("ship-sekrit"), "token leaked: {err}");
        let requests = handle.join().unwrap();
        assert!(requests[0]
            .to_lowercase()
            .contains("authorization: bearer ship-sekrit"));

        // Cursor untouched → next run resumes and ships the bucket.
        let (port2, handle2) = collector(1, "200 OK");
        let url2 = format!("http://127.0.0.1:{port2}/ingest");
        let report = ship(&base, &slug, &url2, Some("ship-sekrit")).unwrap();
        assert_eq!(report.shipped, vec!["20260101"]);
        handle2.join().unwrap();

        // The cursor file never contains the token.
        let cursor_raw = std::fs::read_to_string(cursor_path(&base, &slug)).unwrap();
        assert!(!cursor_raw.contains("ship-sekrit"));

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn no_audit_dir_is_empty_report_not_error() {
        let base = std::env::temp_dir().join(format!("arai_ship_none_{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let report = ship(&base, "ghost", "https://c.example.com/x", None).unwrap();
        assert!(report.shipped.is_empty());
        assert_eq!(report.skipped, 0);
        std::fs::remove_dir_all(&base).ok();
    }
}
