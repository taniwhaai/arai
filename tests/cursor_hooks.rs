//! Native Cursor protocol checks against the executable, without running model
//! requests or the tool commands described by the payloads.
use arai::{
    intent::{Action, Severity, Timing},
    stats,
    store::Store,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

struct Project(PathBuf);

impl Project {
    fn empty() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("arai_cursor_{}_{stamp}", std::process::id()));
        fs::create_dir_all(root.join("project/.git")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        Self(root)
    }

    fn new(rules: &str) -> Self {
        let project = Self::empty();
        project.write("AGENTS.md", rules);
        project.scan();
        project
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.0.join("project").join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
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

    fn scan(&self) {
        let output = self.command().arg("scan").output().unwrap();
        assert!(
            output.status.success(),
            "scan: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn payload(&self, event: &str, tool: &str, input: Value) -> Value {
        json!({
            "hook_event_name": event,
            "tool_name": tool,
            "tool_input": input,
            "conversation_id": "conversation-1",
            "generation_id": "generation-1",
            "tool_use_id": "call-1",
            "cwd": self.0.join("project"),
            "workspace_roots": [self.0.join("project")],
            "tool_output": "{\"exitCode\":0,\"stdout\":\"from alembic import op\"}"
        })
    }

    fn raw_hook(&self, event: &str, payload: &[u8], environment: &[(&str, &str)]) -> Output {
        let mut command = self.command();
        command.args([
            "guardrails",
            "--match-stdin",
            "--platform",
            "cursor",
            "--hook-event",
            event,
        ]);
        for (key, value) in environment {
            command.env(key, value);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Err(error) = child.stdin.take().unwrap().write_all(payload) {
            // Oversize input may be rejected before the producer finishes.
            assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
            assert!(payload.len() > 1024 * 1024);
        }
        child.wait_with_output().unwrap()
    }

    fn hook(&self, event: &str, tool: &str, input: Value) -> Output {
        self.raw_hook(
            event,
            self.payload(event, tool, input).to_string().as_bytes(),
            &[],
        )
    }

    fn audit(&self) -> Vec<Value> {
        let mut files = Vec::new();
        collect_files(&self.0.join("state/audit"), "jsonl", &mut files);
        files
            .into_iter()
            .flat_map(|path| {
                fs::read_to_string(path)
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect::<Vec<Value>>()
            })
            .collect()
    }

    fn database(&self) -> PathBuf {
        let mut files = Vec::new();
        collect_files(&self.0.join("state/projects"), "db", &mut files);
        assert_eq!(
            files.len(),
            1,
            "expected one isolated rule store: {files:?}"
        );
        files.pop().unwrap()
    }

    fn session(&self) -> Option<Value> {
        let path = self.0.join("state/sessions/cursor-conversation-1.json");
        if path.exists() {
            Some(serde_json::from_slice(&fs::read(path).unwrap()).unwrap())
        } else {
            None
        }
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn collect_files(directory: &Path, extension: &str, files: &mut Vec<PathBuf>) {
    if !directory.exists() {
        return;
    }
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_files(&path, extension, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some(extension) {
            files.push(path);
        }
    }
}

fn response(output: &Output, exit_code: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(exit_code),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Parsing the entire stdout also rejects multiple responses and log chatter.
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "one Cursor response required: {error}; stdout={:?}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    assert!(
        value.get("hookSpecificOutput").is_none(),
        "Claude response leaked into Cursor"
    );
    assert!(
        value.get("decision").is_none(),
        "Grok response leaked into Cursor"
    );
    value
}

fn assert_deny(output: &Output) {
    let value = response(output, 2);
    assert_eq!(value["permission"], "deny");
    assert!(!value["user_message"].as_str().unwrap().is_empty());
    assert!(!value["agent_message"].as_str().unwrap().is_empty());
}

fn assert_allow(output: &Output) {
    assert_eq!(response(output, 0), json!({"permission":"allow"}));
}

#[test]
fn shell_denial_and_allow_use_cursor_contract_despite_other_host_environment() {
    let project = Project::new("- Never run cargo clean\n");
    let payload = project.payload("preToolUse", "Shell", json!({"command":"cargo clean"}));
    assert_deny(&project.raw_hook(
        "preToolUse",
        payload.to_string().as_bytes(),
        &[
            ("GROK_HOOK_EVENT", "post_tool_use"),
            ("GROK_SESSION_ID", "grok-session"),
            ("CLAUDE_PROJECT_DIR", "irrelevant-host-hint"),
        ],
    ));
    assert_allow(&project.hook("preToolUse", "Shell", json!({"command":"cargo check"})));
    let entries = project.audit();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["decision"], "deny");
    assert_eq!(entries[0]["session"], "cursor-conversation-1");
    assert_eq!(entries[0]["tool"], "Bash");
}

#[test]
fn no_database_skipped_tools_and_explicit_disable_still_emit_valid_allow() {
    let uninitialized = Project::empty();
    assert_allow(&uninitialized.hook("preToolUse", "Shell", json!({"command":"cargo clean"})));
    let mut databases = Vec::new();
    collect_files(
        &uninitialized.0.join("state/projects"),
        "db",
        &mut databases,
    );
    assert!(
        databases.is_empty(),
        "invocation receipts must not initialize policy"
    );

    let project = Project::new("- Never run cargo clean\n");
    assert_allow(&project.hook("preToolUse", "Read", json!({"file_path":"AGENTS.md"})));
    assert_allow(&project.hook("preToolUse", "Shell", json!({"command":"arai status"})));
    let payload = project.payload("preToolUse", "Shell", json!({"command":"cargo clean"}));
    assert_allow(&project.raw_hook(
        "preToolUse",
        payload.to_string().as_bytes(),
        &[("ARAI_DISABLED", "1")],
    ));
    let entries = project.audit();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["decision"], "bypassed");
}

#[test]
fn malformed_oversize_spoofed_and_corrupt_store_pre_hooks_fail_closed() {
    let project = Project::new("- Never run cargo clean\n");
    for payload in [
        b"{truncated".to_vec(),
        vec![0xff, 0xfe],
        vec![b' '; 1024 * 1024 + 1],
    ] {
        assert_deny(&project.raw_hook("preToolUse", &payload, &[]));
    }
    for event in ["postToolUse", "preToolUseFOO", "PreToolUse"] {
        let payload = project.payload(event, "Shell", json!({"command":"cargo clean"}));
        assert_deny(&project.raw_hook("preToolUse", payload.to_string().as_bytes(), &[]));
    }
    fs::write(project.database(), b"not a SQLite database").unwrap();
    assert_deny(&project.hook("preToolUse", "Shell", json!({"command":"cargo clean"})));
    assert!(project.audit().is_empty());
}

#[test]
fn unsupported_mutations_are_rejected_before_audit_or_session_effects() {
    let project = Project::new("## Alembic\n\n- Never hand-write migration files\n");
    for input in [
        json!({"file_path":"alembic/x.py"}),
        json!({"file_path":"alembic/x.py","content":"x","path":"safe.py"}),
        json!({"file_path":"alembic/x.py","content":"x","command":"unrecognized patch"}),
        json!({"file_path":"alembic/x.py","edits":[{"old_string":"x","new_string":"from alembic import op"},{"old_string":"y"}]}),
    ] {
        assert_deny(&project.hook("preToolUse", "Write", input));
    }
    assert!(project.audit().is_empty());
    assert!(project.session().is_none());
}

#[test]
fn cursor_write_enforces_both_creation_and_modification_rules_including_edits() {
    for rule in [
        "Never hand-write migration files",
        "Never modify migration files",
    ] {
        let project = Project::new(&format!("## Alembic\n\n- {rule}\n"));
        for input in [
            json!({"file_path":"alembic/x.py","content":""}),
            json!({"file_path":"scratch.py","content":"from alembic import op"}),
            json!({"file_path":"scratch.py","edits":[{"old_string":"pass","new_string":"from alembic import op"}]}),
        ] {
            assert_deny(&project.hook("preToolUse", "Write", input));
        }
    }
}

#[test]
fn cursor_file_scope_and_delete_path_coverage_remain_conservative() {
    let project = Project::empty();
    project.write(".cursor/rules/protected.mdc", "---\nglobs: ['protected/**']\nalwaysApply: false\n---\n## Alembic\n\n- Never modify migration files\n");
    project.scan();
    assert_deny(&project.hook(
        "preToolUse",
        "Write",
        json!({"file_path":"protected/alembic.py","content":""}),
    ));
    assert_allow(&project.hook(
        "preToolUse",
        "Write",
        json!({"file_path":"elsewhere/alembic.py","content":""}),
    ));
    assert_deny(&project.hook(
        "preToolUse",
        "Delete",
        json!({"file_path":"protected/alembic.py"}),
    ));
    assert_allow(&project.hook(
        "preToolUse",
        "Delete",
        json!({"file_path":"elsewhere/alembic.py"}),
    ));
}

#[test]
fn expanded_write_matches_produce_one_audit_entry_with_unique_rule_ids() {
    let project = Project::new("## Alembic\n\n- Never modify migration files\n");
    let db = Store::open(&project.database()).unwrap();
    let rules = db.load_guardrails().unwrap();
    assert_eq!(rules.len(), 1);
    let mut intent = rules[0].intent.clone().unwrap();
    intent.action = Action::General;
    intent.tools = vec!["Write".into(), "Edit".into()];
    intent.allow_inverse = false;
    db.upsert_rule_intent(rules[0].triple_id, &intent).unwrap();
    drop(db);
    assert_deny(&project.hook(
        "preToolUse",
        "Write",
        json!({"file_path":"alembic/x.py","content":"from alembic import op"}),
    ));
    let entries = project.audit();
    assert_eq!(
        entries.len(),
        1,
        "one native operation must have one firing"
    );
    let fired = entries[0]["rules"].as_array().unwrap();
    assert_eq!(fired.len(), 1, "the same rule matched Write and Edit");
    let ids: HashSet<_> = fired
        .iter()
        .map(|rule| rule["triple_id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids.len(), fired.len());
}

#[test]
fn post_output_observation_keeps_cursor_session_across_generations() {
    let project = Project::new("- Never run cargo clean\n");
    for generation in ["generation-1", "generation-2"] {
        let mut payload =
            project.payload("postToolUse", "Shell", json!({"command":"printf hello"}));
        payload["generation_id"] = json!(generation);
        payload["session_id"] = json!("spoofed-other-host-session");
        let value = response(
            &project.raw_hook("postToolUse", payload.to_string().as_bytes(), &[]),
            0,
        );
        assert!(value["additional_context"].is_string());
        assert!(value.get("permission").is_none());
    }
    let session = project.session().expect("normalized conversation session");
    let calls = session["tool_calls"].as_array().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|call| call["terms"]
        .as_array()
        .unwrap()
        .contains(&json!("alembic"))));
    assert!(!project
        .0
        .join("state/sessions/spoofed-other-host-session.json")
        .exists());
    assert!(project
        .audit()
        .iter()
        .all(|entry| entry["event"] != "Compliance"));
}

#[test]
fn invalid_post_output_is_observational_and_does_not_seed_session() {
    let project = Project::new("- Never run cargo clean\n");
    let mut payload = project.payload("postToolUse", "Shell", json!({"command":"cargo test"}));
    payload["tool_output"] = json!("not JSON");
    let output = project.raw_hook("postToolUse", payload.to_string().as_bytes(), &[]);
    assert_eq!(response(&output, 0), json!({"additional_context":""}));
    assert!(!output.stderr.is_empty());
    assert!(project.session().is_none());
    assert!(project.audit().is_empty());
}

#[test]
fn malformed_post_input_keeps_selected_cursor_protocol_before_json_parsing() {
    let project = Project::new("- Never run cargo clean\n");
    for payload in [b"{truncated".to_vec(), vec![0xff, 0xfe]] {
        let output = project.raw_hook(
            "postToolUse",
            &payload,
            &[
                ("GROK_HOOK_EVENT", "pre_tool_use"),
                ("GROK_SESSION_ID", "grok-session"),
                ("CLAUDE_PROJECT_DIR", "irrelevant-host-hint"),
            ],
        );
        assert_eq!(response(&output, 0), json!({"additional_context":""}));
        assert!(!output.stderr.is_empty());
    }
    assert!(project.session().is_none());
    assert!(project.audit().is_empty());
}

#[test]
fn advisory_matches_allow_without_claiming_injection_or_compliance() {
    let project = Project::new("- Always run cargo clean\n");
    let db = Store::open(&project.database()).unwrap();
    let rules = db.load_guardrails().unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].intent.as_ref().unwrap().severity, Severity::Warn);
    drop(db);

    for _ in 0..2 {
        assert_allow(&project.hook("preToolUse", "Shell", json!({"command":"cargo clean"})));
    }
    assert!(project.session().is_none());
    response(
        &project.hook("postToolUse", "Shell", json!({"command":"cargo clean"})),
        0,
    );
    let entries = project.audit();
    assert_eq!(entries.len(), 2);
    for entry in &entries {
        assert_eq!(entry["decision"], "allow");
        assert_eq!(entry["event"], "PreToolUse");
        assert_eq!(entry["rules"][0]["severity"], "warn");
        assert_eq!(entry["rules"][0]["seen_before"], false);
    }
    assert!(project.session().unwrap()["seen_rules"]
        .as_array()
        .unwrap()
        .is_empty());
    let computed = stats::compute(&entries);
    assert_eq!(computed.token_economics.suppressed_repeats, 0);
    assert_eq!(computed.token_economics.advisory_obeyed, 0);
    assert_eq!(computed.token_economics.estimated_tokens_saved, 0);
}

