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

use crate::config::Config;
use crate::intent::Severity;
use crate::store::{Guardrail, Store};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Genesis-line `prev_hash` sentinel: 64 hex zeros, i.e. SHA-256 length of an
/// all-zero buffer.  Marks the first line of a day-bucket's chain.  Picked
/// over the empty string so verifier diffs can spot it at a glance.
const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Map a numeric parser layer (1..=6) to a human-readable label.  Kept here
/// rather than in `parser.rs` because it's presentation, not parsing, and the
/// rustc 1.95 dead-code pass chokes on a similar function living in the
/// parser module.
pub fn layer_label(layer: u8) -> &'static str {
    match layer {
        1 => "layer-1 start-of-sentence imperative",
        2 => "layer-2 passive forbidden/required",
        3 => "layer-3 colon-separated label",
        4 => "layer-4 mid-sentence imperative",
        5 => "layer-5 use-X gated (tool / co-imperative / rules section)",
        6 => "layer-6 verb-start catch-all",
        7 => "layer-7 conditional imperative (Before/After/When/If/For \u{2026})",
        _ => "unknown",
    }
}

/// Record a hook firing to today's audit log.  Best-effort — silent on failure.
///
/// `matched` carries `(Guardrail, match_percentage)` pairs.  `prompt_preview`
/// is a short human-readable snippet of the tool input (truncated, no
/// full secret leakage) — callers produce it.
/// Record a firing, looking up per-rule severity from the store when
/// available.  Falls back to predicate-derived severity otherwise.  `db` may
/// be `None` for callers that don't hold an open connection (offline tools,
/// tests) — the log entry is still written, just without enriched severity.
///
/// `seen_set` carries the triple_ids that have already been fully injected
/// earlier in this session.  Each rule entry in the audit log records a
/// `seen_before` boolean so `arai stats` can roll up the token-economics
/// view of how often the compact-format suppression kicked in.  Pass an
/// empty set when the caller doesn't track session state (every rule is
/// recorded as `seen_before: false` → first-time injection).
#[allow(clippy::too_many_arguments)]
pub fn record_firing(
    cfg: &Config,
    event: &str,
    tool_name: &str,
    session_id: &str,
    prompt_preview: &str,
    matched: &[(Guardrail, u8)],
    decision: &str,
    db: Option<&Store>,
    seen_set: &std::collections::HashSet<i64>,
) {
    if matched.is_empty() {
        return;
    }

    let entry = json!({
        "ts": now_rfc3339(),
        "event": event,
        "tool": tool_name,
        "session": session_id,
        "prompt_preview": truncate(prompt_preview, 200),
        "decision": decision,
        "rules": matched.iter().map(|(g, pct)| {
            let severity = match db.and_then(|d| d.get_rule_intent(g.triple_id).ok().flatten()) {
                Some(intent) => intent.severity,
                None => Severity::from_predicate(&g.predicate),
            };
            let mut entry = json!({
                "triple_id": g.triple_id,
                "subject": g.subject,
                "predicate": g.predicate,
                "object": g.object,
                "source": g.file_path,
                "confidence": g.confidence,
                "match_pct": pct,
                "severity": severity.as_str(),
                "seen_before": seen_set.contains(&g.triple_id),
            });
            // Derivation trace: parser layer + label + line, so reviewers can see
            // "fired from CLAUDE.md:42 (layer-1 imperative)" without opening
            // the parser source.
            if let Some(layer) = g.layer {
                entry["layer"] = json!(layer);
                entry["layer_label"] = json!(layer_label(layer));
            }
            if let Some(line) = g.line_start {
                entry["line"] = json!(line);
            }
            entry
        }).collect::<Vec<_>>(),
    });

    seal_and_append(cfg, entry);
}

