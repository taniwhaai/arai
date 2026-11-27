//! Project-file registration evidence only; these tests do not launch native hosts.
use arai::config::Config;
use arai::init::{register_default_hosts, HostRegistrationOutcome, HostRegistrationState};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const HOSTS: [(&str, &str, &str, &str); 5] = [
    (
        "claude",
        "Claude",
        ".claude/settings.json",
        "Claude Code runs project hooks only in a trusted workspace; accept the trust prompt for this folder or the registration stays inactive.",
    ),
    (
        "codex",
        "Codex",
        ".codex/hooks.json",
        "Codex requires a trusted project plus review of each new or changed hook definition in /hooks. Registration does not grant trust. User-level hooks also require review; managed policy may restrict hooks.",
    ),
    (
        "cursor",
        "Cursor",
        ".cursor/hooks.json",
        "Cursor loads project hooks from .cursor/hooks.json; check the Hooks tab in Cursor settings if a managed or team policy restricts them.",
    ),
    (
        "grok",
        "Grok",
        ".grok/hooks/arai.json",
        "Grok Build gates project hooks behind trust: run /hooks-trust in the Grok session (or launch with --trust) the first time, or Arai's hooks stay inactive on the native path.",
    ),
    (
        "copilot",
        "Copilot",
        ".github/hooks/arai.json",
        "Registration does not grant workspace trust. The cloud agent loads only .github/hooks from the default branch; a missing arai binary exits 0 and does not enforce. The Copilot CLI may also run the Claude registration, so one tool call can be evaluated twice.",
    ),
];

struct Fixture {
    root: PathBuf,
    cfg: Config,
}

