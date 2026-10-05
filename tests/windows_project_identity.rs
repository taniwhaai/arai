//! Real Windows aliases and registered PowerShell hooks must share policy state.
#![cfg(windows)]

use arai::{config::Config, intent::Severity, parser, store::Store};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

struct Project(PathBuf);

impl Project {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "arai_identity_{}_{}_{stamp}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("Long Project Name/.git")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        fs::write(
            root.join("Long Project Name/AGENTS.md"),
            "- Never run cargo clean\n",
        )
        .unwrap();
        Self(root)
    }

    fn root(&self) -> PathBuf {
        self.0.join("Long Project Name")
    }

    fn config(&self, root: &Path) -> Config {
        Config {
            project_root: root.to_path_buf(),
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

    fn isolated(&self, command: &mut Command, root: &Path) {
        command
            .current_dir(root)
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("ARAI_BASE_DIR", self.0.join("state"))
            .env("ARAI_TELEMETRY", "off")
            .env_remove("ARAI_DISABLED")
            .env_remove("ARAI_DENY_MODE")
            .env_remove("GROK_HOOK_EVENT")
            .env_remove("GROK_SESSION_ID")
            .env_remove("CLAUDE_PROJECT_DIR")
            .env_remove("CLAUDE_PLUGIN_ROOT");
    }

    fn run(&self, root: &Path, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_arai"));
        self.isolated(&mut command, root);
        let out = command.args(args).output().unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    fn registered_hook(&self, host: &str, payload: Value) -> Value {
        let settings: Value = serde_json::from_slice(
            &fs::read(self.root().join(format!(".{host}/hooks.json"))).unwrap(),
        )
        .unwrap();
        let handler = if host == "codex" {
            &settings["hooks"]["PreToolUse"][0]["hooks"][0]
        } else {
            &settings["hooks"]["preToolUse"][0]
        };
        let command = if host == "codex" {
            &handler["commandWindows"]
        } else {
            &handler["command"]
        };
        let command = command.as_str().expect("registered Windows handler");
        let encoded = command
            .strip_prefix("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ")
            .expect("registered PowerShell command");
        let mut shell = Command::new("powershell.exe");
        self.isolated(&mut shell, &self.root());
        let mut child = shell
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand",
                encoded,
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
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        if out.stdout.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&out.stdout).unwrap()
        }
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn short_path(path: &Path) -> Option<PathBuf> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetShortPathNameW(long: *const u16, short: *mut u16, length: u32) -> u32;
    }
    let input: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let required = unsafe { GetShortPathNameW(input.as_ptr(), std::ptr::null_mut(), 0) };
    assert_ne!(
        required,
        0,
        "GetShortPathNameW: {}",
        std::io::Error::last_os_error()
    );
    let mut output = vec![0; required as usize];
    let length = unsafe { GetShortPathNameW(input.as_ptr(), output.as_mut_ptr(), required) };
    assert!(length > 0 && length < required);
    output.truncate(length as usize);
    let short = PathBuf::from(std::ffi::OsString::from_wide(&output));
    if short.to_string_lossy().contains('~') {
        Some(short)
    } else {
        eprintln!("Skipping 8.3 alias check: this volume did not supply a short name");
        None
    }
}

fn legacy_slug(root: &Path) -> String {
    let digest = Sha256::digest(root.to_string_lossy().as_bytes());
    let hash: String = digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{}-{hash}", root.file_name().unwrap().to_string_lossy())
}

#[test]
fn case_verbatim_and_short_aliases_have_one_project_identity() {
    let p = Project::new();
    let root = p.root();
    let expected = p.config(&root).project_slug();
    let mut aliases = vec![
        PathBuf::from(root.to_string_lossy().to_uppercase()),
        fs::canonicalize(&root).unwrap(),
    ];
    if let Some(short) = short_path(&root) {
        aliases.push(short);
    }
    for alias in aliases {
        assert_eq!(
            p.config(&alias).project_slug(),
            expected,
            "{}",
            alias.display()
        );
    }
}

