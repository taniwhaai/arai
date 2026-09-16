//! Structural enforcement regressions through both embedding and stdin APIs.
//! Tool commands are evaluated as payloads and are never executed.
use arai::{config::Config, hooks, parser, repo_check, store::Store};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture {
    root: PathBuf,
    cfg: Config,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "arai_structural_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        for path in ["project/.git", "home", "state"] {
            fs::create_dir_all(root.join(path)).unwrap();
        }
        let cfg = Config {
            project_root: root.join("project"),
            home_dir: root.join("home"),
            arai_base_dir: root.join("state"),
            extra_sources: Vec::new(),
            guardrails_mode: "advise".into(),
            llm_command: None,
            api_url: None,
            api_key_env: None,
            api_model: None,
        };
        Self { root, cfg }
    }

    fn store(&self, rules: &str) -> Store {
        let db = Store::open(&self.cfg.arai_base_dir.join("test.db")).unwrap();
        if !rules.is_empty() {
            let source = self.cfg.project_root.join("AGENTS.md");
            let triples = parser::extract_rules(rules, "test", 0.95);
            assert!(!triples.is_empty());
            db.upsert_file(&source.to_string_lossy(), rules, &triples, "test")
                .unwrap();
            db.classify_all_guardrails().unwrap();
        }
        db
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_arai"));
        command
            .current_dir(&self.cfg.project_root)
            .env("HOME", &self.cfg.home_dir)
            .env("USERPROFILE", &self.cfg.home_dir)
            .env("ARAI_BASE_DIR", &self.cfg.arai_base_dir)
            .env("ARAI_TELEMETRY", "off")
            .env_remove("ARAI_DISABLED")
            .env_remove("ARAI_DENY_MODE")
            .env_remove("GROK_HOOK_EVENT")
            .env_remove("GROK_SESSION_ID")
            .env_remove("CLAUDE_PROJECT_DIR")
            .env_remove("CLAUDE_PLUGIN_ROOT");
        command
    }

    fn stdin_hook(&self, payload: &Value) -> Output {
        let mut child = self
            .command()
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
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).ok();
    }
}

fn malformed_payloads() -> Vec<Value> {
    vec![
        Value::Null,
        json!([]),
        json!({}),
        json!({"hook_event_name":null}),
        json!({"tool_name":42,"tool_input":{}}),
        json!({"tool_name":"  ","tool_input":{}}),
        json!({"tool_name":"Bash"}),
        json!({"tool_name":"Bash","tool_input":null}),
        json!({"tool_name":"Bash","tool_input":[]}),
        json!({"tool_name":"Bash","tool_input":{"command":42}}),
        json!({"tool_name":"Bash","tool_input":{"command":"bad\0command"}}),
        json!({"tool_name":"Write","tool_input":{"file_path":42}}),
        json!({"tool_name":"Edit","tool_input":{"file_path":""}}),
        json!({"tool_name":"Edit","tool_input":{"file_path":"bad\0path"}}),
        json!({"toolName":"run_terminal_command","toolInput":{"command":null}}),
        json!({"tool_name":"apply_patch","tool_input":{"command":null}}),
    ]
}

#[test]
fn embedding_rejects_malformed_pretooluse_envelopes() {
    let fixture = Fixture::new();
    let db = fixture.store("");
    for payload in malformed_payloads() {
        assert!(
            hooks::match_hook(&payload, &fixture.cfg, &db).is_err(),
            "malformed payload was treated as allow: {payload}"
        );
    }
}

#[test]
fn stdin_validation_precedes_the_no_database_fast_path() {
    let fixture = Fixture::new();
    assert!(!fixture.cfg.db_path().exists());
    for payload in malformed_payloads() {
        let output = fixture.stdin_hook(&payload);
        assert!(output.status.success());
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            response["hookSpecificOutput"]["permissionDecision"], "deny",
            "{payload}"
        );
    }
}

#[test]
fn valid_unknown_tools_aliases_and_passive_events_remain_compatible() {
    let fixture = Fixture::new();
    let db = fixture.store("");
    for payload in [
        json!({"tool_name":"mcp__example__custom_tool","tool_input":{}}),
        json!({"tool_name":"FutureHostTool","tool_input":{"custom_field":[1,2]}}),
        json!({"hookEventName":"pre_tool_use","toolName":"run_terminal_command","toolInput":{"command":""}}),
        json!({"tool_name":"NotebookEdit","tool_input":{"notebook_path":"example.ipynb"}}),
        json!({"tool_name":"Write","tool_input":{"file_path":"notes.txt","content":""}}),
        json!({"hook_event_name":"UserPromptSubmit","prompt":"hello"}),
        json!({"hook_event_name":"SessionStart"}),
        json!({"hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":null}),
    ] {
        assert!(
            hooks::match_hook(&payload, &fixture.cfg, &db).is_ok(),
            "{payload}"
        );
        let output = fixture.stdin_hook(&payload);
        assert!(output.status.success(), "{payload}");
        if payload["hook_event_name"] == "SessionStart" {
            let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(response["systemMessage"].as_str().unwrap().contains("Arai"));
            assert!(response["hookSpecificOutput"]["permissionDecision"].is_null());
        } else {
            assert!(output.stdout.is_empty(), "{payload}: {:?}", output.stdout);
        }
    }
}

