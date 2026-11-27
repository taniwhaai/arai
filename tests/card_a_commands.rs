//! Parent composition coverage through isolated real CLI processes.
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    home: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("arai_parent_{}_{nonce}", std::process::id()));
        let project = root.join("project");
        let home = root.join("home");
        let state = root.join("state");
        for path in [&project, &home, &state, &project.join(".git")] {
            fs::create_dir_all(path).unwrap();
        }
        Self {
            root,
            project,
            home,
            state,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_arai"));
        cmd.args(args).current_dir(&self.project);
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if ["ARAI_", "CLAUDE_", "CODEX_", "GROK_", "CURSOR_", "COPILOT_"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
                || ["ANTHROPIC_API_KEY", "OPENAI_API_KEY"].contains(&name.as_ref())
            {
                cmd.env_remove(key);
            }
        }
        cmd.env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("ARAI_BASE_DIR", &self.state)
            .env("DO_NOT_TRACK", "1");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        let out = self.command(args).output().unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {} {}",
            text(&out),
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    fn alembic(&self) {
        fs::write(
            self.project.join("CLAUDE.md"),
            include_bytes!("fixtures/card_a_alembic/CLAUDE.md"),
        )
        .unwrap();
    }

    fn replay(&self, path: &str) -> Output {
        let payload = json!({"hook_event_name":"PreToolUse", "tool_name":"Write",
            "tool_input":{"file_path":path,"content":""},"session_id":"","cwd":self.project});
        let mut child = self
            .command(&["guardrails", "--match-stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn summary(out: &Output) -> Vec<String> {
    text(out)
        .lines()
        .map(str::trim)
        .filter(|line| {
            [
                "Stored sources:",
                "Stored actionable rules:",
                "Block eligible:",
                "Advise eligible:",
                "Documentary/inert:",
                "Inactive:",
                "Runtime mode:",
            ]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        })
        .map(str::to_owned)
        .collect()
}

#[test]
fn final_summary_is_shared_and_preserves_provenance_and_static_status() {
    let f = Fixture::new();
    f.alembic();
    let scan = f.run(&["scan"]);
    let init = f.run(&["init"]);
    let status = f.run(&["status"]);
    assert_eq!(summary(&scan), summary(&init));
    assert_eq!(summary(&scan), summary(&status));
    assert_eq!(summary(&scan).len(), 7);
    assert!(text(&scan).contains("Extracted: 1 rule(s) from 1 file(s)"));
    assert!(text(&init).contains("1 rules extracted"));
    for out in [&scan, &init, &status] {
        let s = text(out);
        assert!(s.contains("Block eligible: 1") && s.contains("CLAUDE.md:3"));
        assert!(s.contains("Conditional directory:") && s.contains("it is not a match"));
    }
    let before = fs::read(f.project.join(".codex/hooks.json")).unwrap();
    let status = f.run(&["status"]);
    assert_eq!(
        before,
        fs::read(f.project.join(".codex/hooks.json")).unwrap()
    );
    assert!(text(&status).contains("config present") && text(&status).contains("Last firing:"));
    assert!(text(&status).contains("Code graph:") && !text(&status).contains("registered ("));
}

#[test]
fn empty_and_bare_force_push_remain_successful_with_zero_effective_blocks() {
    for content in ["", "- Never force-push to main\n"] {
        let f = Fixture::new();
        if !content.is_empty() {
            fs::write(f.project.join("CLAUDE.md"), content).unwrap();
        }
        for command in ["init", "scan", "status"] {
            let out = f.run(&[command]);
            let s = text(&out);
            assert!(s.contains("Block eligible: 0") && s.contains("Advise eligible: 0"));
            assert!(s.contains(if content.is_empty() {
                "Documentary/inert: 0"
            } else {
                "Documentary/inert: 1"
            }));
        }
    }
}

#[test]
fn isolated_runtime_modes_have_equal_command_labels() {
    // Mode overrides exist only in these isolated child processes to verify display parity.
    for (key, value, label) in [
        ("ARAI_DENY_MODE", "off", "advisory only (deny disabled)"),
        ("ARAI_DISABLED", "1", "hard bypass (no enforcement)"),
    ] {
        let f = Fixture::new();
        f.alembic();
        let mut expected = None;
        for command in ["init", "scan", "status"] {
            let out = f.command(&[command]).env(key, value).output().unwrap();
            assert!(out.status.success());
            assert!(text(&out).contains(label));
            let actual = summary(&out);
            if let Some(previous) = &expected {
                assert_eq!(previous, &actual);
            }
            expected = Some(actual);
        }
    }
}

#[test]
fn real_init_renders_optional_skip_and_mandatory_failure() {
    let optional = Fixture::new();
    fs::create_dir_all(optional.project.join(".grok/hooks")).unwrap();
    fs::write(optional.project.join(".grok/hooks/arai.json"), "broken").unwrap();
    let out = optional.run(&["init"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Grok registration skipped") && err.contains("arai.json"));
    assert!(text(&out).contains("Copilot registered"));
    assert!(!text(&out).contains("Grok registered"));
    assert!(text(&out).contains("writes do not establish trust or hook execution"));

    let mandatory = Fixture::new();
    fs::create_dir_all(mandatory.project.join(".codex")).unwrap();
    fs::write(mandatory.project.join(".codex/hooks.json"), "broken").unwrap();
    let out = mandatory.command(&["init"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Codex registration failed") && err.contains("arai:"));
    assert!(text(&out).contains("Claude registered"));
    assert!(mandatory.project.join(".claude/settings.json").is_file());
    assert!(!mandatory.project.join(".cursor/hooks.json").exists());
    assert!(!mandatory.project.join(".github/hooks/arai.json").exists());
}

#[test]
fn exact_alembic_native_denial_allowed_control_and_current_scenario() {
    let f = Fixture::new();
    f.alembic();
    f.run(&["scan"]);
    let deny = f.replay("alembic/versions/001_example.py");
    assert_eq!(deny.status.code(), Some(0));
    let response: Value = serde_json::from_slice(&deny.stdout).unwrap();
    assert_eq!(response["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(response["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap()
        .contains("CLAUDE.md:3"));
    let allow = f.replay("app/example.py");
    assert_eq!(allow.status.code(), Some(0));
    assert!(allow.stdout.is_empty());
    let scenario =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scenarios/alembic-migration.json");
    let out = f.run(&["test", scenario.to_str().unwrap(), "--json"]);
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    let results = result.as_array().unwrap();
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|row| row["passed"] == true));
    assert_eq!(results[0]["decision"], "deny");
    assert_eq!(results[1]["decision"], "allow");
    assert_eq!(results[2]["decision"], "allow");
}

#[test]
fn frozen_three_source_baseline_has_shared_documentary_counts() {
    let f = Fixture::new();
    let archived = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/card_a_baseline");
    for relative in [
        "CLAUDE.md",
        "AGENTS.md",
        ".taniwha/kupu/orchestrator/handoff/01KSGB5549QPQDQMSH6B8NS077/outputs/files/CLAUDE.md",
    ] {
        let destination = f.project.join(relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(archived.join(relative), destination).unwrap();
    }
    let scan = f.run(&["scan"]);
    let status = f.run(&["status"]);
    assert_eq!(summary(&scan), summary(&status));
    let s = text(&scan);
    for expected in [
        "Stored sources: 3",
        "Stored actionable rules: 22",
        "Block eligible: 0",
        "Advise eligible: 0",
        "Documentary/inert: 22",
        "Inactive: 0",
    ] {
        assert!(s.contains(expected), "missing {expected}: {s}");
    }
}

#[test]
fn failed_requested_import_does_not_print_a_premature_summary() {
    let f = Fixture::new();
    f.alembic();
    let out = f
        .command(&["scan", "--enrich-file", "missing-enrichment.json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out).contains("Importing enrichment"));
    assert!(!text(&out).contains("Effective policy summary"));
    assert!(String::from_utf8_lossy(&out.stderr).contains("arai:"));
}
