//! Aggregate views over the local audit log.
//!
//! `arai stats` reads the same per-project JSONL that `arai audit` tails,
//! and produces summary counts: which rules fire most, which tools attract
//! the most firings, activity per day.  Separate from anonymous telemetry —
//! stats stay on the user's machine.
//!
//! Compliance roll-up: when PostToolUse correlation has produced verdicts,
//! `compute()` joins them against the Pre firings via `triple_id` and reports
//! per-rule obeyed/ignored/unclear counts plus a "compliance ratio"
//! `obeyed / (obeyed + ignored)`.  This is the single most actionable
//! number for a maintainer evaluating Arai on a real project — it tells
//! you which rules the model honours and which ones it routes around.

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
    /// Per-rule compliance roll-up.  Empty when the audit log has no
    /// `Compliance` events yet (typical for projects that have only ever
    /// run with the audit log shipping but no PostToolUse handler wired up,
    /// or projects where no Pre/Post pair has occurred).
    pub by_rule_compliance: Vec<RuleCompliance>,
    /// Token-economics roll-up — calibrated estimates of saved + spent
    /// tokens.  Not measurements; the constants are documented and
    /// labelled as estimates everywhere.
    pub token_economics: TokenEconomics,
}

/// Calibrated estimate of Arai's effect on token burn over the audit window.
///
/// Two streams contribute: the *suppression* stream (counted directly from
/// `seen_before` flags on firings — repeat injections that emit a compact
/// one-liner instead of the full rule payload) and the *counterfactual*
/// stream (each `obeyed` Compliance verdict, weighted by the original Pre's
/// severity, attributing the avoided cost of a mistake we believe we
/// prevented).
///
/// **These are estimates, not measurements.**  The compact-form delta is a
/// rough average from sampling a few real rules; the counterfactual
/// constants are conservative bounds on what "fix the mess" cycles
/// typically cost.  Calibration constants live in `compute()` so they can
/// move without rewriting the audit log.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct TokenEconomics {
    /// Re-firings of a rule that already had its full context injected
    /// earlier in the same session — these emit a compact form.
    pub suppressed_repeats: usize,
    /// `obeyed` Compliance verdicts where the original Pre was `block`
    /// severity — i.e. denials the model honored.
    pub blocked_obeyed: usize,
    /// `obeyed` Compliance verdicts where the original Pre was advisory
    /// (`warn` or `inform`) — the model complied without being denied.
    pub advisory_obeyed: usize,
    /// Sum of the three streams under the documented calibration constants.
    /// Treat as an order-of-magnitude reading, not a precise number.
    pub estimated_tokens_saved: usize,
}

/// Per-rule compliance record, joined across Pre firings (which carry
/// subject/predicate/object) and Compliance events (which carry triple_id +
/// outcome).  `fires` is the number of Pre firings; `obeyed`/`ignored`/
/// `unclear` are verdict counts; `ratio` is `obeyed / (obeyed + ignored)`
/// or `None` when the denominator is zero.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuleCompliance {
    pub triple_id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub fires: usize,
    pub obeyed: usize,
    pub ignored: usize,
    pub unclear: usize,
    /// Ratio in `[0.0, 1.0]`.  `None` when neither `obeyed` nor `ignored`
    /// is non-zero — there's no signal yet to compute a ratio from.
    pub ratio: Option<f64>,
}

/// Calibration constants for `TokenEconomics`.  Documented here so they
/// move atomically and the audit log never carries derived numbers that
/// depend on a constant the user can't re-derive from the data.
///
/// - **Per suppression**: the byte-count delta between a full firing and
///   a compact one-liner is roughly 200 chars (≈50 tokens at typical
///   tokenisation).  Sampled by hand on a few representative rules.
/// - **Per blocked-then-obeyed**: a denied destructive action that the
///   model would otherwise have run typically costs 1–5K tokens of
///   recovery (revert files, undo migrations, rollback push).  We pick
///   2K as a conservative midpoint — over-claiming here would be the
///   easy mistake to make.
/// - **Per advisory-obeyed**: a warned-and-complied action saves less
///   because we don't know the model wouldn't have done the right thing
///   anyway.  500 tokens captures the value of the warning without
///   over-attributing.
const TOKENS_PER_SUPPRESSION: usize = 50;
const TOKENS_PER_BLOCKED_OBEYED: usize = 2000;
const TOKENS_PER_ADVISORY_OBEYED: usize = 500;

