//! Regression: `arai init` with no instruction files must still register
//! host hooks, and `arai add` must refuse inert rules by default.
//!
//! These were the Arai-side half of the Grok Build "enforcement that presents
//! as present" investigation (#173): empty projects never wrote
//! `.grok/hooks/arai.json`, and `arai add "Never run echo …"` printed
//! `Added:` for a rule that can never match.

use base64::Engine;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn arai_bin() -> &'static str {
    env!("CARGO_BIN_EXE_arai")
}

fn fresh_env(label: &str) -> (PathBuf, PathBuf, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "arai_init_hooks_{label}_{}_{}",
        std::process::id(),
        nanos,
    ));
    let project = root.join("project");
    let home = root.join("home");
    fs::create_dir_all(&project).expect("project");
    fs::create_dir_all(project.join(".git")).expect(".git");
    fs::create_dir_all(&home).expect("home");
    (root, project, home)
}

#[test]
fn init_without_instruction_files_registers_hooks() {
    let (root, project, home) = fresh_env("empty");
    let out = Command::new(arai_bin())
        .arg("init")
        .current_dir(&project)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .output()
        .expect("spawn init");
    assert!(
        out.status.success(),
        "init failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("No instruction files found"),
        "expected empty-file notice: {stdout}"
    );

    let claude = project.join(".claude/settings.json");
    let grok = project.join(".grok/hooks/arai.json");
    let codex = project.join(".codex/hooks.json");
    assert!(
        claude.is_file(),
        "missing .claude/settings.json after empty init"
    );
    assert!(
        grok.is_file(),
        "missing .grok/hooks/arai.json after empty init"
    );
    assert!(
        codex.is_file(),
        "missing .codex/hooks.json after empty init"
    );
    let codex_body: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(codex).unwrap()).unwrap();
    let hooks = codex_body["hooks"].as_object().unwrap();
    assert_eq!(hooks.len(), 5);
    for event in [
        "PreToolUse",
        "PostToolUse",
        "UserPromptSubmit",
        "SessionStart",
        "SubagentStart",
    ] {
        assert!(hooks.contains_key(event), "missing Codex event {event}");
    }
    assert!(stdout.contains("/hooks") && stdout.contains("does not grant trust"));

    let grok_body = fs::read_to_string(&grok).expect("read grok hooks");
    let grok_json: serde_json::Value = serde_json::from_str(&grok_body).unwrap();
    let command = grok_json["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    let command = if let Some(encoded) =
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
    };
    assert!(
        command.contains("guardrails --match-stdin --platform grok --hook-event PreToolUse"),
        "hook command missing: {grok_body}"
    );
    assert_eq!(grok_json["hooks"].as_object().unwrap().len(), 4);
    assert!(grok_json["hooks"].get("UserPromptSubmit").is_none());
    assert!(
        grok_body.contains("PreToolUse"),
        "PreToolUse registration missing: {grok_body}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn add_refuses_inert_rule_by_default() {
    let (root, project, home) = fresh_env("inert");
    assert!(Command::new(arai_bin())
        .arg("init")
        .current_dir(&project)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .status()
        .unwrap()
        .success());

    let add = Command::new(arai_bin())
        .args(["add", "Never run echo test161_block_marker"])
        .current_dir(&project)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .output()
        .expect("add");
    assert!(
        !add.status.success(),
        "inert add should fail: stdout={} stderr={}",
        String::from_utf8_lossy(&add.stdout),
        String::from_utf8_lossy(&add.stderr)
    );
    let err = format!(
        "{}{}",
        String::from_utf8_lossy(&add.stdout),
        String::from_utf8_lossy(&add.stderr)
    );
    assert!(
        err.contains("Rule not added") || err.contains("enforceable tool domain"),
        "unexpected error text: {err}"
    );
    assert!(
        !err.lines().any(|l| l.trim_start().starts_with("Added:")),
        "must not print unconditional Added: for inert rule: {err}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn add_allow_inert_keeps_documentary_rule() {
    let (root, project, home) = fresh_env("allow-inert");
    assert!(Command::new(arai_bin())
        .arg("init")
        .current_dir(&project)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .status()
        .unwrap()
        .success());

    let add = Command::new(arai_bin())
        .args([
            "add",
            "--allow-inert",
            "Never run echo test161_block_marker",
        ])
        .current_dir(&project)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .output()
        .expect("add");
    assert!(
        add.status.success(),
        "allow-inert should succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&add.stdout),
        String::from_utf8_lossy(&add.stderr)
    );
    let stdout = String::from_utf8_lossy(&add.stdout);
    assert!(
        stdout.contains("Added") && stdout.to_lowercase().contains("inert"),
        "expected inert label: {stdout}"
    );

    let _ = fs::remove_dir_all(&root);
}
