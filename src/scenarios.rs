//! Scenario-based regression test harness for Ārai rules.
//!
//! Users write a JSON file describing synthetic hook payloads with expected
//! match behaviour.  `arai test <file>` replays each scenario through the
//! same `match_hook` pipeline the live hook handler uses, and reports
//! pass/fail.  No audit log is written and no telemetry is emitted — the
//! harness is read-only against the rule graph.
//!
//! The scenario format is deliberately minimal.  Each scenario specifies
//! a tool call (hook payload) and one or more expectations about which
//! rules should fire.  Expectations are matched on rule subject substrings
//! because full SPO triples are rarely stable across re-ingest.
//!
//! Schema
//! ──────
//! ```json
//! {
//!   "scenarios": [
//!     {
//!       "name": "force-push triggers git guardrail",
//!       "hook": {
//!         "hook_event_name": "PreToolUse",
//!         "tool_name": "Bash",
//!         "tool_input": { "command": "git push --force origin master" }
//!       },
//!       "expect": {
//!         "matches_subject": ["git"],
//!         "does_not_match_subject": ["alembic"],
//!         "min_matches": 1,
//!         "max_matches": 10
//!       }
//!     }
//!   ]
//! }
//! ```
//!
//! All expectation fields are optional.  An empty `expect` block only
//! verifies the hook parses and the match pipeline doesn't error.

use crate::audit;
use crate::config::Config;
use crate::hooks;
use crate::store::{Guardrail, Store};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct ScenarioFile {
    #[serde(default)]
    pub scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub hook: Value,
    #[serde(default)]
    pub expect: Expect,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Expect {
    /// Each listed substring must appear in at least one matched rule subject.
    pub matches_subject: Vec<String>,
    /// No matched rule subject may contain any of these substrings.
    pub does_not_match_subject: Vec<String>,
    /// Minimum number of rules that must match.  `None` = no lower bound.
    pub min_matches: Option<usize>,
    /// Maximum number of rules that may match.  `None` = no upper bound.
    pub max_matches: Option<usize>,
}

/// Outcome of running one scenario.
#[derive(Debug)]
pub struct ScenarioResult {
    pub name: String,
    pub passed: bool,
    pub failures: Vec<String>,
    pub matched_subjects: Vec<String>,
}

/// Run every scenario in the file against the current project's guardrail
/// set.  Returns one result per scenario.
pub fn run_file(path: &Path, cfg: &Config, db: &Store) -> Result<Vec<ScenarioResult>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read scenario file {}: {e}", path.display()))?;
    let file: ScenarioFile = serde_json::from_str(&content)
        .map_err(|e| format!("Invalid scenario JSON: {e}"))?;

    let mut results = Vec::with_capacity(file.scenarios.len());
    for scenario in file.scenarios {
        results.push(run_one(&scenario, cfg, db));
    }
    Ok(results)
}

fn run_one(scenario: &Scenario, cfg: &Config, db: &Store) -> ScenarioResult {
    let mut failures = Vec::new();
    let matched = match hooks::match_hook(&scenario.hook, cfg, db) {
        Ok(m) => m,
        Err(e) => {
            return ScenarioResult {
                name: scenario.name.clone(),
                passed: false,
                failures: vec![format!("match_hook error: {e}")],
                matched_subjects: Vec::new(),
            };
        }
    };

    let matched_subjects: Vec<String> = matched
        .matched
        .iter()
        .map(|(g, _)| g.subject.clone())
        .collect();

    check_expectations(&scenario.expect, &matched.matched, &mut failures);

    ScenarioResult {
        name: scenario.name.clone(),
        passed: failures.is_empty(),
        failures,
        matched_subjects,
    }
}

fn check_expectations(
    expect: &Expect,
    matched: &[(Guardrail, u8)],
    failures: &mut Vec<String>,
) {
    let count = matched.len();
    if let Some(min) = expect.min_matches {
        if count < min {
            failures.push(format!(
                "expected at least {min} matches, got {count}"
            ));
        }
    }
    if let Some(max) = expect.max_matches {
        if count > max {
            failures.push(format!(
                "expected at most {max} matches, got {count}"
            ));
        }
    }
    for needle in &expect.matches_subject {
        let hit = matched
            .iter()
            .any(|(g, _)| g.subject.contains(needle));
        if !hit {
            failures.push(format!(
                "no matched rule subject contained {needle:?}"
            ));
        }
    }
    for needle in &expect.does_not_match_subject {
        let hit = matched
            .iter()
            .find(|(g, _)| g.subject.contains(needle));
        if let Some((g, _)) = hit {
            failures.push(format!(
                "rule {:?} {} {:?} matched but was excluded by does_not_match_subject={needle:?}",
                g.subject, g.predicate, g.object
            ));
        }
    }
}

