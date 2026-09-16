//! Platform selection is project-local, persistent, and ownership-preserving.
use base64::Engine;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    home: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "arai_platform_{label}_{}_{nanos}",
            std::process::id()
        ));
        let fixture = Self {
            project: root.join("project"),
            home: root.join("home"),
            root,
        };
        fs::create_dir_all(&fixture.project).unwrap();
        fs::create_dir_all(&fixture.home).unwrap();
        let mut command = Command::new("git");
        fixture.isolate(&mut command);
        assert_success(&command.arg("init").output().unwrap());
        fixture
    }

    fn isolate(&self, command: &mut Command) {
        command
            .current_dir(&self.project)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("ARAI_BASE_DIR", self.root.join("state"))
            .env("ARAI_TELEMETRY", "off")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join(".gitconfig"))
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GROK_HOOK_EVENT")
            .env_remove("GROK_SESSION_ID")
            .env_remove("CURSOR_VERSION")
            .env_remove("CURSOR_PROJECT_DIR")
            .env_remove("CLAUDE_PROJECT_DIR")
            .env_remove("CLAUDE_PLUGIN_ROOT");
    }

    fn invoke(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_arai"));
        self.isolate(&mut command);
        command.args(args).output().unwrap()
    }

    fn run(&self, args: &[&str]) -> Output {
        let output = self.invoke(args);
        assert_success(&output);
        output
    }

    fn write(&self, path: &str, value: &Value) {
        let path = self.project.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn read(&self, path: &str) -> Value {
        serde_json::from_slice(&fs::read(self.project.join(path)).unwrap()).unwrap()
    }

    fn store(&self) -> arai::store::Store {
        let directory = fs::read_dir(self.root.join("state/projects"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        arai::store::Store::open(&directory.join("arai.db")).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn decoded_command(command: &str) -> String {
    if let Some(encoded) =
        command.strip_prefix("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ")
    {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let words: Vec<_> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16(&words).unwrap()
    } else {
        command.to_string()
    }
}

#[test]
fn cursor_only_selection_persists_through_init_and_add() {
    let fixture = Fixture::new("selection");
    let output = fixture.run(&["init", "--platform", "cursor", "--platform", "cursor"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("third-party Claude hook imports") && stdout.contains("duplicate"));
    let settings = fixture.read(".cursor/hooks.json");
    assert_eq!(settings["version"], 1);
    assert_eq!(settings["hooks"].as_object().unwrap().len(), 2);
    for event in ["preToolUse", "postToolUse"] {
        let handlers = settings["hooks"][event].as_array().unwrap();
        assert_eq!(handlers.len(), 1);
        let handler = &handlers[0];
        assert_eq!(handler["type"], "command");
        assert_eq!(handler["timeout"], 3);
        assert!(handler.get("hooks").is_none());
        assert!(handler.get("commandWindows").is_none());
        let command = decoded_command(handler["command"].as_str().unwrap());
        assert!(
            command.contains(&format!(
                "guardrails --match-stdin --platform cursor --hook-event {event}"
            )),
            "{command}"
        );
        assert!(command.contains(env!("CARGO_BIN_EXE_arai")), "{command}");
    }
    assert_eq!(settings["hooks"]["preToolUse"][0]["failClosed"], true);
    let original = fs::read(fixture.project.join(".cursor/hooks.json")).unwrap();
    fixture.run(&["init"]);
    fixture.run(&["add", "Never run git push --force"]);
    assert_eq!(
        fs::read(fixture.project.join(".cursor/hooks.json")).unwrap(),
        original
    );
    for path in [
        ".claude/settings.json",
        ".grok/hooks/arai.json",
        ".codex/hooks.json",
    ] {
        assert!(!fixture.project.join(path).exists(), "unexpected {path}");
    }
    assert_eq!(
        fixture.store().get_meta("hook_platforms").unwrap().unwrap(),
        r#"["cursor"]"#
    );
}

#[test]
fn cursor_refresh_and_deinit_preserve_neighbours_and_claude_imports() {
    let fixture = Fixture::new("ownership");
    let other = json!({"command":"echo arai audit", "timeout":45});
    let custom = json!({"type":"command", "command":"arai guardrails --match-stdin --platform cursor --hook-event preToolUse && notify"});
    let wrong_platform = json!({"type":"command", "command":"arai guardrails --match-stdin --platform claude --hook-event PreToolUse"});
    let legacy = json!({"type":"command", "command":"arai guardrails --match-stdin"});
    fixture.write(".cursor/hooks.json", &json!({"version":1, "description":"keep", "hooks":{
        "preToolUse":[other, custom, wrong_platform, legacy,
            {"command":"'/old install/arai' guardrails --match-stdin --platform cursor --hook-event preToolUse"}],
        "postToolUse":[{"type":"command", "command":"arai guardrails --match-stdin --platform cursor --hook-event postToolUse"}],
        "stop":[{"command":"notify done"}]
    }}));
    let claude = json!({"hooks":{"PreToolUse":[{"hooks":[legacy]}]}});
    fixture.write(".claude/settings.json", &claude);
    fixture.write(".claude/settings.local.json", &claude);
    let before = fs::read(fixture.project.join(".claude/settings.json")).unwrap();
    fixture.run(&["init", "--platform", "cursor"]);
    let first = fixture.read(".cursor/hooks.json");
    assert_eq!(first["hooks"]["preToolUse"].as_array().unwrap().len(), 5);
    assert_eq!(first["hooks"]["postToolUse"].as_array().unwrap().len(), 1);
    assert!(!first.to_string().contains("old install"));
    fixture.run(&["init", "--platform", "cursor"]);
    assert_eq!(fixture.read(".cursor/hooks.json"), first);
    fixture.run(&["deinit", "--platform", "cursor"]);
    let after = fixture.read(".cursor/hooks.json");
    assert_eq!(after["description"], "keep");
    assert_eq!(
        after["hooks"]["preToolUse"],
        json!([other, custom, wrong_platform, legacy])
    );
    assert_eq!(after["hooks"]["postToolUse"], json!([]));
    assert_eq!(after["hooks"]["stop"], first["hooks"]["stop"]);
    assert_eq!(
        fs::read(fixture.project.join(".claude/settings.json")).unwrap(),
        before
    );
    assert_eq!(fixture.read(".claude/settings.local.json"), claude);
    fixture.run(&["add", "Never run git push --force"]);
    assert_eq!(fixture.read(".cursor/hooks.json"), after);
    assert_eq!(
        fixture.store().get_meta("hook_platforms").unwrap().unwrap(),
        "[]"
    );
}

#[test]
fn selected_deinit_keeps_other_hosts_and_precommit() {
    let fixture = Fixture::new("deinit");
    fixture.run(&[
        "init",
        "--platform",
        "claude",
        "--platform",
        "cursor",
        "--pre-commit",
    ]);
    let cursor = fs::read(fixture.project.join(".cursor/hooks.json")).unwrap();
    let precommit = fs::read(fixture.project.join(".git/hooks/pre-commit")).unwrap();
    fixture.run(&["deinit", "--platform", "claude"]);
    assert_eq!(
        fs::read(fixture.project.join(".cursor/hooks.json")).unwrap(),
        cursor
    );
    assert_eq!(
        fs::read(fixture.project.join(".git/hooks/pre-commit")).unwrap(),
        precommit
    );
    fixture.run(&["add", "Never run git push --force"]);
    let claude = fixture.read(".claude/settings.json");
    assert!(claude["hooks"]
        .as_object()
        .unwrap()
        .values()
        .all(|hooks| hooks == &json!([])));
    assert_eq!(
        fixture.store().get_meta("hook_platforms").unwrap().unwrap(),
        r#"["cursor"]"#
    );
    fixture.run(&["deinit"]);
    assert!(!fixture.project.join(".git/hooks/pre-commit").exists());
    fixture.run(&["add", "Never run git reset --hard"]);
    fixture.run(&["init"]);
    assert!(fixture.read(".cursor/hooks.json")["hooks"]
        .as_object()
        .unwrap()
        .values()
        .all(|hooks| hooks == &json!([])));
    assert!(!fixture.project.join(".codex/hooks.json").exists());
    assert!(!fixture.project.join(".grok/hooks/arai.json").exists());
}

#[test]
fn explicit_host_ownership_does_not_claim_another_adapter_or_custom_arguments() {
    let fixture = Fixture::new("explicit_ownership");
    let foreign = json!({"type":"command", "command":"arai guardrails --match-stdin --platform cursor --hook-event preToolUse"});
    let customized = json!({"type":"command", "command":"arai guardrails --match-stdin --platform claude --json"});
    fixture.write(".claude/settings.json", &json!({"theme":"keep", "hooks":{"PreToolUse":[{"matcher":"Bash", "hooks":[
        {"type":"command", "command":"'/old path/arai' guardrails --match-stdin --platform claude"},
        {"type":"command", "command":"arai guardrails --match-stdin --platform claude --hook-event PreToolUse"},
        foreign, customized
    ]}]}}));
    fixture.run(&["init", "--platform", "claude"]);
    let settings = fixture.read(".claude/settings.json");
    assert_eq!(settings["theme"], "keep");
    assert_eq!(settings["hooks"]["PreToolUse"][0]["matcher"], "Bash");
    assert_eq!(
        settings["hooks"]["PreToolUse"][0]["hooks"],
        json!([foreign, customized])
    );
    assert_eq!(settings["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
    fixture.run(&["deinit", "--platform", "claude"]);
    let after = fixture.read(".claude/settings.json");
    assert_eq!(after["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
    assert_eq!(
        after["hooks"]["PreToolUse"][0]["hooks"],
        json!([foreign, customized])
    );
    assert!(!fixture.project.join(".cursor/hooks.json").exists());
}

#[test]
fn invalid_cursor_config_does_not_partially_register_other_selected_hosts() {
    for (label, value) in [
        ("version", json!({"version":2,"hooks":{}})),
        ("missing_version", json!({"hooks":{}})),
        ("hooks_object", json!({"version":1,"hooks":[]})),
        (
            "event_array",
            json!({"version":1,"hooks":{"preToolUse":{}}}),
        ),
        (
            "flat_shape",
            json!({"version":1,"hooks":{"preToolUse":[{"hooks":[]}]}}),
        ),
        (
            "entry_object",
            json!({"version":1,"hooks":{"preToolUse":["script.sh"]}}),
        ),
    ] {
        let fixture = Fixture::new(label);
        fixture.write(".cursor/hooks.json", &value);
        let bytes = fs::read(fixture.project.join(".cursor/hooks.json")).unwrap();
        let output = fixture.invoke(&["init", "--platform", "claude", "--platform", "cursor"]);
        assert!(!output.status.success(), "accepted {label}");
        assert_eq!(
            fs::read(fixture.project.join(".cursor/hooks.json")).unwrap(),
            bytes
        );
        assert!(!fixture.project.join(".claude/settings.json").exists());
        assert_eq!(fixture.store().get_meta("hook_platforms").unwrap(), None);
        let output = fixture.invoke(&["deinit", "--platform", "cursor"]);
        assert!(!output.status.success(), "removed malformed {label}");
        assert_eq!(
            fs::read(fixture.project.join(".cursor/hooks.json")).unwrap(),
            bytes
        );
    }
}

#[test]
fn corrupt_saved_selection_never_falls_back_to_all_hosts() {
    let fixture = Fixture::new("corrupt_selection");
    fixture.run(&["init", "--platform", "cursor"]);
    fixture
        .store()
        .set_meta("hook_platforms", r#"["unknown-host"]"#)
        .unwrap();
    assert!(!fixture.invoke(&["init"]).status.success());
    assert!(!fixture.project.join(".claude/settings.json").exists());
    // An explicit choice repairs the local selection without guessing.
    fixture.run(&["init", "--platform", "cursor"]);
    assert_eq!(
        fixture.store().get_meta("hook_platforms").unwrap().unwrap(),
        r#"["cursor"]"#
    );
}

#[cfg(windows)]
#[test]
fn cursor_windows_command_runs_literal_executable_with_special_characters() {
    use std::io::Write;
    use std::process::Stdio;
    let fixture = Fixture::new("windows_command");
    let binary = fixture.root.join("Arai's $install").join("arai.exe");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_arai"), &binary).unwrap();
    let mut init = Command::new(&binary);
    fixture.isolate(&mut init);
    assert_success(
        &init
            .args(["init", "--platform", "cursor"])
            .output()
            .unwrap(),
    );
    let settings = fixture.read(".cursor/hooks.json");
    let command = settings["hooks"]["preToolUse"][0]["command"]
        .as_str()
        .unwrap();
    assert!(decoded_command(command).contains("Arai''s $install"));
    let (program, arguments) = command.split_once(' ').unwrap();
    let mut hook = Command::new(program);
    fixture.isolate(&mut hook);
    let mut child = hook
        .args(arguments.split_whitespace())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{ invalid hook JSON")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["permission"], "deny");
    let mut init = Command::new(&binary);
    fixture.isolate(&mut init);
    assert_success(
        &init
            .args(["init", "--platform", "cursor"])
            .output()
            .unwrap(),
    );
    assert_eq!(fixture.read(".cursor/hooks.json"), settings);
    fixture.run(&["deinit", "--platform", "cursor"]);
    assert_eq!(
        fixture.read(".cursor/hooks.json")["hooks"]["preToolUse"],
        json!([])
    );
}
