//! Independent contract tests for host-setup-verification v1 (H1-H6).
use arai::{config::Config, init};
use base64::Engine;
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "arai-host-verifier-{}-{unique}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        assert!(self.0.is_absolute() && self.0.starts_with(std::env::temp_dir()));
        assert!(self
            .0
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("arai-host-verifier-"));
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

struct Fixture {
    _temp: TempDir,
    cfg: Config,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new();
        let project = temp.path().join("project with spaces ' quote $literal");
        fs::create_dir_all(project.join(".git")).unwrap();
        let git = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&project)
            .output()
            .unwrap();
        assert!(
            git.status.success(),
            "{}",
            String::from_utf8_lossy(&git.stderr)
        );
        let mut cfg = Config::load_from(&project).unwrap();
        cfg.home_dir = temp.path().join("home");
        cfg.arai_base_dir = temp.path().join("state");
        fs::create_dir_all(&cfg.home_dir).unwrap();
        Self { _temp: temp, cfg }
    }
    fn path(&self, index: usize) -> std::path::PathBuf {
        self.cfg
            .project_root
            .join(HOSTS[index].2.replace('/', std::path::MAIN_SEPARATOR_STR))
    }
    fn put(&self, index: usize, value: &Value) {
        let path = self.path(index);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }
    fn cli(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_arai"))
            .args(args)
            .current_dir(&self.cfg.project_root)
            .env("HOME", &self.cfg.home_dir)
            .env("USERPROFILE", &self.cfg.home_dir)
            .env("ARAI_BASE_DIR", &self.cfg.arai_base_dir)
            .env_remove("ARAI_DB_DIR")
            .env_remove("ARAI_DISABLED")
            .env_remove("ARAI_DENY_MODE")
            .output()
            .unwrap()
    }
}

fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn success(output: std::process::Output) -> String {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn handler(value: &Value, index: usize, event: &str) -> Value {
    let entries = value["hooks"][event].as_array().unwrap();
    if matches!(index, 0 | 1 | 3) {
        entries.last().unwrap()["hooks"][0].clone()
    } else {
        entries.last().unwrap().clone()
    }
}

fn powershell_script(command: &str) -> String {
    let encoded = command
        .strip_prefix("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ")
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    assert_eq!(bytes.len() % 2, 0);
    String::from_utf16(
        &bytes
            .chunks(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

#[test]
fn h1_fresh_order_write_timing_labels_guidance() {
    // H1: each exact ordered result follows its completed write and precedes the next attempt.
    let fixture = Fixture::new();
    let guidance = [
        "trusted workspace",
        "/hooks",
        "Cursor settings",
        "/hooks-trust",
        "default branch",
    ];
    let mut count = 0;
    init::register_default_hosts(&fixture.cfg, |outcome| {
        assert_eq!(outcome.host, HOSTS[count].0);
        assert_eq!(outcome.registration_label, HOSTS[count].1);
        assert_eq!(outcome.registration_path, fixture.path(count));
        assert_eq!(outcome.state, init::HostRegistrationState::Registered);
        assert_eq!(outcome.diagnostic, None);
        assert!(outcome.trust_guidance.contains(guidance[count]));
        assert!(read(&fixture.path(count))["hooks"].is_object());
        for later in count + 1..5 {
            assert!(
                !fixture.path(later).exists(),
                "later host attempted before callback"
            );
        }
        count += 1;
    })
    .unwrap();
    assert_eq!(count, 5);
}

#[test]
fn h2_optional_grok_original_diagnostic_continues() {
    // H2: optional Grok errors are synchronous skipped outcomes and do not stop Copilot.
    let fixture = Fixture::new();
    fixture.put(3, &json!({"hooks": []}));
    let original = fs::read(fixture.path(3)).unwrap();
    let mut outcomes = Vec::new();
    init::register_default_hosts(&fixture.cfg, |outcome| {
        let index = outcomes.len();
        assert_eq!(outcome.host, HOSTS[index].0);
        if index == 3 {
            assert_eq!(outcome.state, init::HostRegistrationState::Skipped);
            assert_eq!(
                outcome.diagnostic.as_deref(),
                Some("hooks is not an object")
            );
            assert!(!fixture.path(4).exists());
        } else {
            assert_eq!(outcome.state, init::HostRegistrationState::Registered);
            assert_eq!(outcome.diagnostic, None);
            assert!(read(&fixture.path(index))["hooks"].is_object());
        }
        outcomes.push(outcome.clone());
    })
    .unwrap();
    assert_eq!(outcomes.len(), 5);
    assert_eq!(fs::read(fixture.path(3)).unwrap(), original);
    assert!(fixture.path(4).is_file());
}

#[test]
fn h2_no_native_clients_worker() {
    // H2: an empty executable search path does not introduce any native-client registration skip.
    if std::env::var_os("ARAI_HOST_VERIFIER_NO_CLIENTS_WORKER").is_none() {
        return;
    }
    let cfg = Config::load().unwrap();
    let mut count = 0;
    init::register_default_hosts(&cfg, |o| {
        assert_eq!(o.host, HOSTS[count].0);
        assert_eq!(o.state, init::HostRegistrationState::Registered);
        assert!(o.registration_path.is_file());
        count += 1;
    })
    .unwrap();
    assert_eq!(count, 5);
}

#[test]
fn h2_missing_native_clients_still_registers_every_host() {
    // H2: missing native clients do not affect setup; no shell/network/client process is needed.
    let fixture = Fixture::new();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "h2_no_native_clients_worker", "--nocapture"])
        .current_dir(&fixture.cfg.project_root)
        .env("HOME", &fixture.cfg.home_dir)
        .env("USERPROFILE", &fixture.cfg.home_dir)
        .env("ARAI_BASE_DIR", &fixture.cfg.arai_base_dir)
        .env("PATH", "")
        .env("ARAI_HOST_VERIFIER_NO_CLIENTS_WORKER", "1")
        .output()
        .unwrap();
    success(output);
}

#[test]
fn h3_each_mandatory_failure_is_exact_reported_prefix() {
    // H3: every mandatory position terminates after its unchanged failure; prior writes persist.
    for failed in [0, 1, 2, 4] {
        let fixture = Fixture::new();
        fixture.put(failed, &json!({"hooks": []}));
        let original = fs::read(fixture.path(failed)).unwrap();
        let mut outcomes = Vec::new();
        let error = init::register_default_hosts(&fixture.cfg, |outcome| {
            let index = outcomes.len();
            assert_eq!(outcome.host, HOSTS[index].0);
            if index == failed {
                assert_eq!(outcome.state, init::HostRegistrationState::Failed);
                assert_eq!(
                    outcome.diagnostic.as_deref(),
                    Some("hooks is not an object")
                );
            } else {
                assert_eq!(outcome.state, init::HostRegistrationState::Registered);
                assert!(read(&fixture.path(index))["hooks"].is_object());
            }
            for later in failed + 1..5 {
                assert!(!fixture.path(later).exists());
            }
            outcomes.push(outcome.clone());
        })
        .unwrap_err();
        assert_eq!(error, "hooks is not an object");
        assert_eq!(outcomes.len(), failed + 1);
        assert_eq!(
            outcomes.last().unwrap().diagnostic.as_deref(),
            Some(error.as_str())
        );
        assert_eq!(fs::read(fixture.path(failed)).unwrap(), original);
        for earlier in 0..failed {
            assert!(read(&fixture.path(earlier))["hooks"].is_object());
        }
    }
}

#[test]
fn h4_neighbor_ownership_mixed_groups_and_refresh() {
    // H4: only complete recognized Arai invocations refresh; mixed neighbors and unrelated data survive.
    let fixture = Fixture::new();
    let neighbors = vec![
        json!({"type":"command","command":"echo arai guardrails --match-stdin"}),
        json!({"type":"command","command":"arai guardrails --match-stdin; echo extra"}),
        json!({"type":"command","command":"arai guardrails --match-stdin","commandWindows":"echo custom"}),
        json!({"type":"command","command":"echo neighbor","timeout":17}),
    ];
    let mut mixed = neighbors.clone();
    mixed.push(json!({"type":"command","command":"'C:/old/arai.exe' guardrails --match-stdin","timeout":1}));
    for index in 0..5 {
        let event = if matches!(index, 0 | 1 | 3) {
            "PreToolUse"
        } else {
            "preToolUse"
        };
        let entries = if matches!(index, 0 | 1 | 3) {
            json!([{"matcher":"keep-neighbors","metadata":"retain","hooks":mixed}])
        } else {
            json!(mixed)
        };
        fixture.put(index, &json!({"unrelated":{"keep":true}, "hooks":{event:entries,"UnrelatedEvent":[{"custom":true}]}}));
    }
    init::register_default_hosts(&fixture.cfg, |_| {}).unwrap();
    for index in 0..5 {
        let value = read(&fixture.path(index));
        assert_eq!(value["unrelated"], json!({"keep":true}));
        assert_eq!(value["hooks"]["UnrelatedEvent"], json!([{"custom":true}]));
        let event = if matches!(index, 0 | 1 | 3) {
            "PreToolUse"
        } else {
            "preToolUse"
        };
        let entries = value["hooks"][event].as_array().unwrap();
        if matches!(index, 0 | 1 | 3) {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0]["matcher"], "keep-neighbors");
            assert_eq!(entries[0]["metadata"], "retain");
            assert_eq!(entries[0]["hooks"], json!(neighbors));
        } else {
            assert_eq!(entries.len(), neighbors.len() + 1);
            assert_eq!(&entries[..neighbors.len()], neighbors.as_slice());
        }
        let latest = handler(&value, index, event);
        assert!(!latest.to_string().contains("C:/old/arai.exe"));
    }
}