/// CLI entry point for `arai test <file>`.
pub fn run(path: &Path, json: bool) -> Result<(), String> {
    let cfg = Config::load()?;
    let db_path = cfg.db_path();
    if !db_path.exists() {
        return Err(
            "No guardrail database found.  Run `arai init` first before running scenario tests."
                .to_string(),
        );
    }
    let db = Store::open(&db_path)?;
    let results = run_file(path, &cfg, &db)?;

    if json {
        let out: Vec<Value> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "name": r.name,
                    "passed": r.passed,
                    "failures": r.failures,
                    "matched_subjects": r.matched_subjects,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
    } else {
        print_results(&results);
    }

    let failed = results.iter().filter(|r| !r.passed).count();
    if failed > 0 {
        return Err(format!("{failed} scenario(s) failed"));
    }
    Ok(())
}

fn print_results(results: &[ScenarioResult]) {
    for r in results {
        let status = if r.passed { "PASS" } else { "FAIL" };
        println!("  {status}  {}", r.name);
        for f in &r.failures {
            println!("        - {f}");
        }
    }
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.len() - passed;
    println!("\n  {}/{} passed", passed, results.len());
    if failed > 0 {
        println!("  {failed} failed");
    }
}

// ── Recording scenarios from the live audit log ───────────────────────────

/// Build a minimal hook payload from an audit entry's preview.
///
/// The audit log stores a truncated `prompt_preview`, not the full tool
/// input — we can only approximate.  For Bash we treat the preview as the
/// command; for Edit/Write/MultiEdit we parse the `"<tool> <path>"` form
/// back into `{file_path}`; for other tools we pass the preview through
/// as a `"preview"` field so the scenario at least carries identifying
/// text.  Scenarios produced this way are seeds — tune them by hand.
fn reconstruct_tool_input(tool: &str, preview: &str) -> Value {
    match tool {
        "Bash" => json!({ "command": preview }),
        "Edit" | "Write" | "MultiEdit" => {
            let path = preview
                .strip_prefix(&format!("{tool} "))
                .unwrap_or(preview);
            json!({ "file_path": path })
        }
        _ => json!({ "preview": preview }),
    }
}

/// Build scenario fixtures from recent audit-log entries.
///
/// Deduplicates by (tool, prompt_preview) so many identical firings
/// collapse to one fixture.  Each scenario's expectation is a best-guess:
/// every subject that actually fired must appear in the match set, with
/// `min_matches = 1`.
pub fn record_from_audit(
    cfg: &Config,
    since_epoch_secs: Option<u64>,
    tool_filter: Option<&str>,
    limit: usize,
) -> Result<Value, String> {
    let entries = audit::query(
        &cfg.arai_base_dir,
        &cfg.project_slug(),
        since_epoch_secs,
        tool_filter,
        None,
        limit,
    )?;

    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut scenarios = Vec::new();

    for entry in entries {
        let tool = entry.get("tool").and_then(|v| v.as_str()).unwrap_or("");
        let event = entry
            .get("event")
            .and_then(|v| v.as_str())
            .unwrap_or("PreToolUse");
        let preview = entry
            .get("prompt_preview")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if tool.is_empty() || preview.is_empty() {
            continue;
        }

        let key = (tool.to_string(), preview.to_string());
        if !seen.insert(key) {
            continue;
        }

        let subjects: Vec<String> = entry
            .get("rules")
            .and_then(|v| v.as_array())
            .map(|rs| {
                let mut out: Vec<String> = rs
                    .iter()
                    .filter_map(|r| r.get("subject").and_then(|v| v.as_str()).map(String::from))
                    .collect();
                out.sort();
                out.dedup();
                out
            })
            .unwrap_or_default();

        let mut name = format!("{tool}: {preview}");
        if name.len() > 80 {
            name.truncate(79);
            name.push('…');
        }

        let hook = json!({
            "hook_event_name": event,
            "tool_name": tool,
            "tool_input": reconstruct_tool_input(tool, preview),
        });

        let expect = if subjects.is_empty() {
            json!({ "max_matches": 0 })
        } else {
            json!({
                "matches_subject": subjects,
                "min_matches": 1,
            })
        };

        scenarios.push(json!({
            "name": name,
            "hook": hook,
            "expect": expect,
        }));
    }

    Ok(json!({ "scenarios": scenarios }))
}