#[test]
fn edit_block_survives_higher_scoring_write_advisory() {
    let project = Project::new(
        "## Alembic\n\n- Never modify migration files\n- Always review migration files with alembic\n",
    );
    let db = Store::open(&project.database()).unwrap();
    let rules = db.load_guardrails().unwrap();
    assert_eq!(rules.len(), 2);
    for rule in &rules {
        let mut intent = rule.intent.clone().unwrap();
        intent.action = Action::General;
        intent.timing = Timing::ToolCall;
        intent.allow_inverse = false;
        intent.tools = vec![if intent.severity == Severity::Block {
            "Edit"
        } else {
            "Write"
        }
        .into()];
        db.upsert_rule_intent(rule.triple_id, &intent).unwrap();
    }
    drop(db);
    assert_deny(&project.hook(
        "preToolUse",
        "Write",
        json!({
            "file_path":"alembic/migration.py", "content":"alembic migration review"
        }),
    ));
    let entries = project.audit();
    assert_eq!(entries.len(), 1);
    let fired = entries[0]["rules"].as_array().unwrap();
    assert_eq!(fired.len(), 2);
    let block = fired
        .iter()
        .find(|rule| rule["severity"] == "block")
        .unwrap();
    let warn = fired
        .iter()
        .find(|rule| rule["severity"] == "warn")
        .unwrap();
    assert!(warn["match_pct"].as_u64().unwrap() > block["match_pct"].as_u64().unwrap());
}

