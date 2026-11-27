use arai::{config::Config, init};
use base64::Engine;
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};
struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> std::io::Result<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "arai-host-recheck-{}-{now}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const HOSTS: [(&str, &str, &str); 5] = [
    ("claude", "Claude", ".claude/settings.json"),
    ("codex", "Codex", ".codex/hooks.json"),
    ("cursor", "Cursor", ".cursor/hooks.json"),
    ("grok", "Grok", ".grok/hooks/arai.json"),
    ("copilot", "Copilot", ".github/hooks/arai.json"),
];

// Execute this public-API test from a real arai.exe basename, in an isolated process.
// Ownership recognition intentionally rejects arbitrary non-arai executables.
fn worker(name: &str) -> bool {
    if std::env::var("ARAI_HOST_RECHECK_WORKER").as_deref() == Ok(name) {
        return true;
    }
    let dir = TempDir::new().unwrap();
    let bin_dir = dir.path().join("space ' dollar$ tick` amp&");
    fs::create_dir(&bin_dir).unwrap();
    let bin = bin_dir.join(if cfg!(windows) { "arai.exe" } else { "arai" });
    fs::copy(std::env::current_exe().unwrap(), &bin).unwrap();
    let output = Command::new(&bin)
        .args(["--exact", name, "--nocapture"])
        .env("ARAI_HOST_RECHECK_WORKER", name)
        .env("ARAI_BASE_DIR", dir.path().join("state"))
        .env("HOME", dir.path())
        .env("USERPROFILE", dir.path())
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

fn fixture() -> (TempDir, Config) {
    let dir = TempDir::new().unwrap();
    let cfg = Config::load_from(dir.path()).unwrap();
    (dir, cfg)
}
fn write(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}
fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn ps_script(value: &Value) -> String {
    let encoded = value
        .as_str()
        .unwrap()
        .strip_prefix("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ")
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    assert_eq!(bytes.len() % 2, 0);
    let words: Vec<u16> = bytes
        .chunks(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    String::from_utf16(&words).unwrap()
}

#[test]
fn h1_synchronous_exact_outcomes() {
    // H1: exact identities, labels, paths and guidance follow completed writes, before later attempts.
    if !worker("h1_synchronous_exact_outcomes") {
        return;
    }
    let (_dir, cfg) = fixture();
    let mut n = 0;
    init::register_default_hosts(&cfg, |outcome| {
        let (host, label, path) = HOSTS[n];
        assert_eq!(outcome.host, host);
        assert_eq!(outcome.registration_label, label);
        assert_eq!(outcome.registration_path, cfg.project_root.join(path));
        assert_eq!(outcome.state, init::HostRegistrationState::Registered);
        assert_eq!(outcome.diagnostic, None);
        assert!(read(&outcome.registration_path)["hooks"].is_object());
        for (_, _, later) in &HOSTS[n + 1..] {
            assert!(!cfg.project_root.join(later).exists());
        }
        let markers: &[&str] = match host {
            "claude" => &["trusted workspace", "trust prompt"],
            "codex" => &["/hooks", "does not grant trust"],
            "cursor" => &["Hooks tab", "policy"],
            "grok" => &["/hooks-trust", "--trust", "inactive"],
            _ => &[
                "default branch",
                "missing arai binary exits 0",
                "twice",
                "does not grant workspace trust",
            ],
        };
        for marker in markers {
            assert!(outcome.trust_guidance.contains(marker));
        }
        n += 1;
    })
    .unwrap();
    assert_eq!(n, 5);
}

#[test]
fn h2_optional_error_preserves_diagnostic_and_continues() {
    // H2: Grok's original failure is skipped; no installed client is required for other writes.
    if !worker("h2_optional_error_preserves_diagnostic_and_continues") {
        return;
    }
    let (_dir, cfg) = fixture();
    let path = cfg.project_root.join(HOSTS[3].2);
    write(&path, &json!({"hooks": []}));
    let mut outcomes = Vec::new();
    init::register_default_hosts(&cfg, |o| outcomes.push(o.clone())).unwrap();
    assert_eq!(outcomes.len(), 5);
    for (i, o) in outcomes.iter().enumerate() {
        assert_eq!(o.host, HOSTS[i].0);
        assert_eq!(o.registration_label, HOSTS[i].1);
        assert_eq!(o.registration_path, cfg.project_root.join(HOSTS[i].2));
        if i == 3 {
            assert_eq!(o.state, init::HostRegistrationState::Skipped);
            assert_eq!(o.diagnostic.as_deref(), Some("hooks is not an object"));
        } else {
            assert_eq!(o.state, init::HostRegistrationState::Registered);
            assert!(o.diagnostic.is_none());
        }
    }
    assert!(cfg.project_root.join(HOSTS[4].2).exists());
    assert_eq!(read(&path), json!({"hooks": []}));
}

#[test]
fn h3_all_mandatory_failures_stop_at_reported_prefix() {
    // H3: every mandatory error returns unchanged after reporting, with earlier writes intact.
    if !worker("h3_all_mandatory_failures_stop_at_reported_prefix") {
        return;
    }
    for failed in [0, 1, 2, 4] {
        let (_dir, cfg) = fixture();
        let bad = cfg.project_root.join(HOSTS[failed].2);
        write(
            &bad,
            &json!({"hooks": {"PreToolUse": false, "preToolUse": false}}),
        );
        let expected = if failed < 2 {
            "PreToolUse is not an array"
        } else {
            "preToolUse is not an array"
        };
        let mut n = 0;
        let error = init::register_default_hosts(&cfg, |o| {
            assert_eq!(o.host, HOSTS[n].0);
            assert_eq!(o.registration_label, HOSTS[n].1);
            assert_eq!(o.registration_path, cfg.project_root.join(HOSTS[n].2));
            if n == failed {
                assert_eq!(o.state, init::HostRegistrationState::Failed);
                assert_eq!(o.diagnostic.as_deref(), Some(expected));
            } else {
                assert_eq!(o.state, init::HostRegistrationState::Registered);
                assert!(o.diagnostic.is_none());
                assert!(cfg.project_root.join(HOSTS[n].2).exists());
            }
            n += 1;
        })
        .unwrap_err();
        assert_eq!(error, expected);
        assert_eq!(n, failed + 1);
        for (_, _, path) in &HOSTS[failed + 1..] {
            assert!(!cfg.project_root.join(path).exists());
        }
        assert_eq!(
            read(&bad),
            json!({"hooks": {"PreToolUse": false, "preToolUse": false}})
        );
    }
}

#[test]
fn h4_ownership_refresh_native_shapes_and_idempotence() {
    // H4: preserve mixed and unrelated entries while refreshing only actual owned invocations.
    if !worker("h4_ownership_refresh_native_shapes_and_idempotence") {
        return;
    }
    let (_dir, cfg) = fixture();
    let neighbor = json!({"type":"command", "command":"echo arai guardrails --match-stdin"});
    let extra = json!({"type":"command", "command":"arai guardrails --match-stdin; echo extra"});
    let alternate = json!({"type":"command", "command":"arai guardrails --match-stdin", "commandWindows":"custom unrelated"});
    for (i, (_, _, path)) in HOSTS.iter().enumerate() {
        let event = if i == 2 || i == 4 {
            "preToolUse"
        } else {
            "PreToolUse"
        };
        let entries = if i == 2 || i == 4 {
            json!([{"type":"command", "command":"'/old/arai' guardrails --match-stdin"}, neighbor, extra, alternate])
        } else {
            json!([{"matcher":"old", "hooks":[{"type":"command", "command":"'/old/arai' guardrails --match-stdin"}, neighbor, extra, alternate]}])
        };
        let mut value = json!({"custom":"keep", "hooks":{"Unrelated":[{"neighbor":true}]}});
        value["hooks"][event] = entries;
        write(&cfg.project_root.join(path), &value);
    }
    init::register_default_hosts(&cfg, |_| {}).unwrap();
    let mut first = Vec::new();
    let event_lists: [&[&str]; 5] = [
        &[
            "PreToolUse",
            "PostToolUse",
            "UserPromptSubmit",
            "FileChanged",
            "InstructionsLoaded",
            "CwdChanged",
            "PostToolBatch",
            "PermissionDenied",
        ],
        &[
            "PreToolUse",
            "PostToolUse",
            "UserPromptSubmit",
            "SessionStart",
        ],
        &["preToolUse", "postToolUse", "afterFileEdit", "sessionStart"],
        &[
            "PreToolUse",
            "PostToolUse",
            "UserPromptSubmit",
            "SessionStart",
        ],
        &["preToolUse", "postToolUse", "sessionStart"],
    ];
    for (i, (_, _, path)) in HOSTS.iter().enumerate() {
        let value = read(&cfg.project_root.join(path));
        assert_eq!(value["custom"], "keep");
        assert_eq!(value["hooks"]["Unrelated"], json!([{"neighbor":true}]));
        for event in event_lists[i] {
            let entries = value["hooks"][event].as_array().unwrap();
            let last = entries.last().unwrap();
            let handler = if i == 2 || i == 4 {
                last
            } else {
                &last["hooks"][0]
            };
            let timeout_key = if i == 4 { "timeoutSec" } else { "timeout" };
            assert_eq!(
                handler[timeout_key],
                if event.eq_ignore_ascii_case("sessionstart") {
                    10
                } else {
                    3
                }
            );
            if i != 2 && i != 4 {
                assert_eq!(
                    last["matcher"],
                    if i == 1 && *event == "SessionStart" {
                        "startup|resume"
                    } else {
                        ""
                    }
                );
            }
            if i == 4 {
                assert!(handler.get("command").is_none());
                assert!(handler["bash"]
                    .as_str()
                    .unwrap()
                    .ends_with("guardrails --match-stdin; else exit 0; fi"));
                assert!(ps_script(&handler["powershell"]).contains("else { exit 0 }"));
            }
            #[cfg(windows)]
            if i == 0 || i == 1 || i == 2 {
                let key = if i == 1 { "commandWindows" } else { "command" };
                let script = ps_script(&handler[key]);
                let native = std::env::current_exe()
                    .unwrap()
                    .to_string_lossy()
                    .replace('\'', "''");
                assert_eq!(script, format!("$ErrorActionPreference = 'Stop'; & '{native}' guardrails --match-stdin; exit $LASTEXITCODE"));
            }
        }
        if i == 2 {
            assert_eq!(value["version"], 1);
            assert_eq!(
                value["hooks"]["preToolUse"]
                    .as_array()
                    .unwrap()
                    .last()
                    .unwrap()["failClosed"],
                true
            );
            assert_eq!(value["hooks"]["postToolUse"][0]["matcher"], "Shell");
        }
        let event = event_lists[i][0];
        let entries = value["hooks"][event].as_array().unwrap();
        let remaining = if i == 2 || i == 4 {
            entries.clone()
        } else {
            entries[0]["hooks"].as_array().unwrap().clone()
        };
        for kept in [&neighbor, &extra, &alternate] {
            assert!(remaining.contains(kept));
        }
        assert!(!serde_json::to_string(&value).unwrap().contains("/old/arai"));
        first.push(value);
    }
    init::register_default_hosts(&cfg, |_| {}).unwrap();
    for (i, (_, _, path)) in HOSTS.iter().enumerate() {
        assert_eq!(read(&cfg.project_root.join(path)), first[i]);
    }
    for bad in [
        "{",
        "[]",
        "{\"hooks\":[]}",
        "{\"hooks\":{\"PreToolUse\":{}}}",
    ] {
        let (_dir, cfg) = fixture();
        let path = cfg.project_root.join(HOSTS[0].2);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bad).unwrap();
        assert!(init::register_default_hosts(&cfg, |_| {}).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), bad);
    }
}