fn rename_diff(old: &str, new: &str) -> String {
    format!(
        "diff --git a/{old} b/{new}\nsimilarity index 100%\nrename from {old}\nrename to {new}\n"
    )
}

#[test]
fn renamed_and_copied_destinations_obey_creation_rules() {
    let fixture = Fixture::new();
    let db = fixture.store("## Alembic\n\n- Never hand-write migration files\n");
    let renamed = rename_diff("scratch.py", "alembic/x.py");
    let copied = renamed.replace("rename ", "copy ");
    for diff in [renamed, copied] {
        let report = repo_check::check_diff(&diff, &fixture.cfg, &db).unwrap();
        assert!(report.blocked, "{diff}");
        assert_eq!(report.files[0].path, "alembic/x.py");
        assert_eq!(report.files[0].matched.len(), 1);
    }
    assert!(
        !repo_check::check_diff(&rename_diff("scratch.py", "notes.py"), &fixture.cfg, &db)
            .unwrap()
            .blocked
    );
}

#[test]
fn renamed_source_and_destination_both_obey_edit_rules_without_duplicate_matches() {
    let fixture = Fixture::new();
    let db = fixture.store("## Alembic\n\n- Never modify migration files\n");
    for (old, new) in [
        ("alembic/source.py", "scratch.py"),
        ("scratch.py", "alembic/destination.py"),
        ("alembic/source.py", "alembic/destination.py"),
    ] {
        let report = repo_check::check_diff(&rename_diff(old, new), &fixture.cfg, &db).unwrap();
        assert!(report.blocked, "{old} -> {new}");
        assert_eq!(
            report.files[0].matched.len(),
            1,
            "duplicate rule on {old} -> {new}"
        );
    }
}

#[test]
fn changed_copy_hunks_are_additions_without_changing_the_public_kind_schema() {
    let diff = "diff --git a/scratch.py b/alembic/x.py\nsimilarity index 75%\n\
copy from scratch.py\ncopy to alembic/x.py\nindex 111..222 100644\n\
--- a/scratch.py\n+++ b/alembic/x.py\n@@ -1 +1 @@\n-old\n+new\n";
    let changes = repo_check::parse_unified_diff(diff).unwrap();
    assert_eq!(changes[0].kind, repo_check::ChangeKind::Added);
    assert_eq!(changes[0].old_content, "old\n");
    assert_eq!(changes[0].new_content, "new\n");
    let fixture = Fixture::new();
    let db = fixture.store("## Alembic\n\n- Never hand-write migration files\n");
    assert!(
        repo_check::check_diff(diff, &fixture.cfg, &db)
            .unwrap()
            .blocked
    );
}

#[test]
fn empty_and_binary_additions_still_match_path_rules() {
    let fixture = Fixture::new();
    let db = fixture.store("## Alembic\n\n- Never hand-write migration files\n");
    let empty =
        "diff --git a/alembic/x b/alembic/x\nnew file mode 100644\nindex 0000000..e69de29\n";
    let binary = format!("{empty}Binary files /dev/null and b/alembic/x differ\n");
    for diff in [empty, binary.as_str()] {
        let report = repo_check::check_diff(diff, &fixture.cfg, &db).unwrap();
        assert!(report.blocked);
        assert_eq!(report.files[0].kind, repo_check::ChangeKind::Added);
    }
}

#[test]
fn repository_gate_does_not_consult_an_unrelated_live_session() {
    let fixture = Fixture::new();
    let db = fixture.store("## Alembic\n\n- Never hand-write migration files without review\n");
    let diff = "diff --git a/alembic/x b/alembic/x\nnew file mode 100644\nindex 0000000..e69de29\n";
    assert!(
        repo_check::check_diff(diff, &fixture.cfg, &db)
            .unwrap()
            .blocked
    );
    arai::session::record_tool_call(
        &fixture.cfg.arai_base_dir,
        "check-diff",
        "Bash",
        &["review".to_string()],
    );
    assert!(
        repo_check::check_diff(diff, &fixture.cfg, &db)
            .unwrap()
            .blocked,
        "the repository gate has no host session whose prerequisites can waive a rule"
    );
}

#[test]
fn notebook_and_batch_edits_match_paths_and_each_content_field() {
    let fixture = Fixture::new();
    let db = fixture.store("## Alembic\n\n- Never modify migration files\n");
    for (tool, input) in [
        (
            "NotebookEdit",
            json!({"notebook_path":"alembic/x.ipynb","new_source":"pass"}),
        ),
        (
            "NotebookEdit",
            json!({"notebook_path":"x.ipynb","new_source":"from alembic import op"}),
        ),
        ("MultiEdit", json!({"file_path":"alembic/x.py","edits":[]})),
        (
            "MultiEdit",
            json!({"file_path":"x.py","edits":[{"old_string":"pass","new_string":"pass"},{"old_string":"from alembic import op","new_string":"pass"}]}),
        ),
        (
            "MultiEdit",
            json!({"file_path":"x.py","edits":[{"old_string":"pass","new_string":"pass"},{"old_string":"pass","new_string":"from alembic import op"}]}),
        ),
    ] {
        let result = hooks::match_hook(
            &json!({"tool_name":tool,"tool_input":input}),
            &fixture.cfg,
            &db,
        )
        .unwrap();
        assert_eq!(result.tool_name, tool, "canonical names must remain stable");
        assert_eq!(result.matched.len(), 1, "{tool}: {input}");
        assert_eq!(
            hooks::highest_severity(&result.matched),
            arai::intent::Severity::Block
        );
    }
}

