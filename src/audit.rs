//! Local audit log of every rule firing.
//!
//! Ārai writes one JSONL line per hook event where at least one guardrail
//! matched the tool call.  The log lives at
//!   `{arai_base}/audit/{project_slug}/{YYYYMMDD}.jsonl`
//! and is append-only.  Nothing is sent upstream — this is separate from
//! the anonymous usage telemetry (`telemetry.rs`), which is opt-out and
//! aggregates firing counts without project context.
//!
//! The audit log is what a user / compliance reviewer inspects to answer:
//!   "what rules fired, against which prompts, at what times, on which tools"
//!
//! CLI: `arai audit` — tail today.
//!      `arai audit --since=7d` — window.
//!      `arai audit --tool=Bash` — filter.
//!      `arai audit --json` — JSON stream.

use crate::store::Guardrail;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Record a hook firing to today's audit log.  Best-effort — silent on failure.
///
/// `matched` carries `(Guardrail, match_percentage)` pairs.  `prompt_preview`
/// is a short human-readable snippet of the tool input (truncated, no
/// full secret leakage) — callers produce it.
pub fn record_firing(
    arai_base: &Path,
    project_slug: &str,
    event: &str,
    tool_name: &str,
    session_id: &str,
    prompt_preview: &str,
    matched: &[(Guardrail, u8)],
    decision: &str,
) {
    if matched.is_empty() {
        return;
    }
    let log_path = match audit_log_path(arai_base, project_slug) {
        Ok(p) => p,
        Err(_) => return,
    };

    let entry = json!({
        "ts": now_rfc3339(),
        "event": event,
        "tool": tool_name,
        "session": session_id,
        "prompt_preview": truncate(prompt_preview, 200),
        "decision": decision,
        "rules": matched.iter().map(|(g, pct)| json!({
            "triple_id": g.triple_id,
            "subject": g.subject,
            "predicate": g.predicate,
            "object": g.object,
            "source": g.file_path,
            "confidence": g.confidence,
            "match_pct": pct,
        })).collect::<Vec<_>>(),
    });

    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let _ = writeln!(f, "{}", entry);
    }
}

/// Read firings matching the filter from the project's audit directory.
/// Walks files in reverse chronological order so newest entries come first.
pub fn query(
    arai_base: &Path,
    project_slug: &str,
    since_epoch_secs: Option<u64>,
    tool_filter: Option<&str>,
    event_filter: Option<&str>,
    max_entries: usize,
) -> Result<Vec<Value>, String> {
    let dir = arai_base.join("audit").join(project_slug);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    // Collect day-file paths sorted descending by filename (YYYYMMDD.jsonl).
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|e| format!("read audit dir: {e}"))?
        .filter_map(|r| r.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .collect();
    files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

    let mut out: Vec<Value> = Vec::new();
    for path in files {
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        // Read lines into a buffer then reverse so newest-in-file comes first.
        let lines: Vec<String> = BufReader::new(file)
            .lines()
            .filter_map(|l| l.ok())
            .collect();
        for line in lines.into_iter().rev() {
            if out.len() >= max_entries {
                return Ok(out);
            }
            let entry: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(since) = since_epoch_secs {
                let ts = entry.get("ts").and_then(|v| v.as_str()).unwrap_or("");
                if parse_rfc3339_to_epoch(ts).unwrap_or(0) < since {
                    return Ok(out);
                }
            }
            if let Some(t) = tool_filter {
                if entry.get("tool").and_then(|v| v.as_str()).unwrap_or("") != t {
                    continue;
                }
            }
            if let Some(e) = event_filter {
                if entry.get("event").and_then(|v| v.as_str()).unwrap_or("") != e {
                    continue;
                }
            }
            out.push(entry);
        }
    }
    Ok(out)
}

/// Compute today's log path, creating parent directories if needed.
fn audit_log_path(arai_base: &Path, project_slug: &str) -> Result<PathBuf, String> {
    let dir = arai_base.join("audit").join(project_slug);
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir audit: {e}"))?;
    let fname = format!("{}.jsonl", today_yyyymmdd());
    Ok(dir.join(fname))
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let trimmed: String = s.chars().take(n).collect();
        format!("{trimmed}…")
    }
}

fn now_rfc3339() -> String {
    // Minimal RFC3339-ish in UTC: YYYY-MM-DDTHH:MM:SSZ.  Enough for sort +
    // human readability without pulling a time crate.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_epoch_utc(secs)
}

fn today_yyyymmdd() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d, _, _, _) = epoch_to_civil(secs);
    format!("{:04}{:02}{:02}", y, m, d)
}

fn format_epoch_utc(secs: u64) -> String {
    let (y, mo, d, h, mi, se) = epoch_to_civil(secs);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, mi, se)
}

/// Howard Hinnant's civil-from-days algorithm, adapted for epoch seconds.
/// Returns (year, month, day, hour, minute, second) in UTC.
fn epoch_to_civil(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let day_secs = 86_400i64;
    let z = (secs as i64) / day_secs + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64 + era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { y + 1 } else { y };
    let sod = (secs % 86_400) as u32;
    let h = sod / 3600;
    let mi = (sod % 3600) / 60;
    let se = sod % 60;
    (year, m, d, h, mi, se)
}

fn parse_rfc3339_to_epoch(ts: &str) -> Option<u64> {
    // Parse strict "YYYY-MM-DDTHH:MM:SSZ" we emit ourselves.  Not a full RFC
    // 3339 parser — just good enough for our own log.
    if ts.len() != 20 || !ts.ends_with('Z') {
        return None;
    }
    let year: i32 = ts.get(0..4)?.parse().ok()?;
    let mo: u32 = ts.get(5..7)?.parse().ok()?;
    let d: u32 = ts.get(8..10)?.parse().ok()?;
    let h: u32 = ts.get(11..13)?.parse().ok()?;
    let mi: u32 = ts.get(14..16)?.parse().ok()?;
    let se: u32 = ts.get(17..19)?.parse().ok()?;
    Some(civil_to_epoch(year, mo, d, h, mi, se))
}

fn civil_to_epoch(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> u64 {
    let yy = if m <= 2 { y - 1 } else { y };
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = (yy - era * 400) as i64;
    let mp = if m > 2 { m as i64 - 3 } else { m as i64 + 9 };
    let doy = (153 * mp + 2) / 5 + (d as i64 - 1);
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_since_epoch = era * 146_097 + doe - 719_468;
    let secs = days_since_epoch * 86_400
        + (h as i64) * 3600
        + (mi as i64) * 60
        + (s as i64);
    if secs < 0 { 0 } else { secs as u64 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_epoch_roundtrip() {
        // 2026-04-19T17:30:00Z = 1776013800
        let ts = format_epoch_utc(1_776_013_800);
        assert_eq!(ts, "2026-04-19T17:30:00Z");
        assert_eq!(parse_rfc3339_to_epoch(&ts), Some(1_776_013_800));
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("short", 200), "short");
        let long = "a".repeat(250);
        let trimmed = truncate(&long, 200);
        assert!(trimmed.ends_with('…'));
        assert_eq!(trimmed.chars().count(), 201);
    }

    #[test]
    fn test_today_yyyymmdd_shape() {
        let s = today_yyyymmdd();
        assert_eq!(s.len(), 8);
        assert!(s.chars().all(|c| c.is_ascii_digit()));
    }
}