#[test]
fn legacy_store_migration_preserves_manual_rules_and_discovery_pins() {
    let p = Project::new();
    let alias = PathBuf::from(p.root().to_string_lossy().to_uppercase());
    let old_dir = p.0.join("state/projects").join(legacy_slug(&alias));
    let old_db = old_dir.join("arai.db");
    let source = alias.join("AGENTS.md").to_string_lossy().into_owned();
    let nested_source = alias
        .join("packages/web/AGENTS.md")
        .to_string_lossy()
        .into_owned();
    let nested_content = "- Never run npm publish\n";
    fs::create_dir_all(p.root().join("packages/web")).unwrap();
    fs::write(p.root().join("packages/web/AGENTS.md"), nested_content).unwrap();
    let expected_pins = {
        let db = Store::open(&old_db).unwrap();
        let content = "- Never run cargo clean\n";
        db.upsert_file(
            &source,
            content,
            &parser::extract_rules(content, "agents_md", 0.95),
            "agents_md",
        )
        .unwrap();
        db.upsert_file(
            &nested_source,
            nested_content,
            &parser::extract_rules(nested_content, "agents_md", 0.95),
            "agents_md",
        )
        .unwrap();
        let manual = "- Never run git reset --hard\n";
        db.upsert_file(
            "manual://arai-add/retained",
            manual,
            &parser::extract_rules(manual, "manual", 0.95),
            "manual",
        )
        .unwrap();
        db.classify_all_guardrails().unwrap();
        assert_eq!(
            db.set_severity_override("cargo", Severity::Inform)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            db.set_severity_override("git", Severity::Warn)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            db.set_severity_override("npm", Severity::Block)
                .unwrap()
                .len(),
            1
        );
        let conn = rusqlite::Connection::open(&old_db).unwrap();
        conn.execute("UPDATE source_metadata SET owner='discovery' WHERE file_id=(SELECT id FROM files WHERE path=?1)", [&source]).unwrap();
        conn.execute("UPDATE source_metadata SET owner='discovery' WHERE file_id=(SELECT id FROM files WHERE path=?1)", [&nested_source]).unwrap();
        db.list_severity_overrides()
            .unwrap()
            .into_iter()
            .map(|pin| (pin.subject, pin.predicate, pin.object, pin.to))
            .collect::<Vec<_>>()
    };
    p.run(&alias, &["init"]);
    p.run(&p.root(), &["scan"]);
    let canonical_db = p.config(&p.root()).db_path();
    assert!(canonical_db.exists());
    assert!(!old_db.exists(), "legacy DB should be moved, not forked");
    let db = Store::open(&canonical_db).unwrap();
    let overrides = db.list_severity_overrides().unwrap();
    assert_eq!(
        overrides.len(),
        3,
        "both source and manual severity choices survive"
    );
    for (subject, predicate, object, severity) in expected_pins {
        assert!(
            overrides.iter().any(|pin| pin.subject == subject
                && pin.predicate == predicate
                && pin.object == object
                && pin.to == severity),
            "lost severity choice for ({subject}, {predicate}, {object})"
        );
    }
    assert!(db
        .list_files()
        .unwrap()
        .contains(&"manual://arai-add/retained".into()));
    let nested_identity = fs::canonicalize(p.root().join("packages/web/AGENTS.md")).unwrap();
    assert!(db
        .list_files()
        .unwrap()
        .iter()
        .any(|path| { fs::canonicalize(path).is_ok_and(|identity| identity == nested_identity) }));
    for (cwd, denied) in [(p.root().join("packages/web"), true), (p.root(), false)] {
        let output = p.registered_hook("codex", json!({"hook_event_name":"PreToolUse", "tool_name":"Bash",
            "tool_input":{"command":"npm publish"}, "cwd":cwd, "session_id":"nested-identity-test"}));
        assert_eq!(
            output["hookSpecificOutput"]["permissionDecision"] == "deny",
            denied,
            "{output}"
        );
    }
}