#[test]
fn batch_edits_preserve_the_create_rule_inverse_and_explicit_notebook_scope() {
    let fixture = Fixture::new();
    let db = fixture.store("## Alembic\n\n- Never hand-write migration files\n");
    let result = hooks::match_hook(&json!({"tool_name":"MultiEdit","tool_input":{"file_path":"alembic/x.py","edits":[{"old_string":"old","new_string":"new"}]}}), &fixture.cfg, &db).unwrap();
    assert!(
        result.matched.is_empty(),
        "creation prohibition must still permit editing"
    );
    let result = hooks::match_hook(&json!({"tool_name":"NotebookEdit","tool_input":{"notebook_path":"alembic/x.ipynb","new_source":"pass"}}), &fixture.cfg, &db).unwrap();
    assert_eq!(
        result.matched.len(),
        1,
        "preserve the existing explicit NotebookEdit scope on Create rules"
    );
}

#[test]
fn notebook_and_batch_edits_use_the_native_cwd_for_graph_lookup() {
    let fixture = Fixture::new();
    let db = fixture.store("## Alembic\n\n- Never modify migration files\n");
    let cwd = fixture.cfg.project_root.join("nested");
    fs::create_dir_all(&cwd).unwrap();
    db.upsert_code_graph(&[arai::code_scanner::ImportInfo {
        tool_name: "alembic".into(),
        source_file: cwd.join("existing.py").to_string_lossy().into_owned(),
        directory: cwd.to_string_lossy().into_owned(),
    }])
    .unwrap();
    for (tool, input) in [
        (
            "MultiEdit",
            json!({"file_path":"./unused/../x.py","edits":[]}),
        ),
        (
            "NotebookEdit",
            json!({"notebook_path":"./unused/../x.ipynb","new_source":"pass"}),
        ),
    ] {
        let payload = json!({"tool_name":tool,"tool_input":input,"cwd":cwd});
        let result = hooks::match_hook(&payload, &fixture.cfg, &db).unwrap();
        assert_eq!(result.matched.len(), 1, "{payload}");
        assert!(result.terms.iter().any(|term| term == "alembic"));
    }
}

#[test]
fn native_path_separators_follow_the_platform() {
    let input = json!({"file_path":r"alembic\x.py","content":"pass"});
    let terms = arai::guardrails::extract_terms("Write", &input);
    #[cfg(windows)]
    assert!(terms.iter().any(|term| term == "alembic"));
    #[cfg(not(windows))]
    assert!(
        !terms.iter().any(|term| term == "alembic"),
        "backslash is a literal Unix filename character"
    );
}

#[test]
fn cached_diff_uses_raw_content_and_stable_paths_despite_git_display_settings() {
    let fixture = Fixture::new();
    let git = |args: &[&str]| {
        let result = Command::new("git")
            .args(args)
            .current_dir(&fixture.cfg.project_root)
            .env("HOME", &fixture.cfg.home_dir)
            .env("USERPROFILE", &fixture.cfg.home_dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    git(&["init", "--quiet"]);
    fs::write(
        fixture.cfg.project_root.join("AGENTS.md"),
        "## Alembic\n\n- Never hand-write migration files\n",
    )
    .unwrap();
    let scan = fixture.command().arg("scan").output().unwrap();
    assert!(
        scan.status.success(),
        "scan: {}",
        String::from_utf8_lossy(&scan.stderr)
    );
    git(&["config", "diff.noprefix", "true"]);
    git(&["config", "diff.srcPrefix", "custom-before/"]);
    git(&["config", "diff.dstPrefix", "custom-after/"]);
    git(&["config", "diff.relative", "true"]);
    git(&["config", "diff.hide.textconv", "git --version"]);
    fs::write(
        fixture.cfg.project_root.join(".gitattributes"),
        "*.py diff=hide\n",
    )
    .unwrap();
    fs::create_dir_all(fixture.cfg.project_root.join("src")).unwrap();
    fs::write(
        fixture.cfg.project_root.join("src/generated.py"),
        "from alembic import op\n",
    )
    .unwrap();
    git(&["add", "--", "src/generated.py"]);
    let output = fixture
        .command()
        .args(["check-diff", "--cached", "--json"])
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(report["blocked"], true);
    assert_eq!(report["files"][0]["path"], "src/generated.py");
    assert_eq!(report["files"][0]["matched"].as_array().unwrap().len(), 1);
}
