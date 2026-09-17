//! Registration refresh/removal must preserve user hooks and work with Git's
//! configured paths. Child-local environments keep parallel tests isolated.
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
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
            "arai_registration_{label}_{}_{}",
            std::process::id(),
            nanos
        ));
        let fixture = Self {
            project: root.join("project"),
            home: root.join("home"),
            root,
        };
        fs::create_dir_all(&fixture.project).unwrap();
        fs::create_dir_all(&fixture.home).unwrap();
        fixture.git(&fixture.project, &["init"]);
        fixture
    }

    fn isolate(&self, command: &mut Command, project: &Path) {
        command
            .current_dir(project)
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
            .env_remove("CLAUDE_PROJECT_DIR")
            .env_remove("CLAUDE_PLUGIN_ROOT");
    }

    fn arai(&self, project: &Path, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_arai"));
        self.isolate(&mut command, project);
        command.args(args).output().unwrap()
    }

    fn run(&self, args: &[&str]) -> Output {
        let output = self.arai(&self.project, args);
        assert_success(&output);
        output
    }

    fn git(&self, project: &Path, args: &[&str]) -> Output {
        let mut command = Command::new("git");
        self.isolate(&mut command, project);
        let output = command.args(args).output().unwrap();
        assert_success(&output);
        output
    }

    fn write_json(&self, relative: &str, value: &Value) {
        let path = self.project.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
    }

    fn read_json(&self, relative: &str) -> Value {
        serde_json::from_str(&fs::read_to_string(self.project.join(relative)).unwrap()).unwrap()
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

fn commands(value: &Value, event: &str) -> Vec<String> {
    value["hooks"][event]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["hooks"].as_array().unwrap())
        .filter_map(|handler| handler["command"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn refresh_migrates_old_paths_without_removing_neighbours() {
    let fixture = Fixture::new("refresh");
    let other = json!({"type":"command", "command":"echo arai audit", "timeout":45});
    let named_path = json!({"type":"command", "command":"/tools/arai-audit/run.sh"});
    let custom_windows = json!({"type":"command", "command":"arai guardrails --match-stdin",
        "commandWindows":"custom-policy.exe"});
    let initial = json!({"description":"keep me", "hooks":{
        "PreToolUse":[{"matcher":"Bash", "hooks":[
            {"type":"command", "command":"\"/old install/arai\" guardrails --match-stdin"},
            other, named_path, custom_windows
        ]}, {"hooks":[{"type":"command", "command":"arai guardrails --match-stdin"}]}],
        "Stop":[{"hooks":[{"type":"command", "command":"notify finished"}]}]
    }});
    for path in [
        ".claude/settings.json",
        ".codex/hooks.json",
        ".grok/hooks/arai.json",
    ] {
        fixture.write_json(path, &initial);
    }
    fixture.run(&["init"]);
    let mut first = Vec::new();
    for path in [
        ".claude/settings.json",
        ".codex/hooks.json",
        ".grok/hooks/arai.json",
    ] {
        let body = fixture.read_json(path);
        assert_eq!(body["description"], "keep me");
        assert_eq!(body["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        assert_eq!(
            body["hooks"]["PreToolUse"][0]["hooks"],
            json!([other, named_path, custom_windows])
        );
        let hooks = commands(&body, "PreToolUse");
        assert_eq!(hooks.len(), 4);
        assert!(!hooks.iter().any(|command| command.contains("old install")));
        assert_eq!(commands(&body, "Stop"), ["notify finished"]);
        assert!(hooks
            .last()
            .unwrap()
            .contains(&env!("CARGO_BIN_EXE_arai").replace('\\', "/")));
        first.push(body);
    }
    fixture.run(&["init"]);
    for (index, path) in [
        ".claude/settings.json",
        ".codex/hooks.json",
        ".grok/hooks/arai.json",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(
            fixture.read_json(path),
            first[index],
            "refresh changed {path}"
        );
    }
    fixture.run(&["deinit"]);
    for path in [
        ".claude/settings.json",
        ".codex/hooks.json",
        ".grok/hooks/arai.json",
    ] {
        let body = fixture.read_json(path);
        assert_eq!(
            body["hooks"]["PreToolUse"][0]["hooks"],
            json!([other, named_path, custom_windows])
        );
        assert_eq!(commands(&body, "Stop"), ["notify finished"]);
        assert_eq!(body["description"], "keep me");
    }
}

#[test]
fn pre_commit_reinstalls_and_deinit_works_without_claude_settings() {
    let fixture = Fixture::new("reinstall");
    fixture.run(&["init", "--pre-commit"]);
    let hook = fixture.project.join(".git/hooks/pre-commit");
    let original = fs::read_to_string(&hook).unwrap();
    assert!(original.contains("# arai-managed-pre-commit: v1"));
    fixture.run(&["init", "--pre-commit"]);
    assert_eq!(fs::read_to_string(&hook).unwrap(), original);
    fs::remove_file(fixture.project.join(".claude/settings.json")).unwrap();
    fixture.run(&["deinit"]);
    assert!(!hook.exists());
    assert!(!fixture.project.join(".grok/hooks/arai.json").exists());
    assert!(commands(&fixture.read_json(".codex/hooks.json"), "PreToolUse").is_empty());
}

#[test]
fn pre_commit_preserves_custom_script_until_forced() {
    let fixture = Fixture::new("custom");
    let path = fixture.project.join(".git/hooks/pre-commit");
    let custom = "#!/bin/sh\n# mention arai check-diff but this is mine\necho lint\n";
    fs::write(&path, custom).unwrap();
    let output = fixture.arai(&fixture.project, &["init", "--pre-commit"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--force"));
    assert_eq!(fs::read_to_string(&path).unwrap(), custom);
    fixture.run(&["deinit"]);
    assert_eq!(fs::read_to_string(&path).unwrap(), custom);
    fixture.run(&["init", "--pre-commit", "--force"]);
    let mut managed = fs::read_to_string(&path).unwrap();
    managed.push_str("echo custom follow-up\n");
    fs::write(&path, &managed).unwrap();
    fixture.run(&["deinit"]);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        managed,
        "deinit must preserve user additions even when the ownership marker remains"
    );
}

#[test]
fn legacy_pre_commit_is_refreshed_and_removed() {
    let fixture = Fixture::new("legacy");
    let path = fixture.project.join(".git/hooks/pre-commit");
    let legacy = "#!/bin/sh\n# Installed by `arai init --pre-commit`. Evaluates the staged diff\n# against Arai guardrails. Bypass with `git commit --no-verify`.\nexec '/old install/arai' check-diff --cached\n";
    fs::write(&path, legacy).unwrap();
    fixture.run(&["init", "--pre-commit"]);
    assert!(fs::read_to_string(&path)
        .unwrap()
        .contains("# arai-managed-pre-commit: v1"));
    fs::write(&path, legacy).unwrap();
    fixture.run(&["deinit"]);
    assert!(!path.exists());
}

#[test]
fn pre_commit_honours_relative_and_absolute_hooks_path() {
    for absolute in [false, true] {
        let fixture = Fixture::new("hooks_path");
        let hooks = if absolute {
            fixture.root.join("shared hooks")
        } else {
            fixture.project.join("custom-hooks")
        };
        let configured = if absolute {
            hooks.to_string_lossy().into_owned()
        } else {
            "custom-hooks".to_string()
        };
        fixture.git(&fixture.project, &["config", "core.hooksPath", &configured]);
        fixture.run(&["init", "--pre-commit"]);
        assert!(hooks.join("pre-commit").is_file());
        assert!(!fixture.project.join(".git/hooks/pre-commit").exists());
        fixture.run(&["deinit"]);
        assert!(!hooks.join("pre-commit").exists());
    }
}

#[test]
fn pre_commit_honours_worktrees() {
    let fixture = Fixture::new("worktree");
    fixture.git(
        &fixture.project,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "--no-verify",
            "-m",
            "fixture",
        ],
    );
    let worktree = fixture.root.join("linked worktree");
    fixture.git(
        &fixture.project,
        &["worktree", "add", "--detach", worktree.to_str().unwrap()],
    );
    assert!(worktree.join(".git").is_file());
    assert_success(&fixture.arai(&worktree, &["init", "--pre-commit"]));
    let hook = fixture.project.join(".git/hooks/pre-commit");
    assert!(hook.is_file());
    assert_success(&fixture.arai(&worktree, &["deinit"]));
    assert!(!hook.exists());
}

#[test]
fn malformed_codex_config_is_not_overwritten() {
    let fixture = Fixture::new("malformed");
    let path = fixture.project.join(".codex/hooks.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "{ invalid JSON").unwrap();
    let output = fixture.arai(&fixture.project, &["init"]);
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(path).unwrap(), "{ invalid JSON");
}

#[cfg(windows)]
#[test]
fn codex_windows_command_runs_from_path_with_spaces_and_metacharacters() {
    use std::io::Write;
    use std::process::Stdio;
    let fixture = Fixture::new("windows_command");
    let binary = fixture.root.join("Arai's $install").join("arai.exe");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_arai"), &binary).unwrap();
    let mut init = Command::new(&binary);
    fixture.isolate(&mut init, &fixture.project);
    assert_success(&init.arg("init").output().unwrap());
    let mut add = Command::new(&binary);
    fixture.isolate(&mut add, &fixture.project);
    assert_success(&add.args(["add", "Never run cargo clean"]).output().unwrap());
    let settings = fixture.read_json(".codex/hooks.json");
    let command = settings["hooks"]["PreToolUse"][0]["hooks"][0]["commandWindows"]
        .as_str()
        .unwrap();
    let (program, arguments) = command.split_once(' ').unwrap();
    let run = |payload: &[u8]| {
        let mut hook = Command::new(program);
        fixture.isolate(&mut hook, &fixture.project);
        let mut child = hook
            .args(arguments.split_whitespace())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload).unwrap();
        let output = child.wait_with_output().unwrap();
        assert_success(&output);
        output.stdout
    };
    // A malformed deny alone cannot prove stdin was forwarded (an empty
    // stdin denies the same way), so also assert a rule-based deny with
    // its reason and a clean allow for a harmless command.
    let stdout = run(b"{\"hook_event_name\":\"PreToolUse\",\"tool_input\":{");
    let response: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(response["hookSpecificOutput"]["permissionDecision"], "deny");
    let stdout = run(
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo clean"}}"#,
    );
    let response: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(response["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(response["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap()
        .contains("cargo clean"));
    let stdout = run(
        br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo check"}}"#,
    );
    assert!(stdout.is_empty(), "{}", String::from_utf8_lossy(&stdout));
    // Refresh and remove encoded Windows handlers as well as their POSIX side.
    let mut init = Command::new(&binary);
    fixture.isolate(&mut init, &fixture.project);
    assert_success(&init.arg("init").output().unwrap());
    assert_eq!(fixture.read_json(".codex/hooks.json"), settings);
    fixture.run(&["deinit"]);
    assert!(commands(&fixture.read_json(".codex/hooks.json"), "PreToolUse").is_empty());
}

/// Cursor's hooks.json is a flat handler array per camelCase event under a
/// top-level `version`.  Registration must preserve neighbours, set
/// `failClosed` on the decision event only, be idempotent, and `deinit` must
/// leave the user's handlers alone.  Codex and Grok additionally gain a
/// SessionStart registration (Codex filtered to startup|resume).
#[test]
fn cursor_and_session_start_registrations() {
    let fixture = Fixture::new("cursor");
    let other = json!({"command": "./hooks/other.sh", "matcher": "Shell"});
    let format = json!({"command": "./format.sh"});
    fixture.write_json(
        ".cursor/hooks.json",
        &json!({"version": 1, "hooks": {
            "preToolUse": [other, {"type": "command",
                "command": "\"/old install/arai\" guardrails --match-stdin"}],
            "afterFileEdit": [format],
            "stop": [{"command": "./audit.sh", "loop_limit": 10}]
        }}),
    );
    fixture.run(&["init"]);
    let cursor = fixture.read_json(".cursor/hooks.json");
    assert_eq!(cursor["version"], 1);
    let exe = env!("CARGO_BIN_EXE_arai").replace('\\', "/");
    let pre = cursor["hooks"]["preToolUse"].as_array().unwrap();
    assert_eq!(pre.len(), 2, "{pre:?}");
    assert_eq!(pre[0], other);
    let command = pre[1]["command"].as_str().unwrap();
    if cfg!(windows) {
        // Cursor has no commandWindows field: the command itself is the
        // shell-agnostic PowerShell invocation.
        assert!(command.starts_with("powershell.exe"), "{command}");
    } else {
        assert!(command.contains(&exe), "{command}");
    }
    assert_eq!(pre[1]["failClosed"], true);
    assert!(pre[1].get("matcher").is_none());
    assert_eq!(pre[1]["type"], "command");
    let post = cursor["hooks"]["postToolUse"].as_array().unwrap();
    assert_eq!(post.len(), 1);
    assert_eq!(post[0]["matcher"], "Shell");
    assert!(post[0].get("failClosed").is_none());
    let edit = cursor["hooks"]["afterFileEdit"].as_array().unwrap();
    assert_eq!(edit[0], format);
    assert_eq!(edit.len(), 2);
    assert_eq!(cursor["hooks"]["sessionStart"].as_array().unwrap().len(), 1);
    assert_eq!(cursor["hooks"]["stop"][0]["command"], "./audit.sh");

    let codex = fixture.read_json(".codex/hooks.json");
    assert_eq!(
        codex["hooks"]["SessionStart"][0]["matcher"],
        "startup|resume"
    );
    assert_eq!(commands(&codex, "SessionStart").len(), 1);
    let grok = fixture.read_json(".grok/hooks/arai.json");
    assert_eq!(grok["hooks"]["SessionStart"][0]["matcher"], "");
    assert_eq!(commands(&grok, "SessionStart").len(), 1);

    fixture.run(&["init"]);
    assert_eq!(fixture.read_json(".cursor/hooks.json"), cursor);

    fixture.run(&["deinit"]);
    let cursor = fixture.read_json(".cursor/hooks.json");
    assert_eq!(cursor["hooks"]["preToolUse"], json!([other]));
    assert_eq!(cursor["hooks"]["afterFileEdit"], json!([format]));
    assert_eq!(cursor["hooks"]["postToolUse"], json!([]));
    assert_eq!(cursor["hooks"]["stop"][0]["command"], "./audit.sh");
    assert!(commands(&fixture.read_json(".codex/hooks.json"), "SessionStart").is_empty());

    // .cursor/hooks.json is a shared settings file like .claude/settings.json:
    // deinit rewrites it and never deletes it, even when only empty arrays
    // remain, so a user's placeholder file cannot vanish from git status.
    let fresh = Fixture::new("cursor_fresh");
    fresh.run(&["init"]);
    assert!(fresh.project.join(".cursor/hooks.json").is_file());
    fresh.run(&["deinit"]);
    let left = fresh.read_json(".cursor/hooks.json");
    assert_eq!(left["version"], 1);
    assert!(left["hooks"]
        .as_object()
        .unwrap()
        .values()
        .all(|entries| entries.as_array().is_some_and(Vec::is_empty)));
}