#[test]
fn allow_side_matches_claim_neither_injection_nor_compliance_savings() {
    let project = Project::new("- Never run cargo clean\n");
    let payload = project.payload("preToolUse", "Shell", json!({"command":"cargo clean"}));
    for _ in 0..2 {
        assert_allow(&project.raw_hook(
            "preToolUse",
            payload.to_string().as_bytes(),
            &[("ARAI_DENY_MODE", "off")],
        ));
    }
    assert!(
        project.session().is_none(),
        "unsurfaced rules must not be marked seen"
    );
    response(
        &project.hook("postToolUse", "Shell", json!({"command":"cargo clean"})),
        0,
    );
    let entries = project.audit();
    assert_eq!(entries.len(), 2);
    for entry in &entries {
        assert_eq!(entry["decision"], "allow");
        assert_eq!(entry["event"], "PreToolUse");
        for rule in entry["rules"].as_array().unwrap() {
            assert_eq!(rule["seen_before"], false);
        }
    }
    let session = project.session().unwrap();
    assert!(session["seen_rules"].as_array().unwrap().is_empty());
    let computed = stats::compute(&entries);
    assert_eq!(computed.token_economics.suppressed_repeats, 0);
    assert_eq!(computed.token_economics.advisory_obeyed, 0);
    assert_eq!(computed.token_economics.blocked_obeyed, 0);
    assert_eq!(computed.token_economics.estimated_tokens_saved, 0);
}