#[test]
fn h4_malformed_target_diagnostics_and_no_rewrite() {
    // H4: malformed root, hooks and target arrays retain precise existing failure semantics.
    for (value, diagnostic) in [
        (json!([]), None),
        (json!({"hooks":false}), Some("hooks is not an object")),
        (
            json!({"hooks":{"PreToolUse":{}}}),
            Some("PreToolUse is not an array"),
        ),
    ] {
        let fixture = Fixture::new();
        fixture.put(0, &value);
        let before = fs::read(fixture.path(0)).unwrap();
        let mut report = Vec::new();
        let error =
            init::register_default_hosts(&fixture.cfg, |o| report.push(o.clone())).unwrap_err();
        let expected = diagnostic
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{} is not an object", fixture.path(0).display()));
        assert_eq!(error, expected);
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].diagnostic.as_deref(), Some(error.as_str()));
        assert_eq!(fs::read(fixture.path(0)).unwrap(), before);
        assert!(!fixture.path(1).exists());
    }
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.path(0).parent().unwrap()).unwrap();
    fs::write(fixture.path(0), b"{bad json").unwrap();
    let error = init::register_default_hosts(&fixture.cfg, |_| {}).unwrap_err();
    assert!(error.starts_with(&format!("Could not parse {}:", fixture.path(0).display())));
    assert_eq!(fs::read(fixture.path(0)).unwrap(), b"{bad json");
}