#[test]
fn conflicting_audit_histories_leave_legacy_database_and_both_histories_intact() {
    let p = Project::new();
    let alias = PathBuf::from(p.root().to_string_lossy().to_uppercase());
    let legacy_db = seed_legacy_store(&p, &alias, "audit-conflict");
    let audit_paths = [
        p.0.join("state/audit").join(legacy_slug(&alias)),
        p.0.join("state/audit")
            .join(p.config(&p.root()).project_slug()),
    ];
    for (index, path) in audit_paths.iter().enumerate() {
        fs::create_dir_all(path).unwrap();
        fs::write(path.join("history.jsonl"), index.to_string()).unwrap();
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_arai"));
    p.isolated(&mut command, &alias);
    let out = command.arg("status").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Both legacy and canonical audit"));
    assert!(!p.config(&p.root()).db_path().exists());
    assert!(legacy_db.exists());
    for (index, path) in audit_paths.iter().enumerate() {
        assert_eq!(
            fs::read_to_string(path.join("history.jsonl")).unwrap(),
            index.to_string()
        );
    }
}

#[test]
fn scoped_write_to_nonexistent_file_expands_short_ancestor() {
    let p = Project::new();
    let Some(short) = short_path(&p.root()) else {
        return;
    };
    fs::write(
        p.root().join("AGENTS.md"),
        "## Alembic\n\n- Never hand-write migration files\n",
    )
    .unwrap();
    p.run(&p.root(), &["init"]);
    for root in [p.root(), short.clone(), fs::canonicalize(&short).unwrap()] {
        let new_file = root.join("alembic/new_migration.py");
        assert!(!new_file.exists());
        let output = p.registered_hook("codex", json!({"hook_event_name":"PreToolUse", "tool_name":"Write",
            "tool_input":{"file_path":new_file, "content":"from alembic import op"}, "cwd":p.root()}));
        assert_eq!(
            output["hookSpecificOutput"]["permissionDecision"], "deny",
            "{new_file:?}: {output}"
        );
    }
}

fn seed_legacy_store(p: &Project, root: &Path, marker: &str) -> PathBuf {
    let path =
        p.0.join("state/projects")
            .join(legacy_slug(root))
            .join("arai.db");
    let db = Store::open(&path).unwrap();
    db.set_meta("identity-test-marker", marker).unwrap();
    path
}

#[test]
fn long_path_launch_migrates_short_only_store_and_audit_history() {
    let p = Project::new();
    let Some(short) = short_path(&p.root()) else {
        return;
    };
    let old_db = seed_legacy_store(&p, &short, "short-only");
    let old_audit = p.0.join("state/audit").join(legacy_slug(&short));
    fs::create_dir_all(&old_audit).unwrap();
    // Opaque bytes represent existing chain state; migration must not rewrite them.
    let history = b"existing audit history\n";
    fs::write(old_audit.join("retained-history.jsonl"), history).unwrap();
    p.run(&p.root(), &["status"]);
    let canonical = p.config(&p.root());
    assert!(!old_db.exists());
    assert!(!old_audit.exists());
    assert_eq!(
        Store::open(&canonical.db_path())
            .unwrap()
            .get_meta("identity-test-marker")
            .unwrap()
            .as_deref(),
        Some("short-only")
    );
    assert_eq!(
        fs::read(
            p.0.join("state/audit")
                .join(canonical.project_slug())
                .join("retained-history.jsonl")
        )
        .unwrap(),
        history
    );
}

#[test]
fn long_path_launch_migrates_abbreviated_parent_with_long_project_name() {
    let p = Project::new();
    let Some(short_parent) = short_path(&p.0) else {
        return;
    };
    let mixed = short_parent.join("Long Project Name");
    let old_db = seed_legacy_store(&p, &mixed, "short-parent");
    p.run(&p.root(), &["status"]);
    assert!(!old_db.exists());
    assert_eq!(
        Store::open(&p.config(&p.root()).db_path())
            .unwrap()
            .get_meta("identity-test-marker")
            .unwrap()
            .as_deref(),
        Some("short-parent")
    );
}

#[test]
fn ambiguous_legacy_stores_fail_without_altering_either_database() {
    let p = Project::new();
    let Some(short) = short_path(&p.root()) else {
        return;
    };
    let uppercase = PathBuf::from(p.root().to_string_lossy().to_uppercase());
    let short_db = seed_legacy_store(&p, &short, "short");
    let uppercase_db = seed_legacy_store(&p, &uppercase, "uppercase");
    assert_ne!(short_db, uppercase_db);
    let mut command = Command::new(env!("CARGO_BIN_EXE_arai"));
    p.isolated(&mut command, &uppercase);
    let out = command.arg("status").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Multiple legacy Arai stores"));
    assert!(!p.config(&p.root()).db_path().exists());
    for (path, marker) in [(short_db, "short"), (uppercase_db, "uppercase")] {
        assert!(path.exists());
        assert_eq!(
            Store::open(&path)
                .unwrap()
                .get_meta("identity-test-marker")
                .unwrap()
                .as_deref(),
            Some(marker)
        );
    }
}

#[test]
fn existing_canonical_store_wins_and_leaves_legacy_store_intact() {
    let p = Project::new();
    let uppercase = PathBuf::from(p.root().to_string_lossy().to_uppercase());
    let legacy_db = seed_legacy_store(&p, &uppercase, "legacy");
    let canonical_db = p.config(&p.root()).db_path();
    {
        let db = Store::open(&canonical_db).unwrap();
        db.set_meta("identity-test-marker", "canonical").unwrap();
    }
    p.run(&uppercase, &["status"]);
    for (path, marker) in [(canonical_db, "canonical"), (legacy_db, "legacy")] {
        assert!(path.exists());
        assert_eq!(
            Store::open(&path)
                .unwrap()
                .get_meta("identity-test-marker")
                .unwrap()
                .as_deref(),
            Some(marker)
        );
    }
}

#[test]
fn concurrent_first_launches_move_one_legacy_store() {
    let p = Project::new();
    let Some(short) = short_path(&p.root()) else {
        return;
    };
    let old_db = seed_legacy_store(&p, &short, "concurrent");
    let mut children = Vec::new();
    for root in [&short, &p.root()] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_arai"));
        p.isolated(&mut command, root);
        children.push(
            command
                .arg("status")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    for child in children {
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    assert!(!old_db.exists());
    assert_eq!(
        Store::open(&p.config(&p.root()).db_path())
            .unwrap()
            .get_meta("identity-test-marker")
            .unwrap()
            .as_deref(),
        Some("concurrent")
    );
}

#[test]
fn short_path_init_and_registered_powershell_hooks_share_rules() {
    let p = Project::new();
    let Some(short) = short_path(&p.root()) else {
        return;
    };
    p.run(&short, &["init"]);
    for host in ["codex", "cursor"] {
        for (cwd, workdir) in [
            (p.root(), None),
            (short.clone(), None),
            (p.root(), Some(short.clone())),
            (
                PathBuf::from(p.root().to_string_lossy().to_lowercase()),
                None,
            ),
        ] {
            for (command, denied) in [("cargo clean", true), ("cargo check", false)] {
                let mut payload = json!({"hook_event_name":"PreToolUse", "tool_name":"Bash",
                "tool_input":{"command":command}, "cwd":cwd, "session_id":"identity-test"});
                if let Some(workdir) = &workdir {
                    payload["tool_input"]["workdir"] = json!(workdir);
                }
                if host == "cursor" {
                    payload["hook_event_name"] = json!("preToolUse");
                    payload["tool_name"] = json!("Shell");
                    payload["cursor_version"] = json!("2.4.0");
                    payload["conversation_id"] = json!("identity-test");
                    payload["workspace_roots"] = json!([p.root()]);
                }
                let output = p.registered_hook(host, payload);
                let decision = if host == "cursor" {
                    &output["permission"]
                } else {
                    &output["hookSpecificOutput"]["permissionDecision"]
                };
                assert_eq!(
                    decision == "deny",
                    denied,
                    "{host}: {command}: cwd={cwd:?}, workdir={workdir:?}: {output}"
                );
            }
        }
    }
    let stores = fs::read_dir(p.0.join("state/projects"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join("arai.db").exists())
        .count();
    assert_eq!(stores, 1);
}
