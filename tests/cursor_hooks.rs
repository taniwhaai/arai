//! Cursor Agent hooks: documented payloads in, Cursor-shaped decisions out.
//!
//! Payloads follow cursor.com/docs/agent/hooks.  `CURSOR_VERSION` and
//! `CLAUDE_PROJECT_DIR` are both exported, as they are inside Cursor's own
//! terminal, to pin that neither variable decides the response shape: only
//! the payload does.
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

struct Project(PathBuf);

impl Project {
    fn bare() -> Self {
        // Tests start on parallel threads within the same tick, so a
        // pid+timestamp name alone can collide (seen on the Windows runner:
        // two fixtures shared a root, one scan hit "database is locked" and
        // the other's project was torn down under it).  A process-wide
        // counter makes every root unique regardless of clock resolution.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "arai_cursor_{}_{}_{stamp}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("project/.git")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        Self(root)
    }

    fn new(rules: &str) -> Self {
        let project = Self::bare();
        fs::write(project.0.join("project/AGENTS.md"), rules).unwrap();
        project.scan();
        project
    }

    fn scan(&self) {
        let out = self.command().arg("scan").output().unwrap();
        assert!(
            out.status.success(),
            "scan: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// The session-start change check treats a file saved in the same
    /// second the scan started as possibly newer (inclusive compare), so
    /// tests that assert "nothing changed" first move the scan clearly past
    /// the fixture's write.
    fn settle(&self) {
        std::thread::sleep(std::time::Duration::from_millis(1100));
        self.scan();
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_arai"));
        cmd.current_dir(self.0.join("project"))
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("ARAI_BASE_DIR", self.0.join("state"))
            .env("ARAI_TELEMETRY", "off")
            .env("CURSOR_VERSION", "2.4.0")
            .env("CLAUDE_PROJECT_DIR", self.0.join("project"))
            .env_remove("ARAI_DISABLED")
            .env_remove("ARAI_DENY_MODE")
            .env_remove("GROK_HOOK_EVENT")
            .env_remove("GROK_SESSION_ID")
            .env_remove("CLAUDE_PLUGIN_ROOT");
        cmd
    }

    /// Pipe raw bytes and return (parsed stdout, exit code).
    fn hook_bytes(&self, payload: &[u8]) -> (Option<Value>, i32) {
        let mut child = self
            .command()
            .args(["guardrails", "--match-stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload).unwrap();
        let out = child.wait_with_output().unwrap();
        let code = out.status.code().unwrap_or(-1);
        if out.stdout.is_empty() {
            (None, code)
        } else {
            (
                Some(serde_json::from_slice(&out.stdout).expect("hook JSON")),
                code,
            )
        }
    }

    fn hook(&self, mut payload: Value) -> (Option<Value>, i32) {
        // Common fields every Cursor hook carries.
        let base = payload.as_object_mut().unwrap();
        base.entry("conversation_id")
            .or_insert(json!("cursor-conv-1"));
        base.entry("generation_id").or_insert(json!("gen-1"));
        base.entry("cursor_version").or_insert(json!("2.4.0"));
        base.entry("workspace_roots")
            .or_insert(json!([self.0.join("project").to_string_lossy()]));
        base.entry("cwd")
            .or_insert(json!(self.0.join("project").to_string_lossy()));
        self.hook_bytes(payload.to_string().as_bytes())
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

fn pre_tool_use(tool: &str, input: Value) -> Value {
    json!({
        "hook_event_name": "preToolUse",
        "tool_name": tool,
        "tool_input": input,
        "tool_use_id": "abc123",
    })
}

const RULES: &str = "## Alembic\n\n- Never hand-write migration files\n- Never run cargo clean\n";

#[test]
fn shell_deny_uses_cursor_permission_shape_and_exit_zero() {
    let p = Project::new(RULES);
    let (out, code) = p.hook(pre_tool_use(
        "Shell",
        json!({"command": "cargo clean", "working_directory": "/project"}),
    ));
    let out = out.expect("decision");
    assert_eq!(code, 0);
    assert_eq!(out["permission"], "deny");
    let user = out["user_message"].as_str().unwrap();
    let agent = out["agent_message"].as_str().unwrap();
    assert!(user.contains("cargo clean"), "{user}");
    assert!(
        agent.starts_with(user),
        "agent message leads with the reason: {agent}"
    );
    assert!(
        agent.len() > user.len(),
        "agent message carries the rule context"
    );
    assert!(
        out.get("hookSpecificOutput").is_none(),
        "Claude shape must not leak into a Cursor response: {out}"
    );
}

#[test]
fn environment_never_selects_the_cursor_shape() {
    // CURSOR_VERSION and CLAUDE_PROJECT_DIR are exported by the harness, as
    // they are for a Claude Code session started from Cursor's terminal.
    // A Claude-shaped payload must still get a Claude-shaped deny, or the
    // block is invisible to Claude Code (fail-open).
    let p = Project::new(RULES);
    let (out, code) = p.hook_bytes(
        json!({"hook_event_name": "PreToolUse", "tool_name": "Bash",
            "tool_input": {"command": "cargo clean"}, "session_id": "claude-1"})
        .to_string()
        .as_bytes(),
    );
    let out = out.expect("decision");
    assert_eq!(code, 0);
    assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(out.get("permission").is_none(), "{out}");
    // And a Claude-shaped no-match must stay silent (no Cursor allow).
    let (out, _) = p.hook_bytes(
        json!({"hook_event_name": "PreToolUse", "tool_name": "Bash",
            "tool_input": {"command": "cargo check"}})
        .to_string()
        .as_bytes(),
    );
    assert!(out.is_none(), "{out:?}");
}

#[test]
fn every_pretooluse_without_a_match_answers_an_explicit_allow() {
    let p = Project::new(RULES);
    // Nothing matched.
    let (out, code) = p.hook(pre_tool_use("Shell", json!({"command": "cargo check"})));
    assert_eq!((out, code), (Some(json!({"permission": "allow"})), 0));
    // Skipped tool (Read never needs guardrails).
    let (out, code) = p.hook(pre_tool_use("Read", json!({"file_path": "/p/a.py"})));
    assert_eq!((out, code), (Some(json!({"permission": "allow"})), 0));
    // Permission event without a tool name.
    let (out, code) = p.hook(json!({"hook_event_name": "beforeReadFile",
        "file_path": "/p/.env", "content": "SECRET=1", "attachments": []}));
    assert_eq!((out, code), (Some(json!({"permission": "allow"})), 0));
    // MCP tool with params serialised as a string, on the registered event.
    let (out, code) = p.hook(pre_tool_use(
        "linear_create_issue",
        json!("{\"title\":\"x\"}"),
    ));
    assert_eq!((out, code), (Some(json!({"permission": "allow"})), 0));
    // Cursor-spelled event without cursor_version still routes as Cursor.
    let (out, code) = p.hook_bytes(
        json!({"hook_event_name": "preToolUse", "tool_name": "Shell",
            "tool_input": {"command": "cargo check"}})
        .to_string()
        .as_bytes(),
    );
    assert_eq!((out, code), (Some(json!({"permission": "allow"})), 0));
    // Project that has never been scanned: no store, still an explicit allow.
    let bare = Project::bare();
    let (out, code) = bare.hook(pre_tool_use("Shell", json!({"command": "cargo clean"})));
    assert_eq!((out, code), (Some(json!({"permission": "allow"})), 0));
}

#[test]
fn malformed_and_unrecognised_input_fails_closed_in_cursor_shape() {
    let p = Project::new(RULES);
    // Unparseable: only the environment can name the host, and it must not,
    // so the deny is Claude-shaped.  Cursor treats a response that does not
    // match its schema as a block, so this still fails closed there.
    let (out, code) = p.hook_bytes(b"{\"hook_event_name\":\"preToolUse\",\"tool_input\":{");
    let out = out.expect("deny");
    assert_eq!(code, 0);
    assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
    // When the truncated bytes still carry Cursor's marker, the deny keeps
    // Cursor's shape and reason.
    let (out, code) = p.hook_bytes(
        b"{\"cursor_version\":\"2.4.0\",\"hook_event_name\":\"preToolUse\",\"tool_input\":{",
    );
    let out = out.expect("deny");
    assert_eq!(code, 0);
    assert_eq!(out["permission"], "deny");
    assert!(out["user_message"]
        .as_str()
        .unwrap()
        .contains("internal error"));
    // A Cursor-shaped shell event with no command is a Cursor-shaped deny.
    let (out, _) = p.hook(json!({"hook_event_name": "beforeShellExecution", "sandbox": false}));
    let out = out.unwrap();
    assert_eq!(out["permission"], "deny");
    assert!(out["user_message"]
        .as_str()
        .unwrap()
        .contains("internal error"));
    // A file write whose path is under no known field name is denied with a
    // reason that names the problem, so the real shape can be reported.
    let (out, _) = p.hook(pre_tool_use("Write", json!({"paths": ["a.py"]})));
    let out = out.unwrap();
    assert_eq!(out["permission"], "deny");
    assert!(
        out["user_message"].as_str().unwrap().contains("path"),
        "{out}"
    );
}

#[test]
fn shell_synonyms_and_file_field_aliases_are_matched() {
    let p = Project::new(RULES);
    // beforeShellExecution, if registered by hand, is the same decision.
    let (out, _) = p.hook(json!({"hook_event_name": "beforeShellExecution",
        "command": "cargo clean", "sandbox": false}));
    assert_eq!(out.unwrap()["permission"], "deny");
    // Write with the documented canonical name and with aliases.
    for input in [
        json!({"file_path": "alembic/new.py", "content": "pass"}),
        json!({"path": "alembic/new.py", "contents": "pass"}),
        json!({"target_file": "alembic/new.py", "code_edit": "pass"}),
    ] {
        let (out, _) = p.hook(pre_tool_use("Write", input.clone()));
        assert_eq!(out.unwrap()["permission"], "deny", "{input}");
    }
    let (out, _) = p.hook(pre_tool_use(
        "Write",
        json!({"file_path": "README.md", "content": "# hi"}),
    ));
    assert_eq!(out.unwrap()["permission"], "allow");
    // Delete is evaluated as an Edit on its path: the hand-write rule is
    // Write-scoped, so this is an explicit allow rather than a deny or a
    // validation failure.
    let (out, _) = p.hook(pre_tool_use(
        "Delete",
        json!({"target_file": "alembic/old.py"}),
    ));
    assert_eq!(out.unwrap(), json!({"permission": "allow"}));
}

#[test]
fn after_file_edit_feeds_post_observation_and_session_start_summarises() {
    let p = Project::new(RULES);
    p.settle();
    let (out, code) = p.hook(json!({"hook_event_name": "afterFileEdit",
        "file_path": "/p/notes/log.txt",
        "edits": [{"old_string": "", "new_string": "from alembic import op"}]}));
    assert_eq!(code, 0);
    assert!(
        out.is_none() || out.as_ref().unwrap().get("permission").is_none(),
        "post-edit observation must not emit a decision: {out:?}"
    );
    let session = fs::read_to_string(p.0.join("state/sessions/cursor-conv-1.json")).unwrap();
    assert!(
        session.contains("alembic"),
        "edit content must seed observed terms: {session}"
    );

    let session_start = json!({"hook_event_name": "sessionStart",
        "session_id": "cursor-conv-1", "is_background_agent": false, "composer_mode": "agent"});
    let (out, code) = p.hook(session_start.clone());
    assert_eq!(code, 0);
    let out = out.expect("session context");
    let context = out["additional_context"].as_str().unwrap();
    assert!(context.starts_with("Arai: "), "{context}");
    assert!(context.contains("active rule(s)"), "{context}");
    assert!(out.get("hookSpecificOutput").is_none());
    // Nothing changed since the scan: no rescan, no audit entry.
    assert!(p.audit("SessionStart").is_empty());

    // An instruction file edited after the scan triggers one rescan.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    fs::write(
        p.0.join("project/AGENTS.md"),
        format!("{RULES}- Never run docker prune\n"),
    )
    .unwrap();
    let (out, _) = p.hook(session_start);
    assert!(out.is_some());
    let events = p.audit("SessionStart");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["payload"]["trigger"], "instruction_files_changed");
}

#[test]
fn passive_and_unknown_cursor_events_are_silent() {
    let p = Project::new(RULES);
    for event in [
        "stop",
        "sessionEnd",
        "afterAgentResponse",
        "workspaceOpen",
        "preCompact",
        "postToolUseFailure",
    ] {
        let (out, code) = p.hook(json!({"hook_event_name": event, "text": "cargo clean"}));
        assert_eq!((out, code), (None, 0), "{event}");
    }
}

#[test]
fn scenario_replay_sees_cursor_payloads_like_the_live_hook() {
    // CLAUDE.md contract: match_hook is the single entry point, so a
    // recorded Cursor payload must replay through `arai test` unchanged.
    let p = Project::new(RULES);
    let scenarios = json!({"scenarios": [
        {"name": "cursor shell deny",
         "hook": {"hook_event_name": "beforeShellExecution", "conversation_id": "c1",
                  "command": "cargo clean", "sandbox": false},
         "expect": {"min_matches": 1}},
        {"name": "cursor write alias",
         "hook": {"hook_event_name": "preToolUse", "cursor_version": "2.4.0",
                  "tool_name": "Write", "tool_input": {"path": "alembic/new.py", "contents": "x"}},
         "expect": {"min_matches": 1}},
        {"name": "cursor read is silent",
         "hook": {"hook_event_name": "beforeReadFile", "cursor_version": "2.4.0",
                  "file_path": "/p/alembic/old.py", "content": ""},
         "expect": {"min_matches": 0}}
    ]});
    let file = p.0.join("cursor-scenarios.json");
    fs::write(&file, scenarios.to_string()).unwrap();
    let out = p
        .command()
        .args(["test", file.to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "arai test failed: {stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("cursor shell deny"), "{stdout}");
    assert!(stdout.contains("cursor write alias"), "{stdout}");
}