#[test]
fn h4_native_events_wrappers_timeouts_windows_quoting_idempotency() {
    // H4: all native events/matchers/timeouts/wrappers and semantic repeated-init are retained.
    let fixture = Fixture::new();
    success(fixture.cli(&["init"]));
    let before: Vec<_> = (0..5).map(|i| read(&fixture.path(i))).collect();
    let expected_events: [&[&str]; 5] = [
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
    #[cfg(windows)]
    let exe = Path::new(env!("CARGO_BIN_EXE_arai"))
        .canonicalize()
        .unwrap();
    #[cfg(windows)]
    let native = exe.to_string_lossy().trim_start_matches(r"\?\").to_owned();
    for index in 0..5 {
        let hooks = before[index]["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), expected_events[index].len());
        if matches!(index, 2 | 4) {
            assert_eq!(before[index]["version"], 1);
        }
        for &event in expected_events[index] {
            let h = handler(&before[index], index, event);
            assert_eq!(h["type"], "command");
            let timeout = if event.eq_ignore_ascii_case("sessionstart") {
                10
            } else {
                3
            };
            assert_eq!(
                h[if index == 4 { "timeoutSec" } else { "timeout" }],
                timeout
            );
            if index == 4 {
                assert!(h.get("command").is_none());
                let bash = h["bash"].as_str().unwrap();
                assert!(bash.starts_with("if [ -x '"));
                assert!(bash.ends_with("guardrails --match-stdin; else exit 0; fi"));
                let ps = powershell_script(h["powershell"].as_str().unwrap());
                assert!(ps.contains("Test-Path -LiteralPath $bin"));
                assert!(ps.ends_with("else { exit 0 }"));
            }
            #[cfg(windows)]
            if index < 3 {
                let field = if index == 1 {
                    "commandWindows"
                } else {
                    "command"
                };
                let script = powershell_script(h[field].as_str().unwrap());
                let actual_path = std::env::current_exe()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("arai.exe");
                let expected = format!("$ErrorActionPreference = 'Stop'; & '{}' guardrails --match-stdin; exit $LASTEXITCODE", actual_path.to_string_lossy().replace('\'', "''"));
                assert_eq!(script, expected, "native={native}");
            }
        }
    }
    assert_eq!(
        before[1]["hooks"]["SessionStart"][0]["matcher"],
        "startup|resume"
    );
    assert_eq!(before[2]["hooks"]["preToolUse"][0]["failClosed"], true);
    assert_eq!(before[2]["hooks"]["postToolUse"][0]["matcher"], "Shell");
    assert_eq!(before[3]["hooks"]["SessionStart"][0]["matcher"], "");
    success(fixture.cli(&["init"]));
    for (index, expected) in before.iter().enumerate() {
        assert_eq!(read(&fixture.path(index)), *expected);
    }
}

#[test]
#[cfg(windows)]
fn h4_encoded_executable_path_with_shell_metacharacters() {
    // H4: Windows quoting preserves literal spaces, apostrophes, dollar signs and backticks.
    let fixture = Fixture::new();
    let install = fixture
        ._temp
        .path()
        .join("installed ' $literal `literal & space");
    fs::create_dir(&install).unwrap();
    let binary = install.join("arai.exe");
    fs::copy(env!("CARGO_BIN_EXE_arai"), &binary).unwrap();
    success(
        Command::new(&binary)
            .args(["init"])
            .current_dir(&fixture.cfg.project_root)
            .env("HOME", &fixture.cfg.home_dir)
            .env("USERPROFILE", &fixture.cfg.home_dir)
            .env("ARAI_BASE_DIR", &fixture.cfg.arai_base_dir)
            .output()
            .unwrap(),
    );
    let expected = format!(
        "$ErrorActionPreference = 'Stop'; & '{}' guardrails --match-stdin; exit $LASTEXITCODE",
        binary.to_string_lossy().replace('\'', "''")
    );
    for index in 0..3 {
        let event = if index == 2 {
            "preToolUse"
        } else {
            "PreToolUse"
        };
        let h = handler(&read(&fixture.path(index)), index, event);
        let field = if index == 1 {
            "commandWindows"
        } else {
            "command"
        };
        assert_eq!(powershell_script(h[field].as_str().unwrap()), expected);
    }
    let h = handler(&read(&fixture.path(4)), 4, "preToolUse");
    assert!(
        powershell_script(h["powershell"].as_str().unwrap()).contains(&format!(
            "$bin = '{}'",
            binary.to_string_lossy().replace('\'', "''")
        ))
    );
    assert!(h["bash"].as_str().unwrap().contains("'\\''"));
}

#[test]
fn h5_ensure_hooks_worker() {
    // H5: direct ensure_hooks keeps strict Grok propagation, independently of optional setup reporting.
    if std::env::var_os("ARAI_HOST_VERIFIER_ENSURE_WORKER").is_none() {
        return;
    }
    assert_eq!(init::ensure_hooks().unwrap_err(), "hooks is not an object");
    let root = std::env::current_dir().unwrap();
    for (_, _, path) in &HOSTS[..3] {
        assert!(read(&root.join(path))["hooks"].is_object());
    }
    assert!(!root.join(HOSTS[4].2).exists());
}

#[test]
fn h5_strict_ensure_and_add_warning() {
    // H5: Grok remains fatal for automatic ensure_hooks, while add prints its warning and keeps the rule.
    let fixture = Fixture::new();
    fixture.put(3, &json!({"hooks":[]}));
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "h5_ensure_hooks_worker", "--nocapture"])
        .current_dir(&fixture.cfg.project_root)
        .env("HOME", &fixture.cfg.home_dir)
        .env("USERPROFILE", &fixture.cfg.home_dir)
        .env("ARAI_BASE_DIR", &fixture.cfg.arai_base_dir)
        .env("ARAI_HOST_VERIFIER_ENSURE_WORKER", "1")
        .output()
        .unwrap();
    success(output);
    let text = success(fixture.cli(&["add", "Never run git push --force"]));
    assert!(text.contains("hooks is not an object"), "{text}");
    assert!(text.to_ascii_lowercase().contains("warn"), "{text}");
    assert!(!fixture.path(4).exists());
}

#[test]
fn h5_deinit_only_owned_entries_and_eligible_empty_files() {
    // H5: shared files remain; owned Grok/Copilot files disappear only when semantically empty.
    for retain_neighbors in [false, true] {
        let fixture = Fixture::new();
        success(fixture.cli(&["init"]));
        if retain_neighbors {
            for index in 0..5 {
                let mut value = read(&fixture.path(index));
                value["keep"] = json!("neighbor metadata");
                value["hooks"]["UnrelatedEvent"] = json!([{"command":"echo neighbor"}]);
                fixture.put(index, &value);
            }
        }
        success(fixture.cli(&["deinit"]));
        for index in 0..5 {
            if index >= 3 && !retain_neighbors {
                assert!(!fixture.path(index).exists());
            } else {
                let value = read(&fixture.path(index));
                let hooks = value["hooks"].as_object().unwrap();
                for (event, entries) in hooks {
                    if event == "UnrelatedEvent" {
                        assert_eq!(*entries, json!([{"command":"echo neighbor"}]));
                    } else {
                        assert!(entries.as_array().unwrap().is_empty());
                    }
                }
                if retain_neighbors {
                    assert_eq!(value["keep"], "neighbor metadata");
                }
            }
        }
    }
}

#[test]
fn h6_direct_leaf_surface_compiles_before_composition() {
    // H6: the contract callable and canonical child-owned output types compile directly.
    let fixture = Fixture::new();
    let mut observations: Vec<init::HostRegistrationOutcome> = Vec::new();
    let completion: Result<(), String> =
        init::register_default_hosts(&fixture.cfg, |o| observations.push(o.clone()));
    assert!(completion.is_ok());
    assert_eq!(observations.len(), 5);
    // Registration proves only completed project writes; trust and host execution remain unknown.
    assert!(observations
        .iter()
        .all(|o| o.state == init::HostRegistrationState::Registered));
}