impl Fixture {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "arai_card_a_hosts_{}_{}",
            std::process::id(),
            nanos
        ));
        let project = root.join("project");
        fs::create_dir_all(project.join(".git")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        let mut cfg = Config::load_from(&project).unwrap();
        cfg.home_dir = root.join("home");
        cfg.arai_base_dir = root.join("state");
        Self { root, cfg }
    }

    fn path(&self, index: usize) -> PathBuf {
        self.cfg.project_root.join(HOSTS[index].2)
    }

    fn write(&self, index: usize, body: &str) {
        let path = self.path(index);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn read(&self, index: usize) -> Value {
        serde_json::from_str(&fs::read_to_string(self.path(index)).unwrap()).unwrap()
    }

    fn check_identity(&self, index: usize, outcome: &HostRegistrationOutcome) {
        assert_eq!(outcome.host, HOSTS[index].0);
        assert_eq!(outcome.registration_label, HOSTS[index].1);
        assert_eq!(outcome.registration_path, self.path(index));
        assert_eq!(outcome.trust_guidance, HOSTS[index].3);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn reports_five_completed_writes_in_order() {
    let fixture = Fixture::new();
    let mut outcomes = Vec::new();
    register_default_hosts(&fixture.cfg, |outcome| {
        let index = outcomes.len();
        fixture.check_identity(index, outcome);
        assert_eq!(outcome.state, HostRegistrationState::Registered);
        assert_eq!(outcome.diagnostic, None);
        assert!(fixture.read(index)["hooks"].is_object());
        for later in index + 1..HOSTS.len() {
            assert!(!fixture.path(later).exists());
        }
        outcomes.push(outcome.clone());
    })
    .unwrap();
    assert_eq!(outcomes.len(), 5);
}

#[test]
fn optional_grok_failure_reports_original_diagnostic_and_continues() {
    let fixture = Fixture::new();
    let malformed = r#"{"hooks":false}"#;
    fixture.write(3, malformed);
    let mut outcomes = Vec::new();
    register_default_hosts(&fixture.cfg, |outcome| {
        let index = outcomes.len();
        fixture.check_identity(index, outcome);
        if index == 3 {
            assert_eq!(outcome.state, HostRegistrationState::Skipped);
            assert_eq!(
                outcome.diagnostic.as_deref(),
                Some("hooks is not an object")
            );
            assert!(!fixture.path(4).exists());
        } else {
            assert_eq!(outcome.state, HostRegistrationState::Registered);
            assert_eq!(outcome.diagnostic, None);
            assert!(fixture.path(index).is_file());
        }
        outcomes.push(outcome.clone());
    })
    .unwrap();
    assert_eq!(outcomes.len(), 5);
    assert_eq!(fs::read_to_string(fixture.path(3)).unwrap(), malformed);
    assert!(fixture.path(4).is_file());
}

#[test]
fn every_mandatory_failure_reports_only_attempted_prefix_and_keeps_earlier_writes() {
    for failed in [0, 1, 2, 4] {
        let fixture = Fixture::new();
        let malformed = r#"{"hooks":false}"#;
        fixture.write(failed, malformed);
        let mut outcomes = Vec::new();
        let error = register_default_hosts(&fixture.cfg, |outcome| {
            let index = outcomes.len();
            fixture.check_identity(index, outcome);
            if index == failed {
                assert_eq!(outcome.state, HostRegistrationState::Failed);
                assert_eq!(
                    outcome.diagnostic.as_deref(),
                    Some("hooks is not an object")
                );
            } else {
                assert_eq!(outcome.state, HostRegistrationState::Registered);
                assert_eq!(outcome.diagnostic, None);
                assert!(fixture.path(index).is_file());
            }
            for later in index + 1..5 {
                if later != failed {
                    assert!(!fixture.path(later).exists());
                }
            }
            outcomes.push(outcome.clone());
        })
        .unwrap_err();
        assert_eq!(error, "hooks is not an object");
        assert_eq!(outcomes.len(), failed + 1);
        assert_eq!(outcomes.last().unwrap().diagnostic.as_ref(), Some(&error));
        assert_eq!(fs::read_to_string(fixture.path(failed)).unwrap(), malformed);
        for earlier in 0..failed {
            assert!(fixture.read(earlier)["hooks"].is_object());
        }
        for later in failed + 1..5 {
            assert!(!fixture.path(later).exists());
        }
    }
}

#[test]
fn malformed_configurations_keep_existing_errors_and_original_file() {
    for (body, expected) in [
        ("[]", "is not an object"),
        ("{ invalid JSON", "Could not parse"),
        (r#"{"hooks":[]}"#, "hooks is not an object"),
        (
            r#"{"hooks":{"PreToolUse":{}}}"#,
            "PreToolUse is not an array",
        ),
    ] {
        let fixture = Fixture::new();
        fixture.write(0, body);
        let mut reports = 0;
        let error = register_default_hosts(&fixture.cfg, |outcome| {
            reports += 1;
            assert_eq!(outcome.state, HostRegistrationState::Failed);
            assert!(outcome.diagnostic.as_ref().unwrap().contains(expected));
        })
        .unwrap_err();
        assert!(error.contains(expected), "{error}");
        assert_eq!(reports, 1);
        assert_eq!(fs::read_to_string(fixture.path(0)).unwrap(), body);
    }
}

#[test]
fn add_keeps_ensure_hooks_error_as_a_warning_and_stops_before_copilot() {
    let fixture = Fixture::new();
    fixture.write(3, r#"{"hooks":false}"#);
    let output = Command::new(env!("CARGO_BIN_EXE_arai"))
        .current_dir(&fixture.cfg.project_root)
        .env("HOME", &fixture.cfg.home_dir)
        .env("USERPROFILE", &fixture.cfg.home_dir)
        .env("ARAI_BASE_DIR", &fixture.cfg.arai_base_dir)
        .env("ARAI_TELEMETRY", "off")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .args(["add", "Never run cargo clean"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("hooks is not an object"), "{stderr}");
    assert!(stderr.to_lowercase().contains("warning"), "{stderr}");
    assert!(!fixture.path(4).exists());
    for earlier in 0..3 {
        assert!(fixture.path(earlier).is_file());
    }
}
