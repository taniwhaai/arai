//! Native verb regressions through the public matcher and CLI. Payload commands
//! describe proposed actions only; these tests never execute those commands.
use arai::{config::Config, hooks, intent::Severity, store::Store};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

struct Project(PathBuf);

impl Project {
    fn new(rules: &str) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("arai_host_verbs_{}_{stamp}", std::process::id()));
        fs::create_dir_all(root.join("project/.git")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        fs::write(root.join("project/AGENTS.md"), rules).unwrap();
        let project = Self(root);
        let result = project.command().arg("scan").output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        project
    }

    fn config(&self) -> Config {
        Config {
            project_root: self.0.join("project"),
            home_dir: self.0.join("home"),
            arai_base_dir: self.0.join("state"),
            extra_sources: Vec::new(),
            guardrails_mode: "advise".into(),
            llm_command: None,
            api_url: None,
            api_key_env: None,
            api_model: None,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_arai"));
        command
            .current_dir(self.0.join("project"))
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("ARAI_BASE_DIR", self.0.join("state"))
            .env("ARAI_TELEMETRY", "off")
            .env("DO_NOT_TRACK", "1")
            .env_remove("ARAI_DISABLED")
            .env_remove("ARAI_DENY_MODE")
            .env_remove("GROK_HOOK_EVENT")
            .env_remove("GROK_SESSION_ID")
            .env_remove("CLAUDE_PROJECT_DIR")
            .env_remove("CLAUDE_PLUGIN_ROOT");
        command
    }

    fn payload(&self, tool: &str, input: Value) -> Value {
        json!({"hook_event_name":"PreToolUse", "tool_name":tool, "tool_input":input,
            "session_id":"host-verb-session", "cwd":self.0.join("project")})
    }

    fn matched(&self, payload: &Value) -> Result<hooks::HookMatch, String> {
        let cfg = self.config();
        let db = Store::open(&cfg.db_path()).unwrap();
        hooks::match_hook(payload, &cfg, &db)
    }

    fn invoke(&self, platform: &str, payload: &Value) -> Output {
        let mut child = self
            .command()
            .args([
                "guardrails",
                "--match-stdin",
                "--platform",
                platform,
                "--hook-event",
                "PreToolUse",
            ])
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

    fn denies(&self, platform: &str, payload: &Value) -> bool {
        let result = self.invoke(platform, payload);
        let response: Option<Value> = if result.stdout.is_empty() {
            None
        } else {
            Some(serde_json::from_slice(&result.stdout).unwrap())
        };
        let denied = response.as_ref().is_some_and(|value| {
            value["decision"] == "deny"
                || value["hookSpecificOutput"]["permissionDecision"] == "deny"
        });
        assert_eq!(
            result.status.code(),
            Some(if platform == "grok" && denied { 2 } else { 0 }),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        denied
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

#[test]
fn powershell_and_command_monitor_share_cli_and_embedding_execution_rules() {
    let project = Project::new("- Never run cargo clean\n");
    for tool in ["PowerShell", "Monitor"] {
        let denied = project.payload(tool, json!({"command":"cargo clean"}));
        let matched = project.matched(&denied).unwrap();
        assert_eq!(matched.tool_name, "Bash");
        assert_eq!(hooks::highest_severity(&matched.matched), Severity::Block);
        assert!(project.denies("claude", &denied), "{tool}");
        let allowed = project.payload(tool, json!({"command":"cargo check"}));
        assert!(project.matched(&allowed).unwrap().matched.is_empty());
        assert!(!project.denies("claude", &allowed));
    }
}

#[test]
fn websocket_monitor_keeps_its_verb_and_does_not_match_shell_rules() {
    let project = Project::new("- Never run cargo clean\n");
    let payload = project.payload(
        "Monitor",
        json!({"ws":{"url":"wss://example.test/cargo"},
        "description":"cargo clean notifications", "timeout_ms":1000}),
    );
    let result = project.matched(&payload).unwrap();
    assert_eq!(result.tool_name, "Monitor");
    assert!(result.matched.is_empty());
    assert!(!project.denies("claude", &payload));
    assert!(!project
        .0
        .join("state/sessions/host-verb-session.json")
        .exists());
}

#[test]
fn malformed_known_commands_are_rejected_by_embedding_and_cli() {
    let project = Project::new("- Never run cargo clean\n");
    for (tool, input) in [
        ("PowerShell", json!({})),
        ("PowerShell", json!({"command":null})),
        ("Monitor", json!({"command":42})),
        (
            "Monitor",
            json!({"command":"cargo clean", "ws":{"url":"wss://example.test/events"}}),
        ),
    ] {
        let payload = project.payload(tool, input);
        assert!(project.matched(&payload).is_err(), "{payload}");
        assert!(project.denies("claude", &payload), "{payload}");
    }
}

#[test]
fn grok_empty_search_enforces_creation_and_modification_with_one_shared_matcher() {
    for rule in [
        "Never hand-write migration files",
        "Never modify migration files",
    ] {
        let project = Project::new(&format!("## Alembic\n\n- {rule}\n"));
        let payload = project.payload("search_replace", json!({"file_path":"scratch.py", "old_string":"", "new_string":"from alembic import op"}));
        let matched = project.matched(&payload).unwrap();
        assert_eq!(
            hooks::highest_severity(&matched.matched),
            Severity::Block,
            "{rule}"
        );
        assert_eq!(matched.session_id, "host-verb-session");
        assert!(project.denies("grok", &payload), "{rule}");
    }
    let project = Project::new("## Alembic\n\n- Never hand-write migration files\n");
    let payload = project.payload("search_replace", json!({"file_path":"alembic/x.py", "old_string":"pass", "new_string":"from alembic import op"}));
    assert!(
        project.matched(&payload).unwrap().matched.is_empty(),
        "an existing-text Edit must not become Write"
    );
    assert!(!project.denies("grok", &payload));
}

#[test]
fn grok_malformed_replacements_fail_closed_in_both_entry_points() {
    let project = Project::new("## Alembic\n\n- Never hand-write migration files\n");
    for input in [
        json!({"file_path":"safe.py", "new_string":"from alembic import op"}),
        json!({"file_path":"safe.py", "old_string":null, "new_string":"from alembic import op"}),
        json!({"file_path":"safe.py", "old_string":"", "new_string":false}),
        json!({"file_path":"", "old_string":"", "new_string":""}),
    ] {
        let payload = project.payload("search_replace", input);
        assert!(project.matched(&payload).is_err(), "{payload}");
        assert!(project.denies("grok", &payload), "{payload}");
    }
}

#[test]
fn grok_aliases_camel_inputs_and_embedding_purity_are_preserved() {
    let project = Project::new("## Alembic\n\n- Never hand-write migration files\n");
    let payload = json!({"hookEventName":"pre_tool_use", "toolName":"search_replace",
        "toolInput":{"file_path":"alembic/x.py", "old_string":"", "new_string":""},
        "sessionId":"camel-session", "cwd":project.0.join("project")});
    let result = project.matched(&payload).unwrap();
    assert_eq!(result.session_id, "camel-session");
    assert_eq!(hooks::highest_severity(&result.matched), Severity::Block);
    let grep = project
        .matched(&project.payload("grep", json!({"pattern":"alembic"})))
        .unwrap();
    assert_eq!(grep.tool_name, "Grep");
    let agent = project
        .matched(&project.payload("spawn_subagent", json!({"prompt":"inspect alembic"})))
        .unwrap();
    assert!(agent.skipped);
    assert_eq!(agent.tool_name, "Agent");
    assert!(!project.0.join("state/audit").exists());
    assert!(!project.0.join("state/sessions").exists());
}
