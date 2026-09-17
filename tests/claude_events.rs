//! Claude Code events beyond the three tool-call hooks, driven with the
//! documented example payloads (code.claude.com/docs/en/hooks) so field-name
//! drift is caught by the fixture, not by a user's empty audit log.
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Project(PathBuf);

impl Project {
    fn new(label: &str, rules: &str) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "arai_claude_events_{label}_{}_{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("project/.git")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        fs::write(root.join("project/CLAUDE.md"), rules).unwrap();
        let project = Self(root);
        let out = project.command().arg("scan").output().unwrap();
        assert!(
            out.status.success(),
            "scan: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        project
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_arai"));
        cmd.current_dir(self.0.join("project"))
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("ARAI_BASE_DIR", self.0.join("state"))
            .env("ARAI_TELEMETRY", "off")
            .env("ARAI_DENY_MODE", "on")
            .env_remove("ARAI_DISABLED")
            .env_remove("GROK_HOOK_EVENT")
            .env_remove("GROK_SESSION_ID")
            .env_remove("CURSOR_VERSION")
            .env_remove("CLAUDE_PROJECT_DIR")
            .env_remove("CLAUDE_PLUGIN_ROOT");
        cmd
    }

    fn hook(&self, payload: Value) -> Option<Value> {
        let mut child = self
            .command()
            .args(["guardrails", "--match-stdin"])
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
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "hook: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        if out.stdout.is_empty() {
            None
        } else {
            Some(serde_json::from_slice(&out.stdout).expect("hook JSON"))
        }
    }

    fn audit(&self, event: &str) -> Vec<Value> {
        let out = self
            .command()
            .args([
                "audit",
                &format!("--event={event}"),
                "--json",
                "--limit=100",
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn outcomes(entries: &[Value]) -> Vec<(String, String)> {
    entries
        .iter()
        .flat_map(|entry| {
            entry["payload"]["rules"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .map(|rule| {
            (
                rule["outcome"].as_str().unwrap_or("").to_string(),
                rule["predicate"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

const SESSION: &str = "claude-events-session-1";

#[test]
fn post_tool_batch_sniffs_each_entrys_own_tool_response() {
    let p = Project::new("batch", "- Never run alembic upgrade by hand\n");
    let pre = p
        .hook(json!({"hook_event_name": "PreToolUse", "tool_name": "Bash",
            "tool_input": {"command": "alembic upgrade head"}, "session_id": SESSION,
            "tool_use_id": "toolu_01"}))
        .expect("pre decision");
    assert_eq!(pre["hookSpecificOutput"]["permissionDecision"], "deny");

    // Documented shape: tool_response lives inside each tool_calls entry and
    // is the serialised tool_result content (string or content-block array).
    // Neither tool_input mentions the forbidden phrase, so an `ignored`
    // verdict can only come from sniffing the responses.
    let out = p.hook(json!({
        "session_id": SESSION,
        "hook_event_name": "PostToolBatch",
        "tool_calls": [
            {"tool_name": "Bash", "tool_input": {"command": "make migrate"},
             "tool_use_id": "toolu_02", "tool_response": "from alembic import op\n"},
            {"tool_name": "Bash", "tool_input": {"command": "make lint"},
             "tool_use_id": "toolu_03",
             "tool_response": [{"type": "text", "text": "from alembic import op"}]}
        ]
    }));
    assert!(
        out.is_none(),
        "PostToolBatch has no decision surface: {out:?}"
    );

    let batch = p.audit("PostToolBatch");
    assert_eq!(batch.len(), 1, "{batch:?}");
    assert_eq!(batch[0]["payload"]["tool_count"], 2);
    let verdicts = outcomes(&p.audit("Compliance"));
    assert!(
        verdicts
            .iter()
            .any(|(o, pred)| o == "ignored" && pred == "never"),
        "expected an ignored verdict seeded from tool_response; got {verdicts:?}"
    );
}

#[test]
fn permission_denied_records_documented_reason_and_never_retries_a_block() {
    let p = Project::new("denied_block", "- Never run alembic upgrade by hand\n");
    // Documented example payload, with the tool switched to one our rule covers.
    let out = p.hook(json!({
        "session_id": SESSION,
        "permission_mode": "auto",
        "hook_event_name": "PermissionDenied",
        "tool_name": "Bash",
        "tool_input": {"command": "alembic upgrade head", "description": "Migrate"},
        "tool_use_id": "toolu_01ABC123",
        "reason": "[Irreversible Local Destruction]"
    }));
    assert!(
        out.is_none(),
        "Block-severity agreement must not retry: {out:?}"
    );
    let denied = p.audit("PermissionDenied");
    assert_eq!(denied.len(), 1, "{denied:?}");
    let payload = &denied[0]["payload"];
    assert_eq!(payload["denial_reason"], "[Irreversible Local Destruction]");
    assert_eq!(payload["arai_matched"], true);
    assert_eq!(payload["arai_severity"], "block");
    assert_eq!(payload["retry"], false);
}

#[test]
fn permission_denied_retries_when_arai_only_warns() {
    let p = Project::new(
        "denied_warn",
        // Object must share the `alembic upgrade` phrase with the command:
        // the matcher's phrase gate drops affirmative rules that name a
        // different subcommand (see `guardrails::relevance_score`).
        "- Always run alembic upgrade through the Makefile\n",
    );
    let out = p
        .hook(json!({
            "session_id": SESSION,
            "permission_mode": "auto",
            "hook_event_name": "PermissionDenied",
            "tool_name": "Bash",
            "tool_input": {"command": "alembic upgrade head"},
            "tool_use_id": "toolu_01ABC124",
            "reason": "Auto mode could not evaluate this action and is blocking it for safety"
        }))
        .expect("retry response");
    assert_eq!(
        out["hookSpecificOutput"]["hookEventName"],
        "PermissionDenied"
    );
    assert_eq!(out["hookSpecificOutput"]["retry"], true);
    let payload = &p.audit("PermissionDenied")[0]["payload"];
    assert_eq!(payload["arai_severity"], "warn");
    assert_eq!(payload["retry"], true);
    assert!(payload["denial_reason"]
        .as_str()
        .unwrap()
        .starts_with("Auto mode could not evaluate"));
}

#[test]
fn session_start_rescans_only_when_instruction_files_changed() {
    let p = Project::new("session_start", "- Never run alembic upgrade by hand\n");
    // Codex/Claude shape: matcher value arrives as `source`.
    // Codex/Claude shape: matcher value arrives as `source`.  These hosts
    // already get the rules summary on UserPromptSubmit, so SessionStart
    // stays silent, and nothing changed since the scan: no audit entry.
    let out = p.hook(
        json!({"hook_event_name": "SessionStart", "session_id": SESSION,
        "source": "startup"}),
    );
    assert!(out.is_none(), "{out:?}");
    assert!(p.audit("SessionStart").is_empty());

    // An instruction file edited after the scan triggers one background
    // rescan, recorded with its trigger.  Grok's snake_case spelling
    // canonicalises to the same handler.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    fs::write(
        p.0.join("project/CLAUDE.md"),
        "- Never run alembic upgrade by hand\n- Never run docker prune\n",
    )
    .unwrap();
    let out = p.hook(
        json!({"hook_event_name": "session_start", "session_id": SESSION,
        "source": "resume"}),
    );
    assert!(out.is_none(), "{out:?}");
    let events = p.audit("SessionStart");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["payload"]["source"], "resume");
    assert_eq!(events[0]["payload"]["trigger"], "instruction_files_changed");
}

#[test]
fn passive_lifecycle_events_are_silent() {
    let p = Project::new("passive", "- Never run alembic upgrade by hand\n");
    for event in [
        "Stop",
        "Notification",
        "SessionEnd",
        "PreCompact",
        "PostCompact",
        "SubagentStop",
        "post_tool_use_failure",
    ] {
        let out = p.hook(json!({"hook_event_name": event, "session_id": SESSION,
            "tool_name": "Bash", "tool_input": {"command": "alembic upgrade head"}}));
        assert!(out.is_none(), "{event}: {out:?}");
    }
    assert!(p.audit("PreToolUse").is_empty());
}
