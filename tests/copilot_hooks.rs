//! GitHub Copilot command hooks.
//!
//! Payloads follow the published hooks reference. The Copilot CLI is not
//! installed here, so these tests replay the registered command locally.
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

struct Project(PathBuf);

impl Project {
    fn new(rules: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "arai_copilot_{}_{}_{stamp}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("project")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        let init = Command::new("git")
            .args(["init"])
            .current_dir(root.join("project"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", root.join("home/.gitconfig"))
            .env("HOME", root.join("home"))
            .output()
            .unwrap();
        assert!(
            init.status.success(),
            "git init: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        fs::write(root.join("project/AGENTS.md"), rules).unwrap();
        let project = Self(root);
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

    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_arai"));
        cmd.current_dir(self.0.join("project"))
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("ARAI_BASE_DIR", self.0.join("state"))
            .env("ARAI_TELEMETRY", "off")
            .env("CLAUDE_PROJECT_DIR", self.0.join("project"))
            .env_remove("ARAI_DISABLED")
            .env_remove("ARAI_DENY_MODE")
            .env_remove("GROK_HOOK_EVENT")
            .env_remove("GROK_SESSION_ID")
            .env_remove("CLAUDE_PLUGIN_ROOT");
        cmd
    }

    fn hook_bytes(&self, payload: &[u8]) -> (String, i32) {
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
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    /// Write one complete payload and leave stdin open. The hook must answer
    /// before that pipe closes.
    fn hook_open_stdin(&self, payload: &[u8]) -> (String, i32) {
        let mut child = self
            .command()
            .args(["guardrails", "--match-stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(payload).unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            let _ = tx.send(buf);
        });
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(buf) => {
                let status = child.wait().unwrap();
                drop(stdin);
                (
                    String::from_utf8_lossy(&buf).into_owned(),
                    status.code().unwrap_or(-1),
                )
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(stdin);
                panic!("hook did not answer while stdin stayed open");
            }
        }
    }

    fn audit(&self, event: &str) -> Vec<Value> {
        let out = self
            .command()
            .args(["audit", &format!("--event={event}"), "--json", "--limit=20"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

const RULES: &str = "## Alembic\n\n- Never hand-write migration files\n- Never run cargo clean\n";

fn bash_payload(command: &str) -> Vec<u8> {
    // `toolArgs` is a JSON string, which is the form the Copilot CLI sends.
    // `cwd` is omitted: a directory-scoped rule follows the payload cwd, and
    // these rules belong to the project the process was started in.
    serde_json::to_vec(&json!({
        "sessionId": "copilot-1",
        "toolName": "bash",
        "toolArgs": format!("{{\"command\":{}}}", serde_json::to_string(command).unwrap())
    }))
    .unwrap()
}

#[test]
fn bash_string_args_deny_on_exit_zero_and_silence_is_allow() {
    let project = Project::new(RULES);
    let (stdout, code) = project.hook_bytes(&bash_payload("cargo clean"));
    assert_eq!(code, 0, "{stdout}");
    let deny: Value = serde_json::from_str(stdout.trim()).expect(&stdout);
    assert_eq!(deny["permissionDecision"], "deny");
    assert!(
        deny["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .contains("cargo clean"),
        "{deny}"
    );
    assert!(deny.get("hookSpecificOutput").is_none(), "{deny}");
    assert!(deny.get("additionalContext").is_none(), "{deny}");

    let (stdout, code) = project.hook_bytes(&bash_payload("cargo check"));
    assert_eq!(code, 0);
    assert!(stdout.trim().is_empty(), "{stdout}");

    let (stdout, code) = project.hook_open_stdin(&bash_payload("cargo clean"));
    assert_eq!(code, 0, "{stdout}");
    let deny: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(deny["permissionDecision"], "deny");
}

#[test]
fn create_path_denies_and_a_grouped_claude_payload_stays_claude() {
    let project = Project::new(RULES);
    let (stdout, code) = project.hook_bytes(
        &serde_json::to_vec(&json!({
            "sessionId": "copilot-1",
            "toolName": "create",
            "toolArgs": { "path": "alembic/new.py", "contents": "from alembic import op\n" }
        }))
        .unwrap(),
    );
    assert_eq!(code, 0, "{stdout}");
    let deny: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(deny["permissionDecision"], "deny");

    let (stdout, code) = project.hook_bytes(
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo clean"}}"#,
    );
    assert_eq!(code, 0, "{stdout}");
    let claude: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(claude["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(claude.get("permissionDecision").is_none(), "{claude}");
}

#[test]
fn advisory_pre_tool_use_is_silent_and_post_tool_use_carries_context() {
    let project = Project::new(RULES);
    let mut cmd = project.command();
    cmd.env("ARAI_DENY_MODE", "off");
    let mut child = cmd
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
        .write_all(&bash_payload("cargo clean"))
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let events = project.audit("PreToolUse");
    assert!(
        events.iter().any(|event| event["decision"] == "inject"),
        "{events:?}"
    );

    // Tool-call rules are timed to preToolUse, so a normal postToolUse match
    // list is empty and stdout stays quiet. The call is still recorded.
    let (stdout, code) = project.hook_bytes(
        &serde_json::to_vec(&json!({
            "sessionId": "copilot-1",
            "toolName": "bash",
            "toolArgs": {"command": "cargo clean"},
            "toolResult": {"resultType": "success", "textResultForLlm": "removed"}
        }))
        .unwrap(),
    );
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.trim().is_empty(), "{stdout}");
    let session = fs::read_to_string(project.0.join("state/sessions/copilot-1.json")).unwrap();
    assert!(
        session.to_ascii_lowercase().contains("cargo"),
        "post-tool observation must record the command: {session}"
    );
}

#[test]
fn session_start_summarises_and_malformed_tool_args_exit_two() {
    let project = Project::new(RULES);
    std::thread::sleep(Duration::from_millis(1100));
    project.scan();
    let (stdout, code) = project.hook_bytes(
        &serde_json::to_vec(&json!({
            "sessionId": "copilot-1",
            "cwd": project.0.join("project"),
            "source": "startup"
        }))
        .unwrap(),
    );
    assert_eq!(code, 0, "{stdout}");
    let start: Value = serde_json::from_str(stdout.trim()).expect(&stdout);
    let context = start["additionalContext"].as_str().unwrap();
    assert!(context.contains("active rule"), "{context}");
    assert!(start.get("permissionDecision").is_none(), "{start}");

    let (stdout, code) =
        project.hook_bytes(br#"{"toolName":"bash","toolArgs":{"command":"cargo clean"}"#);
    assert_eq!(code, 2, "{stdout}");
    let deny: Value = serde_json::from_str(stdout.trim()).expect(&stdout);
    assert_eq!(deny["permissionDecision"], "deny");
    assert!(deny.get("hookSpecificOutput").is_none(), "{deny}");
}

#[test]
fn empty_session_without_a_store_does_not_fail_closed() {
    let project = Project::new(RULES);
    fs::remove_dir_all(project.0.join("state")).unwrap();
    let (stdout, code) =
        project.hook_bytes(br#"{"sessionId":"copilot-1","cwd":"/tmp/project","source":"startup"}"#);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.trim().is_empty(), "{stdout}");
}

#[test]
fn registration_preserves_neighbours_and_deinit_drops_an_owned_file() {
    let project = Project::new(RULES);
    let path = project.0.join("project/.github/hooks/arai.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let neighbour = json!({"type": "command", "bash": "echo hello", "timeoutSec": 5});
    fs::write(
        &path,
        serde_json::to_string_pretty(&json!({
            "version": 1,
            "description": "keep me",
            "hooks": {
                "preToolUse": [neighbour],
                "stop": [{"type": "command", "bash": "./audit.sh"}]
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let init = project.command().arg("init").output().unwrap();
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    let body: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(body["version"], 1);
    assert_eq!(body["description"], "keep me");
    let pre = body["hooks"]["preToolUse"].as_array().unwrap();
    assert_eq!(pre[0], neighbour);
    assert_eq!(pre.len(), 2, "{pre:?}");
    let handler = &pre[1];
    assert!(handler.get("command").is_none(), "{handler}");
    let bash = handler["bash"].as_str().unwrap();
    let exe = env!("CARGO_BIN_EXE_arai").replace('\\', "/");
    assert!(bash.contains(&exe), "{bash}");
    assert!(bash.contains("guardrails --match-stdin"), "{bash}");
    assert!(bash.contains("else exit 0"), "{bash}");
    assert_eq!(handler["timeoutSec"], 3);
    assert!(handler["powershell"]
        .as_str()
        .unwrap()
        .starts_with("powershell.exe "));
    assert_eq!(body["hooks"]["sessionStart"][0]["timeoutSec"], 10);
    let again = project.command().arg("init").output().unwrap();
    assert!(again.status.success());
    let refreshed: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        refreshed, body,
        "a second init duplicated or rewrote handlers"
    );

    let test = project
        .command()
        .args([
            "test",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/copilot-pre-tool-use.json"
            ),
        ])
        .output()
        .unwrap();
    assert!(
        test.status.success(),
        "{}{}",
        String::from_utf8_lossy(&test.stdout),
        String::from_utf8_lossy(&test.stderr)
    );

    let deinit = project.command().arg("deinit").output().unwrap();
    assert!(
        deinit.status.success(),
        "{}",
        String::from_utf8_lossy(&deinit.stderr)
    );
    let left: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(left["description"], "keep me");
    assert_eq!(left["hooks"]["preToolUse"], json!([neighbour]));
    assert_eq!(left["hooks"]["stop"][0]["bash"], "./audit.sh");
    assert!(left["hooks"]["postToolUse"].as_array().unwrap().is_empty());

    let owned = Project::new(RULES);
    let owned_init = owned.command().arg("init").output().unwrap();
    assert!(owned_init.status.success());
    let owned_path = owned.0.join("project/.github/hooks/arai.json");
    assert!(owned_path.is_file());
    let owned_deinit = owned.command().arg("deinit").output().unwrap();
    assert!(owned_deinit.status.success());
    assert!(
        !owned_path.exists(),
        "an Arai-only Copilot file must be removed"
    );
}