/// Record an `ARAI_DISABLED` bypass entry — written when the env var
/// short-circuits the hook so `arai stats` can still see "Arai was off
/// during this firing window".  No rule matching ran, so `rules` is empty
/// and `decision` is the literal string `"bypassed"`.  Best-effort, silent
/// on failure (matches `record_firing`'s I/O-handling).
pub fn record_bypass(cfg: &Config, event: &str, tool_name: &str, session_id: &str) {
    let entry = json!({
        "ts": now_rfc3339(),
        "event": event,
        "tool": tool_name,
        "session": session_id,
        "decision": "bypassed",
        "rules": [],
    });
    seal_and_append(cfg, entry);
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
        let lines: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
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

/// Compute today's log path, creating parent directories if needed.  On Unix
/// the directory is locked down to 0700 — the audit log contains session ids,
/// truncated prompt previews, and rule subjects.  Without this, the default
/// umask (typically 0022) leaves the per-day file world-readable on
/// multi-user systems.
fn audit_log_path(arai_base: &Path, project_slug: &str) -> Result<PathBuf, String> {
    let dir = arai_base.join("audit").join(project_slug);
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir audit: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Best-effort — if the chmod fails we still write the file (with file
        // mode 0600 below), so the leak surface is limited to file *names*.
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }
    #[cfg(windows)]
    {
        // Windows equivalent of `chmod 0700`: drop inheritance from the user
        // profile and grant the current user full control with no other
        // principals.  Done once per audit dir, gated by `.arai_acl_set` so
        // every audit-write doesn't re-shell to icacls.  Best-effort —
        // failure falls back to the inherited ACL (typically user-only on a
        // single-user profile but not pinned).
        lock_dir_windows(&dir);
    }
    let fname = format!("{}.jsonl", today_yyyymmdd());
    Ok(dir.join(fname))
}

#[cfg(windows)]
fn lock_dir_windows(dir: &Path) {
    let marker = dir.join(".arai_acl_set");
    if marker.exists() {
        return;
    }
    let username = std::env::var("USERNAME").unwrap_or_default();
    if username.is_empty() {
        return;
    }
    // /inheritance:r — strip inherited ACEs (so a relaxed profile ACL
    // doesn't grant Everyone read).
    // /grant:r USER:(OI)(CI)F — replace any existing entry for USER with
    // (Object-Inherit, Container-Inherit, Full-control).  Children created
    // afterwards inherit the same.
    let _ = std::process::Command::new("icacls.exe")
        .arg(dir)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(format!("{username}:(OI)(CI)F"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    // Write the marker only after icacls returns — if it failed (e.g. on a
    // network share that doesn't honour DACLs) we'll retry on the next
    // audit write, which is what we want.
    let _ = fs::File::create(&marker);
}

/// Open an audit-log file with restrictive permissions.  On Unix the file is
/// created with mode 0600 (owner-only read/write); on Windows the inherited
/// ACL from the parent dir applies (typically user-only by default).
fn open_audit_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

/// Append a non-firing event (Compliance, Diff-check, ad-hoc trace) to the
/// project's daily audit log.  Used by `compliance.rs` so all observability
/// lives in one place — `arai audit --event=Compliance` Just Works.
///
/// Best-effort: silently no-ops on IO failure so a borked log file never
/// blocks a hook response.
pub fn record_event(cfg: &Config, event: &str, tool_name: &str, session_id: &str, payload: Value) {
    let entry = json!({
        "ts": now_rfc3339(),
        "event": event,
        "tool": tool_name,
        "session": session_id,
        "payload": payload,
    });
    seal_and_append(cfg, entry);
}

/// Single common writer for every audit entry — adds the SHA-256 chain
/// (`prev_hash` + `hash`) so a reviewer can detect any line being edited,
/// deleted, or reordered after the fact.  Best-effort: silent on I/O failure
/// so a borked log file never blocks a hook response (matches the previous
/// behaviour of `record_firing` / `record_event`).
///
/// Chain rules:
///   - First line of a day-bucket has `prev_hash = GENESIS_HASH`.
///   - Each subsequent line's `prev_hash` is the previous line's `hash`.
///   - `hash` covers everything in the line *except* `hash` itself —
///     `prev_hash` is hashed in, so tampering with it is detected too.
///   - Canonicalisation: `serde_json::to_string` on the `prev_hash`-extended
///     entry.  `serde_json`'s default `Map` is `BTreeMap`-backed → sorted
///     keys → deterministic bytes both at write and verify time.
///
/// Head storage: per-day sidecar at
/// `{arai_base}/audit/{slug}/.head.{YYYYMMDD}` containing the last hash.
/// Acts as a cache; if the sidecar is missing or stale, `seal_and_append`
/// recovers by reading the actual last line of the day-bucket.
fn seal_and_append(cfg: &Config, mut entry: Value) {
    let arai_base = &cfg.arai_base_dir;
    let slug = cfg.project_slug();
    let log_path = match audit_log_path(arai_base, &slug) {
        Ok(p) => p,
        Err(_) => return,
    };
    let day = today_yyyymmdd();

    // Recover previous hash from the per-day sidecar; fall back to scanning
    // the last line of the day-bucket if the sidecar is missing (process
    // killed between line-write and head-write).  GENESIS_HASH starts the
    // chain on a fresh day-bucket.
    let prev_hash = read_head(arai_base, &slug, &day)
        .or_else(|| last_line_hash(&log_path))
        .unwrap_or_else(|| GENESIS_HASH.to_string());

    if let Some(obj) = entry.as_object_mut() {
        obj.insert("prev_hash".to_string(), Value::String(prev_hash.clone()));
    }
    let canonical = match serde_json::to_string(&entry) {
        Ok(s) => s,
        Err(_) => return,
    };
    let new_hash = chain_hash(&prev_hash, &canonical);

    if let Some(obj) = entry.as_object_mut() {
        obj.insert("hash".to_string(), Value::String(new_hash.clone()));
    }

    if let Ok(mut f) = open_audit_file(&log_path) {
        if writeln!(f, "{}", entry).is_ok() {
            let _ = write_head(arai_base, &slug, &day, &new_hash);
        }
    }
}

/// SHA-256(prev_hash || "|" || canonical_bytes).  Hex-encoded so the hash
/// sits cleanly inside JSON and inside the sidecar file.  The separator
/// byte rules out length-extension collisions between `prev_hash` and the
/// payload's leading characters.
fn chain_hash(prev_hash: &str, canonical: &str) -> String {
    let mut h = Sha256::new();
    h.update(prev_hash.as_bytes());
    h.update(b"|");
    h.update(canonical.as_bytes());
    let bytes = h.finalize();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn head_path(arai_base: &Path, project_slug: &str, day: &str) -> PathBuf {
    arai_base
        .join("audit")
        .join(project_slug)
        .join(format!(".head.{day}"))
}

fn read_head(arai_base: &Path, project_slug: &str, day: &str) -> Option<String> {
    let path = head_path(arai_base, project_slug, day);
    let raw = fs::read_to_string(&path).ok()?;
    let trimmed = raw.trim();
    if is_sha256_hex(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn write_head(
    arai_base: &Path,
    project_slug: &str,
    day: &str,
    new_hash: &str,
) -> std::io::Result<()> {
    let path = head_path(arai_base, project_slug, day);
    let mut opts = OpenOptions::new();
    opts.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&path)?;
    writeln!(f, "{}", new_hash)
}

fn last_line_hash(log_path: &Path) -> Option<String> {
    let file = fs::File::open(log_path).ok()?;
    let mut last = None;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if !line.trim().is_empty() {
            last = Some(line);
        }
    }
    let line = last?;
    let v: Value = serde_json::from_str(&line).ok()?;
    v.get("hash")
        .and_then(|h| h.as_str())
        .filter(|s| is_sha256_hex(s))
        .map(|s| s.to_string())
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// One line in a chain-verification report.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VerifyIssue {
    pub day: String,
    pub line_no: usize,
    pub kind: String,
    pub detail: String,
}

/// Verify the SHA-256 chain across every day-bucket for `project_slug`.
/// Walks files in calendar order.  An issue is appended for any of:
///
///   - missing `prev_hash` / `hash` fields (pre-chain legacy entries)
///   - `prev_hash` not matching the previous line's `hash` (reordering / deletion)
///   - recomputed `hash` not matching the stored value (tampered payload)
///   - malformed JSON
///
/// Returns the list of issues; an empty list means the chain verifies clean.
pub fn verify_chain(arai_base: &Path, project_slug: &str) -> Result<Vec<VerifyIssue>, String> {
    let dir = arai_base.join("audit").join(project_slug);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|e| format!("read audit dir: {e}"))?
        .filter_map(|r| r.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .collect();
    files.sort();

    let mut issues = Vec::new();
    for path in files {
        let day = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                issues.push(VerifyIssue {
                    day: day.clone(),
                    line_no: 0,
                    kind: "open_failed".to_string(),
                    detail: e.to_string(),
                });
                continue;
            }
        };
        let mut expected_prev = GENESIS_HASH.to_string();
        for (idx, line) in BufReader::new(file).lines().enumerate() {
            let line_no = idx + 1;
            let line = match line {
                Ok(l) if l.trim().is_empty() => continue,
                Ok(l) => l,
                Err(e) => {
                    issues.push(VerifyIssue {
                        day: day.clone(),
                        line_no,
                        kind: "read_failed".to_string(),
                        detail: e.to_string(),
                    });
                    break;
                }
            };
            let mut v: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => {
                    issues.push(VerifyIssue {
                        day: day.clone(),
                        line_no,
                        kind: "malformed_json".to_string(),
                        detail: e.to_string(),
                    });
                    break;
                }
            };
            let claimed_hash = v
                .get("hash")
                .and_then(|h| h.as_str())
                .map(|s| s.to_string());
            let claimed_prev = v
                .get("prev_hash")
                .and_then(|h| h.as_str())
                .map(|s| s.to_string());

            let (Some(claimed_hash), Some(claimed_prev)) = (claimed_hash, claimed_prev) else {
                issues.push(VerifyIssue {
                    day: day.clone(),
                    line_no,
                    kind: "unchained_legacy".to_string(),
                    detail: "line predates the SHA-256 chain (no prev_hash/hash fields)"
                        .to_string(),
                });
                // Best-effort recovery: keep walking but reset the expected
                // chain to whatever this line claims so we still surface a
                // mid-file break.
                expected_prev = String::new();
                continue;
            };

            if !expected_prev.is_empty() && claimed_prev != expected_prev {
                issues.push(VerifyIssue {
                    day: day.clone(),
                    line_no,
                    kind: "broken_chain".to_string(),
                    detail: format!(
                        "prev_hash={} but previous line's hash={}",
                        short(&claimed_prev),
                        short(&expected_prev)
                    ),
                });
            }

            if let Some(obj) = v.as_object_mut() {
                obj.remove("hash");
            }
            let canonical = match serde_json::to_string(&v) {
                Ok(s) => s,
                Err(e) => {
                    issues.push(VerifyIssue {
                        day: day.clone(),
                        line_no,
                        kind: "reserialise_failed".to_string(),
                        detail: e.to_string(),
                    });
                    break;
                }
            };
            let recomputed = chain_hash(&claimed_prev, &canonical);
            if recomputed != claimed_hash {
                issues.push(VerifyIssue {
                    day: day.clone(),
                    line_no,
                    kind: "tampered_payload".to_string(),
                    detail: format!(
                        "stored hash={} but recomputed={}",
                        short(&claimed_hash),
                        short(&recomputed)
                    ),
                });
            }

            expected_prev = claimed_hash;
        }
    }

    Ok(issues)
}

fn short(hash: &str) -> String {
    hash.chars().take(12).collect()
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
    let yy: i64 = if m <= 2 { (y - 1) as i64 } else { y as i64 };
    let era: i64 = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe: i64 = yy - era * 400;
    let mp: i64 = if m > 2 { m as i64 - 3 } else { m as i64 + 9 };
    let doy: i64 = (153 * mp + 2) / 5 + (d as i64 - 1);
    let doe: i64 = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_since_epoch: i64 = era * 146_097 + doe - 719_468;
    let secs: i64 = days_since_epoch * 86_400 + (h as i64) * 3600 + (mi as i64) * 60 + (s as i64);
    if secs < 0 {
        0
    } else {
        secs as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_epoch_roundtrip() {
        // Round-trip a handful of fixed epochs through format + parse.
        for epoch in [
            0u64,
            946_684_800,
            1_577_836_800,
            1_776_013_800,
            2_524_608_000,
        ] {
            let ts = format_epoch_utc(epoch);
            assert_eq!(
                parse_rfc3339_to_epoch(&ts),
                Some(epoch),
                "roundtrip failed for {ts}"
            );
        }
        // And a known reference — 2000-01-01 00:00:00 UTC is 946684800.
        assert_eq!(format_epoch_utc(946_684_800), "2000-01-01T00:00:00Z");
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

    #[cfg(unix)]
    #[test]
    fn test_audit_file_created_with_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "arai_audit_perm_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("today.jsonl");
        {
            let _f = open_audit_file(&path).expect("open audit file");
        }
        let meta = std::fs::metadata(&path).expect("stat audit file");
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "audit log should be 0600 on Unix (got {mode:o})"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_chain_hash_deterministic_and_separator_safe() {
        // Same inputs → same hash.
        let a = chain_hash(GENESIS_HASH, r#"{"a":1}"#);
        let b = chain_hash(GENESIS_HASH, r#"{"a":1}"#);
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        // Concatenating prev_hash and payload without a separator would let
        // these two inputs collide.  With the `|` separator they must not.
        let split_a = chain_hash("aa", "bb");
        let split_b = chain_hash("aab", "b");
        assert_ne!(
            split_a, split_b,
            "separator should prevent length-extension collision"
        );
    }

    fn fresh_tmp_base(label: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!(
            "arai_audit_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn test_config(base: &std::path::Path) -> Config {
        Config {
            project_root: base.to_path_buf(),
            home_dir: base.to_path_buf(),
            arai_base_dir: base.to_path_buf(),
            extra_sources: Vec::new(),
            guardrails_mode: "advise".to_string(),
            llm_command: None,
            api_url: None,
            api_key_env: None,
            api_model: None,
        }
    }

    #[test]
    fn test_verify_chain_passes_on_clean_log() {
        let base = fresh_tmp_base("verify_clean");
        let cfg = test_config(&base);
        for i in 0..3 {
            record_event(&cfg, "TestEvent", "Bash", "sess", json!({"i": i}));
        }
        let issues = verify_chain(&base, &cfg.project_slug()).unwrap();
        assert!(
            issues.is_empty(),
            "chain should verify cleanly; got: {:?}",
            issues
        );
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn test_verify_chain_detects_tampered_payload() {
        let base = fresh_tmp_base("verify_tamper");
        let cfg = test_config(&base);
        record_event(&cfg, "TestEvent", "Bash", "sess", json!({"i": 1}));
        record_event(&cfg, "TestEvent", "Bash", "sess", json!({"i": 2}));
        // Hand-edit the log: tweak the payload of the second line without
        // touching its hash.  Verification must catch this.
        let log = base
            .join("audit")
            .join(cfg.project_slug())
            .join(format!("{}.jsonl", today_yyyymmdd()));
        let contents = std::fs::read_to_string(&log).unwrap();
        let tampered = contents.replace(r#""i":2"#, r#""i":99"#);
        assert_ne!(contents, tampered, "test setup: replacement must apply");
        std::fs::write(&log, tampered).unwrap();
        let issues = verify_chain(&base, &cfg.project_slug()).unwrap();
        assert!(
            issues.iter().any(|i| i.kind == "tampered_payload"),
            "expected tampered_payload issue, got: {:?}",
            issues
        );
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn test_verify_chain_detects_deleted_line() {
        let base = fresh_tmp_base("verify_delete");
        let cfg = test_config(&base);
        for i in 0..4 {
            record_event(&cfg, "TestEvent", "Bash", "sess", json!({"i": i}));
        }
        let log = base
            .join("audit")
            .join(cfg.project_slug())
            .join(format!("{}.jsonl", today_yyyymmdd()));
        // Drop the second line — verifier should flag a broken chain at
        // the line that previously followed it.
        let contents = std::fs::read_to_string(&log).unwrap();
        let kept: Vec<&str> = contents
            .lines()
            .enumerate()
            .filter(|(idx, _)| *idx != 1)
            .map(|(_, l)| l)
            .collect();
        std::fs::write(&log, format!("{}\n", kept.join("\n"))).unwrap();
        let issues = verify_chain(&base, &cfg.project_slug()).unwrap();
        assert!(
            issues.iter().any(|i| i.kind == "broken_chain"),
            "expected broken_chain issue, got: {:?}",
            issues
        );
        std::fs::remove_dir_all(&base).ok();
    }

    #[cfg(unix)]
    #[test]
    fn test_audit_dir_locked_to_0700() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!(
            "arai_audit_dir_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&base).unwrap();
        let _path = audit_log_path(&base, "test-slug").expect("compute audit path");
        let dir = base.join("audit").join("test-slug");
        let mode = std::fs::metadata(&dir)
            .expect("stat audit dir")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o700,
            "audit dir should be 0700 on Unix (got {mode:o})"
        );
        std::fs::remove_dir_all(&base).ok();
    }
}
