//! Host lifecycle contracts, exercised through stdin without model/tool execution.
use arai::{config::Config, lifecycle, platforms::Platform};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new(rules: Option<&str>) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let f = Self(
            std::env::temp_dir().join(format!("arai_lifecycle_{}_{stamp}", std::process::id())),
        );
        fs::create_dir_all(f.0.join("project/.git")).unwrap();
        fs::create_dir_all(f.0.join("home")).unwrap();
        if let Some(rules) = rules {
            fs::write(f.0.join("project/AGENTS.md"), rules).unwrap();
            let output = f.command().arg("scan").output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        f
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_arai"));
        command
            .current_dir(self.0.join("project"))
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("ARAI_BASE_DIR", self.0.join("state"))
            .env("ARAI_TELEMETRY", "off")
            .env_remove("ARAI_DISABLED")
            .env_remove("ARAI_DENY_MODE")
            .env_remove("GROK_HOOK_EVENT")
            .env_remove("GROK_SESSION_ID")
            .env_remove("CLAUDE_PROJECT_DIR")
            .env_remove("CLAUDE_PLUGIN_ROOT");
        command
    }
    fn hook(&self, platform: &str, event: &str, payload: Value, env: &[(&str, &str)]) -> Output {
        let mut command = self.command();
        command.args([
            "guardrails",
            "--match-stdin",
            "--platform",
            platform,
            "--hook-event",
            event,
        ]);
        for (key, value) in env {
            command.env(key, value);
        }
        let mut child = command
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
    fn cfg(&self) -> Config {
        let mut cfg = Config::load_from(&self.0.join("project")).unwrap();
        cfg.arai_base_dir = self.0.join("state");
        cfg
    }
    fn audit(&self) -> Vec<Value> {
        arai::audit::query(
            &self.0.join("state"),
            &self.cfg().project_slug(),
            None,
            None,
            None,
            100,
        )
        .unwrap()
    }
    fn payload(event: &str) -> Value {
        json!({"hook_event_name": event, "source":"startup", "session_id":"lifecycle-session",
            "tool_name":"Bash", "tool_input":{"command":"cargo clean"}})
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn response(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn startup_surfaces_missing_policy_without_creating_a_store_or_claiming_tool_activation() {
    let f = Fixture::new(None);
    for platform in ["claude", "codex", "grok"] {
        let out = response(&f.hook(
            platform,
            "SessionStart",
            Fixture::payload("SessionStart"),
            &[],
        ));
        assert!(out["systemMessage"]
            .as_str()
            .unwrap()
            .contains("policy store missing"));
        assert!(out["systemMessage"]
            .as_str()
            .unwrap()
            .contains("activation is not proven"));
        assert!(out["hookSpecificOutput"]["permissionDecision"].is_null());
        if platform == "grok" {
            assert!(out["hookSpecificOutput"].is_null());
        } else {
            assert!(out["hookSpecificOutput"]["additionalContext"].is_string());
        }
    }
    assert!(!f.cfg().db_path().exists());
    assert!(!f.0.join("state/sessions").exists());
    assert!(f.audit().is_empty());
    let status = f.command().arg("status").output().unwrap();
    let status = String::from_utf8(status.stdout).unwrap();
    assert!(status.contains("owned handlers"));
    assert!(status.contains("not initialized"));
    assert!(status.contains("adapter invocation"));
    assert!(
        !f.cfg().db_path().exists(),
        "status must not initialize policy either"
    );
}

#[test]
fn startup_and_subagent_report_modes_without_marking_rule_delivery() {
    let f = Fixture::new(Some("- Never run cargo clean\n"));
    for platform in ["claude", "codex", "grok"] {
        let startup = response(&f.hook(
            platform,
            "SessionStart",
            Fixture::payload("SessionStart"),
            &[("ARAI_DISABLED", "1")],
        ));
        assert!(startup["systemMessage"]
            .as_str()
            .unwrap()
            .contains("disabled"));
        let subagent = response(&f.hook(
            platform,
            "SubagentStart",
            Fixture::payload("SubagentStart"),
            &[("ARAI_DENY_MODE", "off")],
        ));
        assert!(subagent["systemMessage"].is_null());
        if platform != "grok" {
            assert!(subagent["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .contains("advisory"));
        } else {
            assert_eq!(subagent, json!({}));
        }
    }
    assert!(!f.0.join("state/sessions").exists());
    let summary = lifecycle::invocation_summary(&f.cfg(), Platform::Codex);
    assert!(summary.contains("disabled") && summary.contains("SubagentStart"));
}

#[test]
fn passive_events_with_tool_fields_never_enter_the_rule_matcher() {
    let f = Fixture::new(Some("- Never run cargo clean\n"));
    for event in [
        "PostCompact",
        "PreCompact",
        "PermissionRequest",
        "Interrupt",
        "Stop",
        "SessionEnd",
        "PostToolUseFailure",
        "ConfigChange",
        "TaskCompleted",
        "PreModelSwitch",
    ] {
        let output = f.hook("claude", event, Fixture::payload(event), &[]);
        assert!(output.status.success());
        assert!(output.stdout.is_empty(), "{event}: {:?}", output.stdout);
    }
    assert!(f.audit().is_empty());
}

#[test]
fn advisory_context_does_not_auto_approve_and_grok_records_deferred_delivery() {
    let f = Fixture::new(Some("- Never run cargo clean\n"));
    for platform in ["claude", "codex", "grok"] {
        let mut payload = Fixture::payload("PreToolUse");
        // Native Grok can include equivalent aliases with different spelling.
        payload["hookEventName"] = json!("pre_tool_use");
        let out = response(&f.hook(
            platform,
            "PreToolUse",
            payload,
            &[("ARAI_DENY_MODE", "off")],
        ));
        assert!(
            out["hookSpecificOutput"]["additionalContext"].is_string(),
            "{platform}: {out}"
        );
        assert!(out["hookSpecificOutput"]["permissionDecision"].is_null());
        if platform == "grok" {
            assert!(out["additionalContext"].is_null());
        }
    }
    assert!(f.audit().iter().any(|entry| entry["decision"] == "defer"));
    let g = Fixture::new(Some("- Never run cargo clean\n"));
    g.hook(
        "grok",
        "PreToolUse",
        Fixture::payload("PreToolUse"),
        &[("ARAI_DENY_MODE", "off")],
    );
    assert!(
        !g.0.join("state/sessions").exists(),
        "deferred advice cannot mark rules seen"
    );
    g.hook("grok", "PostToolUse", Fixture::payload("PostToolUse"), &[]);
    assert!(!g.audit().iter().any(|entry| entry["event"] == "Compliance"));
    let imported = Fixture::new(Some("- Never run cargo clean\n"));
    imported.hook(
        "claude",
        "PreToolUse",
        Fixture::payload("PreToolUse"),
        &[
            ("ARAI_DENY_MODE", "off"),
            ("GROK_HOOK_EVENT", "pre_tool_use"),
        ],
    );
    assert!(
        !imported.0.join("state/sessions").exists(),
        "Grok compatibility imports also defer advice"
    );
}

#[test]
fn batch_summaries_do_not_duplicate_native_post_tool_accounting() {
    let f = Fixture::new(Some("- Always run cargo test before commit\n"));
    let post = Fixture::payload("PostToolUse");
    f.hook("claude", "PostToolUse", post, &[]);
    let session_path = f.0.join("state/sessions/lifecycle-session.json");
    let before = fs::read(&session_path).unwrap();
    for payload in [
        json!({"hook_event_name":"PostToolBatch", "session_id":"lifecycle-session",
            "tool_calls":[{"tool_name":"Bash","tool_input":{"command":"cargo clean"},"tool_response":{"stdout":"ok"}}]}),
        json!({"hook_event_name":"PostToolBatch", "session_id":"lifecycle-session",
            "tool_calls":[{"tool_name":"Bash","tool_input":{"command":"cargo clean"}}],
            "tool_results":[{"output":{"content":"ok"}}]}),
    ] {
        let out = f.hook("claude", "PostToolBatch", payload, &[]);
        assert!(out.status.success() && out.stdout.is_empty());
    }
    assert_eq!(before, fs::read(&session_path).unwrap());
    let batches: Vec<_> = f
        .audit()
        .into_iter()
        .filter(|e| e["event"] == "PostToolBatch")
        .collect();
    assert_eq!(batches.len(), 2);
}

#[test]
fn permission_denial_records_native_reason_without_requesting_retry() {
    let f = Fixture::new(Some("- Always run cargo test before commit\n"));
    let mut payload = Fixture::payload("PermissionDenied");
    payload["tool_input"] = json!({"command":"cargo test"});
    payload["reason"] = json!("native host denied");
    payload["denial_reason"] = json!("legacy reason");
    let out = f.hook("claude", "PermissionDenied", payload, &[]);
    assert!(out.status.success() && out.stdout.is_empty());
    let event = f
        .audit()
        .into_iter()
        .find(|e| e["event"] == "PermissionDenied")
        .unwrap();
    assert!(event.to_string().contains("native host denied"));
    assert!(!event.to_string().contains("legacy reason"));
    assert!(!event.to_string().contains("\"retry\":true"));
}