/// CLI entry point for `arai record`.
pub fn record(
    cfg: &Config,
    since_epoch_secs: Option<u64>,
    tool: Option<String>,
    limit: usize,
) -> Result<(), String> {
    let out = record_from_audit(cfg, since_epoch_secs, tool.as_deref(), limit)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gr(triple_id: i64, subject: &str, predicate: &str, object: &str) -> (Guardrail, u8) {
        (
            Guardrail {
                triple_id,
                subject: subject.to_string(),
                predicate: predicate.to_string(),
                object: object.to_string(),
                confidence: 0.9,
                source_file: "test".to_string(),
                file_path: "test.md".to_string(),
                layer: None,
                line_start: None,
                expires_at: None,
                intent: None,
            },
            100,
        )
    }

    #[test]
    fn test_min_matches_failure() {
        let mut failures = Vec::new();
        let expect = Expect {
            min_matches: Some(2),
            ..Default::default()
        };
        check_expectations(&expect, &[gr(1, "git", "never", "force-push")], &mut failures);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("at least 2"));
    }

    #[test]
    fn test_max_matches_failure() {
        let mut failures = Vec::new();
        let expect = Expect {
            max_matches: Some(1),
            ..Default::default()
        };
        check_expectations(
            &expect,
            &[
                gr(1, "git", "never", "a"),
                gr(2, "git", "never", "b"),
            ],
            &mut failures,
        );
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("at most 1"));
    }

    #[test]
    fn test_matches_subject_substring() {
        let mut failures = Vec::new();
        let expect = Expect {
            matches_subject: vec!["git".to_string(), "alembic".to_string()],
            ..Default::default()
        };
        check_expectations(
            &expect,
            &[
                gr(1, "git push", "never", "force"),
                // alembic missing → should fail
            ],
            &mut failures,
        );
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("alembic"));
    }

    #[test]
    fn test_does_not_match_subject() {
        let mut failures = Vec::new();
        let expect = Expect {
            does_not_match_subject: vec!["alembic".to_string()],
            ..Default::default()
        };
        check_expectations(
            &expect,
            &[gr(1, "alembic", "never", "hand-write")],
            &mut failures,
        );
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("alembic"));
    }

    #[test]
    fn test_all_pass() {
        let mut failures = Vec::new();
        let expect = Expect {
            matches_subject: vec!["git".to_string()],
            does_not_match_subject: vec!["alembic".to_string()],
            min_matches: Some(1),
            max_matches: Some(3),
        };
        check_expectations(
            &expect,
            &[gr(1, "git", "never", "force-push")],
            &mut failures,
        );
        assert!(failures.is_empty());
    }

    #[test]
    fn test_parse_scenario_file() {
        let raw = r#"
        {
          "scenarios": [
            {
              "name": "t1",
              "hook": {
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": "git push --force"}
              },
              "expect": {
                "matches_subject": ["git"],
                "min_matches": 1
              }
            }
          ]
        }
        "#;
        let file: ScenarioFile = serde_json::from_str(raw).unwrap();
        assert_eq!(file.scenarios.len(), 1);
        assert_eq!(file.scenarios[0].name, "t1");
        assert_eq!(file.scenarios[0].expect.matches_subject, vec!["git"]);
    }

    #[test]
    fn test_reconstruct_bash_tool_input() {
        let input = reconstruct_tool_input("Bash", "git push --force origin main");
        assert_eq!(input["command"], "git push --force origin main");
    }

    #[test]
    fn test_reconstruct_edit_tool_input() {
        let input = reconstruct_tool_input("Edit", "Edit /tmp/foo.rs");
        assert_eq!(input["file_path"], "/tmp/foo.rs");
    }

    #[test]
    fn test_reconstruct_write_tool_input() {
        let input = reconstruct_tool_input("Write", "Write /tmp/bar.md");
        assert_eq!(input["file_path"], "/tmp/bar.md");
    }

    #[test]
    fn test_reconstruct_unknown_tool_passes_through_preview() {
        let input = reconstruct_tool_input("CustomTool", "some opaque payload");
        assert_eq!(input["preview"], "some opaque payload");
    }

    #[test]
    fn test_empty_expect_parses() {
        let raw = r#"{"scenarios": [{"name": "t", "hook": {}}]}"#;
        let file: ScenarioFile = serde_json::from_str(raw).unwrap();
        assert_eq!(file.scenarios[0].expect.matches_subject.len(), 0);
    }
}