/// Compute aggregate stats over audit entries.  Entries are the raw JSON
/// values emitted by `audit::query`; this function never re-reads the log.
pub fn compute(entries: &[Value]) -> Stats {
    let mut s = Stats {
        total_firings: 0,
        ..Stats::default()
    };

    let mut rule_counts: HashMap<String, usize> = HashMap::new();
    let mut tool_counts: HashMap<String, usize> = HashMap::new();
    let mut event_counts: HashMap<String, usize> = HashMap::new();
    let mut day_counts: HashMap<String, usize> = HashMap::new();

    // Token-economics counters — accumulated as we walk the entries.
    let mut suppressed_repeats: usize = 0;
    let mut blocked_obeyed: usize = 0;
    let mut advisory_obeyed: usize = 0;

    // Per-triple-id accumulators for the compliance roll-up.  We discover
    // SPO via Pre firings (which carry subject/predicate/object) and
    // outcomes via Compliance events (which carry triple_id + outcome).
    //
    // `outcomes_by_pre` is keyed on `(session, pre_ts, triple_id)` so that a
    // single Pre firing produces one rolled-up verdict regardless of how
    // many Posts correlated against it inside the 5-minute window.  Without
    // this dedupe, an unrelated Post that didn't trigger the rule still
    // wrote an Obeyed Compliance entry against the original Pre, inflating
    // the denominator of the ratio.  See arai#37.
    let mut spo_by_id: HashMap<i64, (String, String, String)> = HashMap::new();
    let mut fires_by_id: HashMap<i64, usize> = HashMap::new();
    let mut outcomes_by_pre: HashMap<(String, String, i64), Vec<(String, String)>> = HashMap::new();

    for entry in entries {
        let ts = entry.get("ts").and_then(|v| v.as_str()).unwrap_or("");
        let ev = entry.get("event").and_then(|v| v.as_str()).unwrap_or("");
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

        // Compliance events live in their own bucket: they don't count as
        // firings, they don't carry tool context worth tallying separately,
        // and their rules live under `payload.rules[]` not `rules[]`.
        //
        // We collect *all* observed outcomes per `(session, pre_ts,
        // triple_id)` and resolve them after the scan — a single Pre
        // firing produces at most one rolled-up verdict (first-definitive-
        // wins), regardless of how many Posts correlated against it.
        if ev == "Compliance" {
            let session = entry
                .get("session")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let post_ts = ts.to_string();
            if let Some(rules) = entry
                .get("payload")
                .and_then(|p| p.get("rules"))
                .and_then(|r| r.as_array())
            {
                for r in rules {
                    let triple_id = r.get("triple_id").and_then(|v| v.as_i64()).unwrap_or(-1);
                    if triple_id < 0 {
                        continue;
                    }
                    let pre_ts = r
                        .get("pre_ts")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let outcome = r
                        .get("outcome")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    // Token-economics: weight obeyed verdicts by the
                    // original Pre's severity.  Denied-and-honored is the
                    // high-confidence saving (the model would otherwise
                    // have run the destructive command); advisory-and-
                    // honored is lower-confidence (we don't know the
                    // model wouldn't have done the right thing anyway).
                    if outcome == "obeyed" {
                        let severity = r.get("severity").and_then(|v| v.as_str()).unwrap_or("");
                        match severity {
                            "block" => blocked_obeyed += 1,
                            "warn" | "inform" => advisory_obeyed += 1,
                            _ => {}
                        }
                    }

                    outcomes_by_pre
                        .entry((session.clone(), pre_ts, triple_id))
                        .or_default()
                        .push((post_ts.clone(), outcome));
                }
            }
            continue;
        }

        // Non-Compliance events: count as firings, accumulate top-rule and
        // tool/event histograms.
        s.total_firings += 1;
        if !ev.is_empty() {
            *event_counts.entry(ev.to_string()).or_insert(0) += 1;
        }
        let tool = entry.get("tool").and_then(|v| v.as_str()).unwrap_or("");
        if !tool.is_empty() {
            *tool_counts.entry(tool.to_string()).or_insert(0) += 1;
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

                // Token-economics: a `seen_before` rule was emitted in the
                // compact form, so the model didn't re-read the full
                // payload.  Older audit entries don't carry the field —
                // treat absent as `false` (first-time injection, no
                // saving claimed).
                if r.get("seen_before").and_then(|v| v.as_bool()).unwrap_or(false) {
                    suppressed_repeats += 1;
                }

                // Compliance roll-up bookkeeping: remember the SPO for this
                // triple_id (Compliance events only carry the id), and bump
                // the firing counter.  Only Pre firings count toward
                // `fires` — Post events would double-count the same call.
                let triple_id = r.get("triple_id").and_then(|v| v.as_i64()).unwrap_or(-1);
                if triple_id >= 0 {
                    spo_by_id
                        .entry(triple_id)
                        .or_insert_with(|| (subj.to_string(), pred.to_string(), obj.to_string()));
                    if ev == "PreToolUse" {
                        *fires_by_id.entry(triple_id).or_insert(0) += 1;
                    }
                }
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

    // Resolve per-Pre outcome lists into a single verdict per Pre firing.
    // First-definitive-wins: scan in post-ts order, take the first
    // `obeyed` or `ignored`; if none appear, the verdict is `unclear`.
    // The first observation after the warning is what tells us whether
    // the model honored the rule on that specific call — later commands
    // are evidence about later state, not the original Pre.
    let mut outcomes_by_id: HashMap<i64, (usize, usize, usize)> = HashMap::new();
    for ((_session, _pre_ts, triple_id), mut occurrences) in outcomes_by_pre {
        occurrences.sort_by(|a, b| a.0.cmp(&b.0));
        let resolved = occurrences
            .iter()
            .find(|(_, o)| matches!(o.as_str(), "obeyed" | "ignored"))
            .map(|(_, o)| o.as_str())
            .unwrap_or("unclear");
        let entry_o = outcomes_by_id.entry(triple_id).or_insert((0, 0, 0));
        match resolved {
            "obeyed" => entry_o.0 += 1,
            "ignored" => entry_o.1 += 1,
            _ => entry_o.2 += 1,
        }
    }

    // Build the compliance roll-up: every triple_id we saw in a Pre firing
    // OR a Compliance event becomes a row.  This includes rules that fired
    // but produced no Compliance verdict (Pre with no matching Post in the
    // correlation window).
    let mut compliance: Vec<RuleCompliance> = spo_by_id
        .into_iter()
        .map(|(triple_id, (subject, predicate, object))| {
            let fires = *fires_by_id.get(&triple_id).unwrap_or(&0);
            let (obeyed, ignored, unclear) = outcomes_by_id
                .get(&triple_id)
                .copied()
                .unwrap_or((0, 0, 0));
            let denom = obeyed + ignored;
            let ratio = if denom > 0 {
                Some(obeyed as f64 / denom as f64)
            } else {
                None
            };
            RuleCompliance {
                triple_id,
                subject,
                predicate,
                object,
                fires,
                obeyed,
                ignored,
                unclear,
                ratio,
            }
        })
        .collect();
    // Sort by fires desc, then by ignored desc (floutings float to the top
    // for equal fire counts), then alphabetical for stability.
    compliance.sort_by(|a, b| {
        b.fires
            .cmp(&a.fires)
            .then_with(|| b.ignored.cmp(&a.ignored))
            .then_with(|| a.subject.cmp(&b.subject))
            .then_with(|| a.object.cmp(&b.object))
    });
    s.by_rule_compliance = compliance;

    let estimated_tokens_saved = suppressed_repeats * TOKENS_PER_SUPPRESSION
        + blocked_obeyed * TOKENS_PER_BLOCKED_OBEYED
        + advisory_obeyed * TOKENS_PER_ADVISORY_OBEYED;
    s.token_economics = TokenEconomics {
        suppressed_repeats,
        blocked_obeyed,
        advisory_obeyed,
        estimated_tokens_saved,
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
    by_rule_only: bool,
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
        let mut out = serde_json::json!({
            "total_firings": stats.total_firings,
            "window_start": stats.window_start,
            "window_end": stats.window_end,
            "by_rule_compliance": stats.by_rule_compliance,
            "token_economics": stats.token_economics,
        });
        if !by_rule_only {
            out["by_rule"] = serde_json::Value::Array(
                stats
                    .by_rule
                    .iter()
                    .map(|(k, v)| serde_json::json!({"rule": k, "count": v}))
                    .collect(),
            );
            out["by_tool"] = serde_json::Value::Array(
                stats
                    .by_tool
                    .iter()
                    .map(|(k, v)| serde_json::json!({"tool": k, "count": v}))
                    .collect(),
            );
            out["by_event"] = serde_json::Value::Array(
                stats
                    .by_event
                    .iter()
                    .map(|(k, v)| serde_json::json!({"event": k, "count": v}))
                    .collect(),
            );
            out["by_day"] = serde_json::Value::Array(
                stats
                    .by_day
                    .iter()
                    .map(|(k, v)| serde_json::json!({"day": k, "count": v}))
                    .collect(),
            );
        }
        println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
        return Ok(());
    }

    print_table(&stats, top, by_rule_only);
    Ok(())
}

fn print_table(stats: &Stats, top: usize, by_rule_only: bool) {
    if stats.total_firings == 0 && stats.by_rule_compliance.is_empty() {
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

    if by_rule_only {
        print_compliance_section(&stats.by_rule_compliance, top);
        print_token_economics(&stats.token_economics);
        return;
    }

    print_section("Top rules", &stats.by_rule, top);
    print_compliance_section(&stats.by_rule_compliance, top);
    print_token_economics(&stats.token_economics);
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

fn print_token_economics(t: &TokenEconomics) {
    // Skip the section entirely when there's nothing to report — avoids
    // bragging "0 tokens saved" on first runs.
    if t.suppressed_repeats == 0 && t.blocked_obeyed == 0 && t.advisory_obeyed == 0 {
        return;
    }
    println!("Token economics (estimates)");
    if t.suppressed_repeats > 0 {
        let saved = t.suppressed_repeats * TOKENS_PER_SUPPRESSION;
        println!(
            "  {:>5}  repeat-injection suppressions  (~{} tokens, {} ea.)",
            t.suppressed_repeats, saved, TOKENS_PER_SUPPRESSION,
        );
    }
    if t.blocked_obeyed > 0 {
        let saved = t.blocked_obeyed * TOKENS_PER_BLOCKED_OBEYED;
        println!(
            "  {:>5}  denied-and-honored mistakes    (~{} tokens, {} ea.)",
            t.blocked_obeyed, saved, TOKENS_PER_BLOCKED_OBEYED,
        );
    }
    if t.advisory_obeyed > 0 {
        let saved = t.advisory_obeyed * TOKENS_PER_ADVISORY_OBEYED;
        println!(
            "  {:>5}  advised-and-honored events     (~{} tokens, {} ea.)",
            t.advisory_obeyed, saved, TOKENS_PER_ADVISORY_OBEYED,
        );
    }
    println!(
        "         total estimated tokens saved:  ~{}",
        t.estimated_tokens_saved,
    );
    println!("         (calibrated estimates, not measurements — see CLAUDE.md)");
    println!();
}

fn print_compliance_section(rows: &[RuleCompliance], top: usize) {
    if rows.is_empty() {
        return;
    }
    let any_outcomes = rows.iter().any(|r| r.obeyed + r.ignored + r.unclear > 0);
    println!("Per-rule compliance");
    if !any_outcomes {
        println!("  (no Compliance events yet — Pre/Post correlation produces these on PostToolUse)");
    }
    println!(
        "  {:>5} {:>6} {:>7} {:>7} {:>7}  rule",
        "fires", "obeyed", "ignored", "unclear", "ratio",
    );
    for r in rows.iter().take(top) {
        let ratio = match r.ratio {
            Some(v) => format!("{:>6.0}%", v * 100.0),
            None => "  —  ".to_string(),
        };
        // Visual nudge for rules the model is routing around.
        let flag = match r.ratio {
            Some(v) if v < 0.6 && (r.obeyed + r.ignored) >= 2 => " ⚠",
            _ => "",
        };
        println!(
            "  {:>5} {:>6} {:>7} {:>7} {:>7}  {} {}: {}{}",
            r.fires, r.obeyed, r.ignored, r.unclear, ratio,
            r.subject, r.predicate, r.object, flag,
        );
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

    fn pre_firing(ts: &str, tool: &str, triple_id: i64, subj: &str, pred: &str, obj: &str) -> Value {
        json!({
            "ts": ts,
            "tool": tool,
            "event": "PreToolUse",
            "session": "s1",
            "rules": [{
                "triple_id": triple_id,
                "subject": subj,
                "predicate": pred,
                "object": obj,
            }],
        })
    }

    /// Build a Compliance event correlated against a specific Pre firing.
    /// `pre_ts` must match the timestamp of the Pre this Post is responding
    /// to so the dedupe key `(session, pre_ts, triple_id)` collapses
    /// correctly.  Use a fresh `pre_ts` to simulate a separate Pre firing.
    fn compliance_event(
        post_ts: &str,
        tool: &str,
        triple_id: i64,
        outcome: &str,
        pre_ts: &str,
    ) -> Value {
        json!({
            "ts": post_ts,
            "tool": tool,
            "event": "Compliance",
            "session": "s1",
            "payload": {
                "rules": [{
                    "triple_id": triple_id,
                    "pre_ts": pre_ts,
                    "predicate": "never",
                    "object": "force-push",
                    "outcome": outcome,
                }]
            }
        })
    }

    #[test]
    fn test_empty_stats() {
        let stats = compute(&[]);
        assert_eq!(stats.total_firings, 0);
        assert!(stats.by_rule.is_empty());
        assert!(stats.by_rule_compliance.is_empty());
    }

    #[test]
    fn test_rule_count_aggregation() {
        let entries = vec![
            pre_firing("2026-04-20T10:00:00Z", "Bash", 1, "git", "never", "force-push"),
            pre_firing("2026-04-20T11:00:00Z", "Bash", 1, "git", "never", "force-push"),
            pre_firing("2026-04-20T12:00:00Z", "Write", 2, "alembic", "never", "hand-write"),
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
            pre_firing("2026-04-20T10:00:00Z", "Bash", 1, "a", "b", "c"),
            json!({
                "ts": "2026-04-20T10:01:00Z",
                "tool": "Bash",
                "event": "PostToolUse",
                "rules": [{"triple_id": 1, "subject": "a", "predicate": "b", "object": "c"}]
            }),
            pre_firing("2026-04-20T10:02:00Z", "Write", 2, "a", "b", "c"),
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
            pre_firing("2026-04-22T10:00:00Z", "Bash", 1, "a", "b", "c"),
            pre_firing("2026-04-20T10:00:00Z", "Bash", 1, "a", "b", "c"),
            pre_firing("2026-04-20T11:00:00Z", "Bash", 1, "a", "b", "c"),
        ];
        let stats = compute(&entries);
        assert_eq!(stats.by_day[0].0, "2026-04-20");
        assert_eq!(stats.by_day[0].1, 2);
        assert_eq!(stats.by_day[1].0, "2026-04-22");
        assert_eq!(stats.window_end.as_deref(), Some("2026-04-22T10:00:00Z"));
        assert_eq!(stats.window_start.as_deref(), Some("2026-04-20T11:00:00Z"));
    }

    #[test]
    fn test_rule_tiebreak_alphabetical() {
        let entries = vec![
            pre_firing("2026-04-20T10:00:00Z", "Bash", 1, "zebra", "never", "x"),
            pre_firing("2026-04-20T11:00:00Z", "Bash", 2, "alpha", "never", "x"),
        ];
        let stats = compute(&entries);
        assert_eq!(stats.by_rule[0].0, "alpha never: x");
        assert_eq!(stats.by_rule[1].0, "zebra never: x");
    }

    #[test]
    fn test_compliance_rollup_basic() {
        // 3 distinct Pre firings of rule 1 (different pre_ts), each
        // correlated with one definitive Post: 2 obeyed + 1 ignored.
        let entries = vec![
            pre_firing("2026-04-20T10:00:00Z", "Bash", 1, "git", "never", "force-push"),
            pre_firing("2026-04-20T10:01:00Z", "Bash", 1, "git", "never", "force-push"),
            pre_firing("2026-04-20T10:02:00Z", "Bash", 1, "git", "never", "force-push"),
            compliance_event("2026-04-20T10:00:30Z", "Bash", 1, "obeyed", "2026-04-20T10:00:00Z"),
            compliance_event("2026-04-20T10:01:30Z", "Bash", 1, "obeyed", "2026-04-20T10:01:00Z"),
            compliance_event("2026-04-20T10:02:30Z", "Bash", 1, "ignored", "2026-04-20T10:02:00Z"),
        ];
        let stats = compute(&entries);

        // Total firings excludes Compliance events.
        assert_eq!(stats.total_firings, 3);

        // One rule in the compliance roll-up.
        assert_eq!(stats.by_rule_compliance.len(), 1);
        let rc = &stats.by_rule_compliance[0];
        assert_eq!(rc.triple_id, 1);
        assert_eq!(rc.subject, "git");
        assert_eq!(rc.predicate, "never");
        assert_eq!(rc.fires, 3);
        assert_eq!(rc.obeyed, 2);
        assert_eq!(rc.ignored, 1);
        assert_eq!(rc.unclear, 0);
        // 2 / (2 + 1) = 0.666...
        let ratio = rc.ratio.unwrap();
        assert!((ratio - 2.0 / 3.0).abs() < 1e-9, "ratio = {ratio}");
    }

    #[test]
    fn test_compliance_rollup_no_outcomes_yields_none_ratio() {
        // Pre firings only, no Compliance events: ratio is None.
        let entries = vec![
            pre_firing("2026-04-20T10:00:00Z", "Bash", 7, "alembic", "never", "hand-write"),
        ];
        let stats = compute(&entries);
        assert_eq!(stats.by_rule_compliance.len(), 1);
        assert_eq!(stats.by_rule_compliance[0].fires, 1);
        assert!(stats.by_rule_compliance[0].ratio.is_none());
    }

    #[test]
    fn test_compliance_rollup_unclear_only_yields_unclear_verdict() {
        // Two unclear Compliance events for the same Pre dedupe to one
        // unclear verdict; ratio remains None (no signal).
        let pre = "2026-04-20T10:00:00Z";
        let entries = vec![
            pre_firing(pre, "Bash", 5, "x", "always", "y"),
            compliance_event("2026-04-20T10:00:30Z", "Bash", 5, "unclear", pre),
            compliance_event("2026-04-20T10:01:30Z", "Bash", 5, "unclear", pre),
        ];
        let stats = compute(&entries);
        assert_eq!(stats.by_rule_compliance.len(), 1);
        let rc = &stats.by_rule_compliance[0];
        assert_eq!(
            rc.unclear, 1,
            "two unclear verdicts on the same Pre should dedupe to one"
        );
        assert_eq!(rc.obeyed, 0);
        assert_eq!(rc.ignored, 0);
        assert!(rc.ratio.is_none());
    }

    #[test]
    fn test_compliance_rollup_sort_order() {
        // Two rules: rule 1 has more Pre firings, rule 2 fewer.  Sort
        // primary key is fires desc, so rule 1 sorts first.  Each rule
        // has its own distinct Pre/pre_ts pairing.
        let entries = vec![
            pre_firing("2026-04-20T10:00:00Z", "Bash", 1, "a", "never", "x"),
            pre_firing("2026-04-20T10:01:00Z", "Bash", 1, "a", "never", "x"),
            pre_firing("2026-04-20T10:02:00Z", "Bash", 1, "a", "never", "x"),
            pre_firing("2026-04-20T10:03:00Z", "Bash", 2, "b", "never", "y"),
            compliance_event("2026-04-20T10:00:30Z", "Bash", 1, "obeyed", "2026-04-20T10:00:00Z"),
            compliance_event("2026-04-20T10:03:30Z", "Bash", 2, "ignored", "2026-04-20T10:03:00Z"),
        ];
        let stats = compute(&entries);
        assert_eq!(stats.by_rule_compliance[0].triple_id, 1);
        assert_eq!(stats.by_rule_compliance[1].triple_id, 2);
    }

    // ── #37 dedupe semantics ─────────────────────────────────────────

    #[test]
    fn test_compliance_dedupe_one_pre_many_obeyed_posts_counts_once() {
        // The reproduction case from arai#37: one Pre fires, eight
        // unrelated Posts in the correlation window each emit an `obeyed`
        // Compliance entry against the same Pre.  Pre/before #37: 8 obeyed.
        // Post-fix: dedupes to 1 obeyed, ratio is 100% on n=1 not n=8.
        let pre = "2026-04-20T10:00:00Z";
        let entries = vec![
            pre_firing(pre, "Bash", 1, "git", "never", "force-push"),
            compliance_event("2026-04-20T10:00:05Z", "Bash", 1, "obeyed", pre),
            compliance_event("2026-04-20T10:00:30Z", "Bash", 1, "obeyed", pre),
            compliance_event("2026-04-20T10:01:00Z", "Bash", 1, "obeyed", pre),
            compliance_event("2026-04-20T10:01:30Z", "Bash", 1, "obeyed", pre),
            compliance_event("2026-04-20T10:02:00Z", "Bash", 1, "obeyed", pre),
            compliance_event("2026-04-20T10:02:30Z", "Bash", 1, "obeyed", pre),
            compliance_event("2026-04-20T10:03:00Z", "Bash", 1, "obeyed", pre),
            compliance_event("2026-04-20T10:03:30Z", "Bash", 1, "obeyed", pre),
        ];
        let stats = compute(&entries);
        let rc = &stats.by_rule_compliance[0];
        assert_eq!(rc.fires, 1);
        assert_eq!(
            rc.obeyed, 1,
            "8 obeyed Posts against the same Pre should dedupe to 1 verdict"
        );
        assert_eq!(rc.ignored, 0);
        assert_eq!(rc.ratio, Some(1.0));
    }

    #[test]
    fn test_compliance_dedupe_first_definitive_wins_ignored_then_obeyed() {
        // First definitive-wins: model runs the forbidden command first,
        // then later runs unrelated commands.  Verdict is `ignored`
        // because that's the first non-unclear outcome after the warning.
        let pre = "2026-04-20T10:00:00Z";
        let entries = vec![
            pre_firing(pre, "Bash", 1, "git", "never", "force-push"),
            // The transgression comes first — this is the verdict.
            compliance_event("2026-04-20T10:00:10Z", "Bash", 1, "ignored", pre),
            // Subsequent unrelated commands don't get to "rehabilitate" the
            // original Pre's verdict.
            compliance_event("2026-04-20T10:00:30Z", "Bash", 1, "obeyed", pre),
            compliance_event("2026-04-20T10:01:00Z", "Bash", 1, "obeyed", pre),
        ];
        let stats = compute(&entries);
        let rc = &stats.by_rule_compliance[0];
        assert_eq!(rc.ignored, 1);
        assert_eq!(rc.obeyed, 0);
        assert_eq!(rc.ratio, Some(0.0));
    }

    #[test]
    fn test_compliance_dedupe_first_definitive_wins_obeyed_then_ignored() {
        // Symmetric inverse: model is initially compliant, then runs the
        // forbidden command later.  Verdict is `obeyed` — the first thing
        // after the warning is what's measured.  The later `ignored` is
        // about subsequent state, not the original Pre.  (If the rule
        // would have fired again on that later command, that's a separate
        // Pre and a separate verdict.)
        let pre = "2026-04-20T10:00:00Z";
        let entries = vec![
            pre_firing(pre, "Bash", 1, "git", "never", "force-push"),
            compliance_event("2026-04-20T10:00:10Z", "Bash", 1, "obeyed", pre),
            compliance_event("2026-04-20T10:01:00Z", "Bash", 1, "ignored", pre),
        ];
        let stats = compute(&entries);
        let rc = &stats.by_rule_compliance[0];
        assert_eq!(rc.obeyed, 1);
        assert_eq!(rc.ignored, 0);
        assert_eq!(rc.ratio, Some(1.0));
    }

    #[test]
    fn test_compliance_dedupe_unclear_then_definitive_picks_definitive() {
        // Unclear is not a definitive outcome.  Even if it appears first
        // in time, a later definitive outcome (obeyed or ignored) wins.
        let pre = "2026-04-20T10:00:00Z";
        let entries = vec![
            pre_firing(pre, "Bash", 1, "git", "never", "force-push"),
            compliance_event("2026-04-20T10:00:10Z", "Bash", 1, "unclear", pre),
            compliance_event("2026-04-20T10:00:30Z", "Bash", 1, "ignored", pre),
            compliance_event("2026-04-20T10:01:00Z", "Bash", 1, "obeyed", pre),
        ];
        let stats = compute(&entries);
        let rc = &stats.by_rule_compliance[0];
        assert_eq!(rc.ignored, 1, "first definitive wins, even if preceded by unclear");
        assert_eq!(rc.obeyed, 0);
        assert_eq!(rc.unclear, 0);
    }

    // ── Token economics ──────────────────────────────────────────────

    fn pre_firing_seen(
        ts: &str,
        tool: &str,
        triple_id: i64,
        subj: &str,
        pred: &str,
        obj: &str,
        seen_before: bool,
    ) -> Value {
        json!({
            "ts": ts,
            "tool": tool,
            "event": "PreToolUse",
            "session": "s1",
            "rules": [{
                "triple_id": triple_id,
                "subject": subj,
                "predicate": pred,
                "object": obj,
                "seen_before": seen_before,
            }],
        })
    }

    fn compliance_event_with_severity(
        post_ts: &str,
        tool: &str,
        triple_id: i64,
        outcome: &str,
        pre_ts: &str,
        severity: &str,
    ) -> Value {
        json!({
            "ts": post_ts,
            "tool": tool,
            "event": "Compliance",
            "session": "s1",
            "payload": {
                "rules": [{
                    "triple_id": triple_id,
                    "pre_ts": pre_ts,
                    "predicate": "never",
                    "object": "force-push",
                    "severity": severity,
                    "outcome": outcome,
                }]
            }
        })
    }

    #[test]
    fn test_token_economics_counts_suppressed_repeats() {
        // 4 firings of rule 1: first is fresh, next 3 are seen_before.
        // Suppression count is 3; tokens saved = 3 * 50 = 150.
        let entries = vec![
            pre_firing_seen("2026-04-20T10:00:00Z", "Bash", 1, "git", "never", "force-push", false),
            pre_firing_seen("2026-04-20T10:01:00Z", "Bash", 1, "git", "never", "force-push", true),
            pre_firing_seen("2026-04-20T10:02:00Z", "Bash", 1, "git", "never", "force-push", true),
            pre_firing_seen("2026-04-20T10:03:00Z", "Bash", 1, "git", "never", "force-push", true),
        ];
        let stats = compute(&entries);
        let t = &stats.token_economics;
        assert_eq!(t.suppressed_repeats, 3);
        assert_eq!(t.blocked_obeyed, 0);
        assert_eq!(t.advisory_obeyed, 0);
        assert_eq!(t.estimated_tokens_saved, 3 * 50);
    }

    #[test]
    fn test_token_economics_weights_blocked_vs_advisory() {
        // 2 obeyed-block + 3 obeyed-warn → 2*2000 + 3*500 = 5500.
        let entries = vec![
            pre_firing("2026-04-20T10:00:00Z", "Bash", 1, "git", "never", "force-push"),
            pre_firing("2026-04-20T10:01:00Z", "Bash", 1, "git", "never", "force-push"),
            pre_firing("2026-04-20T10:02:00Z", "Bash", 2, "cargo", "always", "test before commit"),
            pre_firing("2026-04-20T10:03:00Z", "Bash", 2, "cargo", "always", "test before commit"),
            pre_firing("2026-04-20T10:04:00Z", "Bash", 2, "cargo", "always", "test before commit"),
            compliance_event_with_severity("2026-04-20T10:00:30Z", "Bash", 1, "obeyed", "2026-04-20T10:00:00Z", "block"),
            compliance_event_with_severity("2026-04-20T10:01:30Z", "Bash", 1, "obeyed", "2026-04-20T10:01:00Z", "block"),
            compliance_event_with_severity("2026-04-20T10:02:30Z", "Bash", 2, "obeyed", "2026-04-20T10:02:00Z", "warn"),
            compliance_event_with_severity("2026-04-20T10:03:30Z", "Bash", 2, "obeyed", "2026-04-20T10:03:00Z", "warn"),
            compliance_event_with_severity("2026-04-20T10:04:30Z", "Bash", 2, "obeyed", "2026-04-20T10:04:00Z", "warn"),
        ];
        let stats = compute(&entries);
        let t = &stats.token_economics;
        assert_eq!(t.blocked_obeyed, 2);
        assert_eq!(t.advisory_obeyed, 3);
        assert_eq!(t.estimated_tokens_saved, 2 * 2000 + 3 * 500);
    }

    #[test]
    fn test_token_economics_ignored_does_not_save() {
        // An `ignored` verdict means the model ran the action despite the
        // rule.  No tokens saved — we don't claim retroactive credit.
        let entries = vec![
            pre_firing("2026-04-20T10:00:00Z", "Bash", 1, "git", "never", "force-push"),
            compliance_event_with_severity("2026-04-20T10:00:30Z", "Bash", 1, "ignored", "2026-04-20T10:00:00Z", "block"),
        ];
        let stats = compute(&entries);
        let t = &stats.token_economics;
        assert_eq!(t.blocked_obeyed, 0);
        assert_eq!(t.advisory_obeyed, 0);
        assert_eq!(t.estimated_tokens_saved, 0);
    }

    #[test]
    fn test_token_economics_unknown_severity_does_not_save() {
        // Defensive: an `obeyed` verdict with a missing or unrecognised
        // severity field shouldn't blow up or attribute tokens.  Older
        // audit entries (pre-severity tracking) take this path.
        let entries = vec![
            pre_firing("2026-04-20T10:00:00Z", "Bash", 1, "git", "never", "force-push"),
            json!({
                "ts": "2026-04-20T10:00:30Z",
                "tool": "Bash",
                "event": "Compliance",
                "session": "s1",
                "payload": {
                    "rules": [{
                        "triple_id": 1,
                        "pre_ts": "2026-04-20T10:00:00Z",
                        "outcome": "obeyed",
                    }]
                }
            }),
        ];
        let stats = compute(&entries);
        let t = &stats.token_economics;
        assert_eq!(t.blocked_obeyed, 0);
        assert_eq!(t.advisory_obeyed, 0);
        assert_eq!(t.estimated_tokens_saved, 0);
    }

    #[test]
    fn test_token_economics_old_audit_entries_have_no_seen_before() {
        // Audit entries written before this fix have no `seen_before`
        // field.  They should be treated as first-time injections (no
        // suppression credit) — never as `seen_before: true`.
        let entries = vec![
            // Vanilla pre_firing helper omits seen_before.
            pre_firing("2026-04-20T10:00:00Z", "Bash", 1, "git", "never", "force-push"),
            pre_firing("2026-04-20T10:01:00Z", "Bash", 1, "git", "never", "force-push"),
            pre_firing("2026-04-20T10:02:00Z", "Bash", 1, "git", "never", "force-push"),
        ];
        let stats = compute(&entries);
        assert_eq!(stats.token_economics.suppressed_repeats, 0);
    }

    #[test]
    fn test_compliance_dedupe_distinct_pres_count_independently() {
        // Two separate Pre firings of the same rule (different pre_ts)
        // each get their own verdict.  Dedupe is per-Pre, not per-rule —
        // multiple firings of the same rule are still independent
        // observations.
        let entries = vec![
            pre_firing("2026-04-20T10:00:00Z", "Bash", 1, "git", "never", "force-push"),
            pre_firing("2026-04-20T11:00:00Z", "Bash", 1, "git", "never", "force-push"),
            // Pre #1: lots of correlated obeyed, dedupes to 1 obeyed.
            compliance_event("2026-04-20T10:00:30Z", "Bash", 1, "obeyed", "2026-04-20T10:00:00Z"),
            compliance_event("2026-04-20T10:01:00Z", "Bash", 1, "obeyed", "2026-04-20T10:00:00Z"),
            compliance_event("2026-04-20T10:01:30Z", "Bash", 1, "obeyed", "2026-04-20T10:00:00Z"),
            // Pre #2: ignored.
            compliance_event("2026-04-20T11:00:30Z", "Bash", 1, "ignored", "2026-04-20T11:00:00Z"),
        ];
        let stats = compute(&entries);
        let rc = &stats.by_rule_compliance[0];
        assert_eq!(rc.fires, 2);
        assert_eq!(rc.obeyed, 1);
        assert_eq!(rc.ignored, 1);
        // 1 / (1 + 1) = 0.5
        assert_eq!(rc.ratio, Some(0.5));
    }
}