#[test]
fn h5_ensure_add_warning_and_owned_deinit() {
    // H5: ensure fails even for Grok, add warns, deinit preserves neighbors/shared files.
    if !worker("h5_ensure_add_warning_and_owned_deinit") {
        return;
    }
    let (dir, cfg) = fixture();
    std::env::set_current_dir(dir.path()).unwrap();
    let grok = cfg.project_root.join(HOSTS[3].2);
    write(&grok, &json!({"hooks": []}));
    assert_eq!(init::ensure_hooks().unwrap_err(), "hooks is not an object");
    assert!(!cfg.project_root.join(HOSTS[4].2).exists());
    let output = Command::new(env!("CARGO_BIN_EXE_arai"))
        .args(["add", "--allow-inert", "Never delete the protected fixture"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.to_lowercase().contains("warn"), "{text}");
    assert!(text.contains("hooks is not an object"), "{text}");
    fs::remove_file(&grok).unwrap();
    init::register_default_hosts(&cfg, |_| {}).unwrap();
    let copilot = cfg.project_root.join(HOSTS[4].2);
    let mut custom = read(&copilot);
    custom["description"] = json!("neighbor metadata");
    custom["hooks"]["preToolUse"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"command", "bash":"echo neighbor"}));
    write(&copilot, &custom);
    init::deinit().unwrap();
    for (_, _, path) in &HOSTS[..3] {
        assert!(cfg.project_root.join(path).exists());
    }
    assert!(!grok.exists());
    let remaining = read(&copilot);
    assert_eq!(remaining["description"], "neighbor metadata");
    assert_eq!(
        remaining["hooks"]["preToolUse"],
        json!([{"type":"command", "bash":"echo neighbor"}])
    );
    assert_eq!(remaining["hooks"]["postToolUse"], json!([]));
    assert_eq!(remaining["hooks"]["sessionStart"], json!([]));
    fs::remove_file(&copilot).unwrap();
    init::register_default_hosts(&cfg, |_| {}).unwrap();
    init::deinit().unwrap();
    assert!(!grok.exists());
    assert!(!copilot.exists());
}

#[test]
fn h6_leaf_public_surface_requires_no_parent_caller() {
    // H6: compile and invoke the leaf independently; evidence is registration, never live trust.
    if !worker("h6_leaf_public_surface_requires_no_parent_caller") {
        return;
    }
    let (_dir, cfg) = fixture();
    let mut outcomes: Vec<init::HostRegistrationOutcome> = Vec::new();
    init::register_default_hosts(&cfg, |o| outcomes.push(o.clone())).unwrap();
    assert_eq!(outcomes.len(), 5);
    for o in outcomes {
        assert_eq!(o.state, init::HostRegistrationState::Registered);
        assert!(!o.trust_guidance.is_empty());
    }
}
