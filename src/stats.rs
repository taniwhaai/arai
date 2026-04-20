//! Aggregate views over the local audit log.
//!
//! `arai stats` reads the same per-project JSONL that `arai audit` tails,
//! and produces summary counts: which rules fire most, which tools attract
//! the most firings, activity per day.  Separate from anonymous telemetry —
//! stats stay on the user's machine.

use crate::config::Config;
use crate::audit;
use serde_json::Value;
use std::collections::HashMap;

/// Aggregate summary of audit log entries.
#[derive(Debug, Default)]
pub struct Stats {
    pub total_firings: usize,
    pub window_start: Option<String>,
    pub window_end: Option<String>,
    pub by_rule: Vec<(String, usize)>,
    pub by_tool: Vec<(String, usize)>,
    pub by_event: Vec<(String, usize)>,
    pub by_day: Vec<(String, usize)>,
}

/// Compute aggregate stats over audit entries.  Entries are the raw JSON
/// values emitted by `audit::query`; this function never re-reads the log.
pub fn compute(entries: &[Value]) -> Stats {
    let mut s = Stats {
        total_firings: entries.len(),
        ..Stats::default()
    };

    let mut rule_counts: HashMap<String, usize> = HashMap::new();
    let mut tool_counts: HashMap<String, usize> = HashMap::new();
    let mut event_counts: HashMap<String, usize> = HashMap::new();
    let mut day_counts: HashMap<String, usize> = HashMap::new();

    for entry in entries {
        let ts = entry.get("ts").and_then(|v| v.as_str()).unwrap_or("");
        if !ts.is_empty() {
            // Newer entries come first in `entries`, so window_end is the
            // first non-empty timestamp and window_start is the last.
            if s.window_end.is_none() {
                s.window_end = Some(ts.to_string());
            }
            s.window_start = Some(ts.to_string());
            if ts.len() >= 10 {
                *day_counts.entry(ts[..10].to_string()).or_insert(0) += 1;
            }
        }

        let tool = entry.get("tool").and_then(|v| v.as_str()).unwrap_or("");
        if !tool.is_empty() {
            *tool_counts.entry(tool.to_string()).or_insert(0) += 1;
        }

        let ev = entry.get("event").and_then(|v| v.as_str()).unwrap_or("");
        if !ev.is_empty() {
            *event_counts.entry(ev.to_string()).or_insert(0) += 1;
        }

        if let Some(rules) = entry.get("rules").and_then(|v| v.as_array()) {
            for r in rules {
                let subj = r.get("subject").and_then(|v| v.as_str()).unwrap_or("");
                let pred = r.get("predicate").and_then(|v| v.as_str()).unwrap_or("");
                let obj = r.get("object").and_then(|v| v.as_str()).unwrap_or("");
                if subj.is_empty() && pred.is_empty() && obj.is_empty() {
                    continue;
                }
                let key = format!("{subj} {pred}: {obj}");
                *rule_counts.entry(key).or_insert(0) += 1;
            }
        }
    }

    s.by_rule = sort_desc(rule_counts);
    s.by_tool = sort_desc(tool_counts);
    s.by_event = sort_desc(event_counts);
    s.by_day = {
        let mut v: Vec<(String, usize)> = day_counts.into_iter().collect();
        // chronological ascending — easier to read as a timeline
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    s
}

fn sort_desc(m: HashMap<String, usize>) -> Vec<(String, usize)> {
    let mut v: Vec<(String, usize)> = m.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v
}

/// CLI entry point: load audit entries, aggregate, print.
pub fn run(
    cfg: &Config,
    since_epoch_secs: Option<u64>,
    top: usize,
    json: bool,
) -> Result<(), String> {
    let entries = audit::query(
        &cfg.arai_base_dir,
        &cfg.project_slug(),
        since_epoch_secs,
        None,
        None,
        usize::MAX,
    )?;
    let stats = compute(&entries);

    if json {
        let out = serde_json::json!({
            "total_firings": stats.total_firings,
            "window_start": stats.window_start,
            "window_end": stats.window_end,
            "by_rule": stats.by_rule.iter().map(|(k, v)| serde_json::json!({"rule": k, "count": v})).collect::<Vec<_>>(),
            "by_tool": stats.by_tool.iter().map(|(k, v)| serde_json::json!({"tool": k, "count": v})).collect::<Vec<_>>(),
            "by_event": stats.by_event.iter().map(|(k, v)| serde_json::json!({"event": k, "count": v})).collect::<Vec<_>>(),
            "by_day": stats.by_day.iter().map(|(k, v)| serde_json::json!({"day": k, "count": v})).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
        return Ok(());
    }

    print_table(&stats, top);
    Ok(())
}

fn print_table(stats: &Stats, top: usize) {
    if stats.total_firings == 0 {
        println!("No audit entries.  Rules haven't fired yet, or --since excluded everything.");
        return;
    }

    println!("Arai stats");
    println!("  Total firings: {}", stats.total_firings);
    if let (Some(start), Some(end)) = (&stats.window_start, &stats.window_end) {
        if start == end {
            println!("  Window:        {start}");
        } else {
            println!("  Window:        {start}  →  {end}");
        }
    }
    println!();

    print_section("Top rules", &stats.by_rule, top);
    print_section("By tool", &stats.by_tool, top);
    print_section("By event", &stats.by_event, top);
    print_section("By day", &stats.by_day, top);
}

fn print_section(title: &str, rows: &[(String, usize)], top: usize) {
    if rows.is_empty() {
        return;
    }
    println!("{title}");
    let max_count = rows.iter().map(|(_, c)| *c).max().unwrap_or(1).max(1);
    let shown = rows.iter().take(top);
    for (label, count) in shown {
        let bar_width = 20usize.min((count * 20) / max_count.max(1));
        let bar: String = "█".repeat(bar_width);
        println!("  {:>5}  {:<20}  {}", count, bar, label);
    }
    if rows.len() > top {
        println!("        … {} more", rows.len() - top);
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn e(ts: &str, tool: &str, event: &str, subj: &str, pred: &str, obj: &str) -> Value {
        json!({
            "ts": ts,
            "tool": tool,
            "event": event,
            "rules": [{
                "subject": subj,
                "predicate": pred,
                "object": obj,
            }],
        })
    }

    #[test]
    fn test_empty_stats() {
        let stats = compute(&[]);
        assert_eq!(stats.total_firings, 0);
        assert!(stats.by_rule.is_empty());
    }

    #[test]
    fn test_rule_count_aggregation() {
        let entries = vec![
            e("2026-04-20T10:00:00Z", "Bash", "PreToolUse", "git", "never", "force-push"),
            e("2026-04-20T11:00:00Z", "Bash", "PreToolUse", "git", "never", "force-push"),
            e("2026-04-20T12:00:00Z", "Write", "PreToolUse", "alembic", "never", "hand-write"),
        ];
        let stats = compute(&entries);
        assert_eq!(stats.total_firings, 3);
        assert_eq!(stats.by_rule[0].0, "git never: force-push");
        assert_eq!(stats.by_rule[0].1, 2);
        assert_eq!(stats.by_rule[1].1, 1);
    }

    #[test]
    fn test_tool_and_event_counts() {
        let entries = vec![
            e("2026-04-20T10:00:00Z", "Bash", "PreToolUse", "a", "b", "c"),
            e("2026-04-20T10:01:00Z", "Bash", "PostToolUse", "a", "b", "c"),
            e("2026-04-20T10:02:00Z", "Write", "PreToolUse", "a", "b", "c"),
        ];
        let stats = compute(&entries);
        assert_eq!(stats.by_tool[0], ("Bash".to_string(), 2));
        assert_eq!(stats.by_tool[1], ("Write".to_string(), 1));
        assert_eq!(stats.by_event.iter().find(|(k, _)| k == "PreToolUse").unwrap().1, 2);
    }

    #[test]
    fn test_by_day_ordering() {
        // Newer-first in input (matches audit::query output)
        let entries = vec![
            e("2026-04-22T10:00:00Z", "Bash", "PreToolUse", "a", "b", "c"),
            e("2026-04-20T10:00:00Z", "Bash", "PreToolUse", "a", "b", "c"),
            e("2026-04-20T11:00:00Z", "Bash", "PreToolUse", "a", "b", "c"),
        ];
        let stats = compute(&entries);
        // by_day is chronological ascending
        assert_eq!(stats.by_day[0].0, "2026-04-20");
        assert_eq!(stats.by_day[0].1, 2);
        assert_eq!(stats.by_day[1].0, "2026-04-22");
        // window_start is oldest (last iterated), window_end is newest (first iterated)
        assert_eq!(stats.window_end.as_deref(), Some("2026-04-22T10:00:00Z"));
        assert_eq!(stats.window_start.as_deref(), Some("2026-04-20T11:00:00Z"));
    }

    #[test]
    fn test_rule_tiebreak_alphabetical() {
        let entries = vec![
            e("2026-04-20T10:00:00Z", "Bash", "PreToolUse", "zebra", "never", "x"),
            e("2026-04-20T11:00:00Z", "Bash", "PreToolUse", "alpha", "never", "x"),
        ];
        let stats = compute(&entries);
        // equal counts → sorted alphabetically
        assert_eq!(stats.by_rule[0].0, "alpha never: x");
        assert_eq!(stats.by_rule[1].0, "zebra never: x");
    }
}
