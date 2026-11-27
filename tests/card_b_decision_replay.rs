//! Policy assertions through the real CLI, isolated from user policy and modes.
use arai::{config::Config, intent::Severity, session, store::Store};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let f =
            Self(std::env::temp_dir().join(format!("arai-card-b-{}-{nonce}", std::process::id())));
        for path in ["project/.git", "home", "state"] {
            fs::create_dir_all(f.0.join(path)).unwrap();
        }
        fs::write(
            f.0.join("project/CLAUDE.md"),
            include_bytes!("fixtures/card_a_alembic/CLAUDE.md"),
        )
        .unwrap();
        let out = f.command(&["scan"]).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        f
    }
    fn config(&self) -> Config {
        Config {
            project_root: self.0.join("project"),
            home_dir: self.0.join("home"),
            arai_base_dir: self.0.join("state"),
            extra_sources: vec![],
            guardrails_mode: "inject".into(),
            llm_command: None,
            api_url: None,
            api_key_env: None,
            api_model: None,
        }
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_arai"));
        cmd.args(args).current_dir(self.0.join("project"));
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy().to_ascii_uppercase();
            if ["ARAI_", "CLAUDE_", "CODEX_", "GROK_", "CURSOR_", "COPILOT_"]
                .iter()
                .any(|p| name.starts_with(p))
                || ["ANTHROPIC_API_KEY", "OPENAI_API_KEY"].contains(&name.as_str())
            {
                cmd.env_remove(key);
            }
        }
        cmd.env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("ARAI_BASE_DIR", self.0.join("state"))
            .env("DO_NOT_TRACK", "1");
        cmd
    }
    fn scenario(&self, rows: Value) -> PathBuf {
        let path = self.0.join("scenarios.json");
        fs::write(
            &path,
            json!({"_comment":"supported metadata", "scenarios":rows}).to_string(),
        )
        .unwrap();
        path
    }
    fn replay(&self, rows: Value, mode: Option<(&str, &str)>) -> Output {
        let path = self.scenario(rows);
        let mut cmd = self.command(&["test", path.to_str().unwrap(), "--json"]);
        if let Some((key, value)) = mode {
            cmd.env(key, value);
        }
        cmd.output().unwrap()
    }
    fn native(&self, payload: &Value, mode: Option<(&str, &str)>) -> Output {
        let mut cmd = self.command(&["guardrails", "--match-stdin"]);
        if let Some((key, value)) = mode {
            cmd.env(key, value);
        }
        let mut child = cmd
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
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn hook(path: &str) -> Value {
    json!({"hook_event_name":"PreToolUse","tool_name":"Write","tool_input":{"file_path":path,"content":""},"session_id":"b-session"})
}
fn row(name: &str, hook: Value, expect: Value) -> Value {
    json!({"name":name,"hook":hook,"expect":expect})
}
fn results(out: &Output, success: bool) -> Value {
    assert_eq!(
        out.status.success(),
        success,
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, at: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files.insert(path.strip_prefix(root).unwrap().to_path_buf(), vec![]);
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

#[test]
fn assertions_are_strict_and_the_whole_file_is_validated() {
    let f = Fixture::new();
    for invalid in [
        json!([]),
        json!([[], [], null, null]),
        json!({"decison":"deny"}),
        json!({"rule_ids":[]}),
        json!({"decision":"DENY"}),
        json!({"decision":"block"}),
        json!({"decision":null}),
        json!({"decision":false}),
        json!({"decision":[]}),
        json!({"min_matches":-1}),
        json!({"min_matches":1.5}),
        json!({"max_matches":"1"}),
        json!({"matches_subject":[1]}),
        json!({"does_not_match_subject":null}),
    ] {
        let out = f.replay(
            json!([row("invalid-assertion", hook("app/example.py"), invalid)]),
            None,
        );
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("invalid-assertion") || err.contains("scenarios.json"),
            "{err}"
        );
    }
    for event in [
        "PostToolUse",
        "post_tool_use",
        "UserPromptSubmit",
        "Stop",
        "SessionStart",
    ] {
        let mut h = hook("app/example.py");
        h["hook_event_name"] = json!(event);
        let out = f.replay(
            json!([
                row("first-valid", hook("app/example.py"), json!({})),
                row("unsupported-event", h, json!({"decision":"allow"}))
            ]),
            None,
        );
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        assert!(String::from_utf8_lossy(&out.stderr).contains("unsupported-event"));
    }
    let out=f.replay(json!([{"name":"omitted","hook":hook("app/example.py")},row("null-bounds",hook("app/example.py"),json!({"min_matches":null,"max_matches":null}))]),None);
    assert_eq!(results(&out, true).as_array().unwrap().len(), 2);
    assert_eq!(results(&f.replay(json!([]), None), true), json!([]));
    let path = f.0.join("empty.json");
    fs::write(&path, "{\"_comment\":\"empty\"}").unwrap();
    assert!(f
        .command(&["test", path.to_str().unwrap()])
        .output()
        .unwrap()
        .status
        .success());
    for raw in [
        json!([]),
        json!({"scenarios":[["sequence-scenario",hook("app/example.py"),{}]]}),
    ] {
        fs::write(&path, raw.to_string()).unwrap();
        let out = f
            .command(&["test", path.to_str().unwrap(), "--json"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        assert!(String::from_utf8_lossy(&out.stderr).contains("expected an object"));
    }
}

#[test]
fn native_replay_override_and_mode_parity() {
    let f = Fixture::new();
    let blocked = hook("alembic/versions/001_example.py");
    let native = f.native(&blocked, None);
    println!(
        "native deny input={blocked} exit={:?} stdout={} stderr={}",
        native.status.code(),
        String::from_utf8_lossy(&native.stdout),
        String::from_utf8_lossy(&native.stderr)
    );
    assert_eq!(native.status.code(), Some(0));
    assert_eq!(
        serde_json::from_slice::<Value>(&native.stdout).unwrap()["hookSpecificOutput"]
            ["permissionDecision"],
        "deny"
    );
    let near = hook("app/example.py");
    let native = f.native(&near, None);
    println!(
        "native control input={near} exit={:?} stdout={} stderr={}",
        native.status.code(),
        String::from_utf8_lossy(&native.stdout),
        String::from_utf8_lossy(&native.stderr)
    );
    assert_eq!(native.status.code(), Some(0));
    assert!(native.stdout.is_empty());
    let rows = json!([
        row(
            "block",
            blocked.clone(),
            json!({"decision":"deny","matches_subject":["Alembic"],"min_matches":1})
        ),
        row(
            "near",
            near.clone(),
            json!({"decision":"allow","max_matches":0})
        ),
        row("count-only", blocked.clone(), json!({"min_matches":1}))
    ]);
    let report = results(&f.replay(rows, None), true);
    println!("same-fixture replay={report}");
    let actual = results(
        &f.replay(
            json!([row("empty-deny", near, json!({"decision":"deny"}))]),
            None,
        ),
        false,
    );
    assert_eq!(actual[0]["decision"], "allow");
    assert!(actual[0]["failures"][0]
        .as_str()
        .unwrap()
        .contains("got allow"));
    // Mode changes are isolated child-process regression inputs, never host bypasses.
    for (key, value, decision, native_decision) in [
        ("ARAI_DENY_MODE", "off", "advise", Some("allow")),
        ("ARAI_DISABLED", "1", "allow", None),
    ] {
        let out = f.replay(
            json!([row("mode", blocked.clone(), json!({"decision":decision}))]),
            Some((key, value)),
        );
        assert_eq!(results(&out, true)[0]["decision"], decision);
        let out = f.native(&blocked, Some((key, value)));
        println!(
            "native mode {key}={value} replay={decision} exit={:?} stdout={}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(out.status.code(), Some(0));
        if let Some(expected) = native_decision {
            assert_eq!(
                serde_json::from_slice::<Value>(&out.stdout).unwrap()["hookSpecificOutput"]
                    ["permissionDecision"],
                expected
            );
        } else {
            assert!(out.stdout.is_empty());
        }
    }
    let cfg = f.config();
    let db = Store::open(&cfg.db_path()).unwrap();
    for severity in [Severity::Warn, Severity::Inform] {
        assert!(!db
            .set_severity_override("Alembic", severity)
            .unwrap()
            .is_empty());
        let r = results(
            &f.replay(
                json!([row("demoted", blocked.clone(), json!({"decision":"deny"}))]),
                None,
            ),
            false,
        );
        assert_eq!(r[0]["decision"], "advise");
        assert!(r[0]["failures"][0].as_str().unwrap().contains("got advise"));
        let out = f.native(&blocked, None);
        println!(
            "native demotion {severity:?} replay={} exit={:?} stdout={}",
            r[0]["decision"],
            out.status.code(),
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(
            serde_json::from_slice::<Value>(&out.stdout).unwrap()["hookSpecificOutput"]
                ["permissionDecision"],
            "allow"
        );
    }
}

#[test]
fn repeated_replay_is_read_only_with_existing_session_and_nonpretool_events() {
    let f = Fixture::new();
    let cfg = f.config();
    let db = Store::open(&cfg.db_path()).unwrap();
    let ids: Vec<_> = db
        .load_guardrails()
        .unwrap()
        .iter()
        .map(|g| g.triple_id)
        .collect();
    drop(db);
    // Seed real audit state before measuring the read-only replay boundary.
    assert_eq!(
        f.native(&hook("alembic/versions/001_example.py"), None)
            .status
            .code(),
        Some(0)
    );
    fs::write(
        cfg.arai_base_dir.join("telemetry_queue.jsonl"),
        "preexisting telemetry\n",
    )
    .unwrap();
    fs::write(cfg.arai_base_dir.join(".telemetry_id"), "preexisting-id").unwrap();
    session::record_tool_call(
        &cfg.arai_base_dir,
        "b-session",
        "Bash",
        &["cargo".into(), "test".into()],
    );
    session::mark_rules_seen(&cfg.arai_base_dir, "b-session", &ids);
    fs::create_dir_all(f.0.join("project/.codex")).unwrap();
    fs::write(
        f.0.join("project/.codex/hooks.json"),
        "preexisting host file",
    )
    .unwrap();
    let mut post = hook("alembic/versions/001_example.py");
    post["hook_event_name"] = json!("PostToolUse");
    let mut prompt = hook("alembic/versions/001_example.py");
    prompt["hook_event_name"] = json!("UserPromptSubmit");
    let rows = json!([
        row(
            "seen-still-denies",
            hook("alembic/versions/001_example.py"),
            json!({"decision":"deny"})
        ),
        row("failed", hook("app/example.py"), json!({"decision":"deny"})),
        row("post", post, json!({})),
        row("prompt", prompt, json!({})),
        row("invalid-envelope", json!({}), json!({}))
    ]);
    let path = f.scenario(rows);
    let before = snapshot(&f.0);
    for _ in 0..2 {
        let out = f
            .command(&["test", path.to_str().unwrap(), "--json"])
            .output()
            .unwrap();
        let r = results(&out, false);
        assert_eq!(r[0]["decision"], "deny");
        assert_eq!(r[1]["decision"], "allow");
        for index in [2, 3, 4] {
            assert!(r[index].get("decision").is_none());
        }
        assert!(r
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r.get("hookSpecificOutput").is_none()));
        assert_eq!(snapshot(&f.0), before);
    }
    let path = f.scenario(json!([row(
        "success",
        hook("alembic/versions/001_example.py"),
        json!({"decision":"deny"})
    )]));
    let before = snapshot(&f.0);
    for _ in 0..2 {
        let out = f
            .command(&["test", path.to_str().unwrap(), "--json"])
            .output()
            .unwrap();
        results(&out, true);
        assert_eq!(snapshot(&f.0), before);
    }
}

#[test]
fn aliases_skips_self_commands_and_complete_patches_use_effective_policy() {
    let f = Fixture::new();
    let mut rows = vec![];
    let mut alias = hook("alembic/versions/001_example.py");
    alias.as_object_mut().unwrap().remove("hook_event_name");
    rows.push(row(
        "default-event",
        alias.clone(),
        json!({"decision":"deny"}),
    ));
    alias["hookEventName"] = json!("pre_tool_use");
    rows.push(row("event-alias", alias, json!({"decision":"deny"})));
    rows.push(row(
        "read-skip",
        json!({"tool_name":"Read","tool_input":{"file_path":"alembic/a.py"}}),
        json!({"decision":"allow"}),
    ));
    rows.push(row(
        "self",
        json!({"tool_name":"Bash","tool_input":{"command":"arai why alembic"}}),
        json!({"decision":"allow"}),
    ));
    for patch in ["*** Begin Patch\n*** Add File: app/example.py\n+ok\n*** Add File: alembic/versions/001_example.py\n+bad\n*** End Patch", "*** Begin Patch\n*** Update File: app/example.py\n*** Move to: alembic/versions/001_example.py\n@@\n-old\n+new\n*** End Patch"] {
        let h=json!({"hook_event_name":"pre_tool_use","tool_name":"apply_patch","tool_input":{"command":patch}});
        let native=f.native(&h,None); assert_eq!(native.status.code(),Some(0)); assert_eq!(serde_json::from_slice::<Value>(&native.stdout).unwrap()["hookSpecificOutput"]["permissionDecision"],"deny");
        rows.push(row("compound-patch",h,json!({"decision":"deny"})));
    }
    results(&f.replay(json!(rows), None), true);
}

#[test]
fn prerequisites_scope_inactive_and_missing_intent_preserve_decisions() {
    let f = Fixture::new();
    let cfg = f.config();
    let db = Store::open(&cfg.db_path()).unwrap();
    let mut triple = arai::parser::extract_rules_from_resolved(
        "- Never run git push without running cargo test first\n",
        "claude_md_project",
        0.99,
    )
    .remove(0);
    // Use the existing parser/store path, retaining its missing-intent predicate fallback.
    triple.subject = "git".into();
    triple.predicate = "never".into();
    triple.object = "run git push without running cargo test first".into();
    let path = cfg.project_root.join("CLAUDE.md");
    db.upsert_file(
        path.to_str().unwrap(),
        "claude_md_project",
        &[triple],
        "manual",
    )
    .unwrap();
    // Finish fixture writes before any byte snapshots. Reading database files while
    // holding this writer open can release POSIX locks and let another connection
    // checkpoint its WAL on close, independently of replay's read-only behavior.
    drop(db);
    let h =
        json!({"tool_name":"Bash","tool_input":{"command":"git push"},"session_id":"b-session"});
    assert_eq!(
        results(
            &f.replay(
                json!([row("fallback", h.clone(), json!({"decision":"deny"}))]),
                None
            ),
            true
        )[0]["decision"],
        "deny"
    );
    session::record_tool_call(
        &cfg.arai_base_dir,
        "b-session",
        "Bash",
        &["cargo".into(), "test".into()],
    );
    let file = f.scenario(json!([row(
        "met",
        h,
        json!({"decision":"allow","max_matches":0})
    )]));
    let before = snapshot(&f.0);
    let out = f
        .command(&["test", file.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    results(&out, true);
    assert_eq!(snapshot(&f.0), before);
    let db = Store::open(&cfg.db_path()).unwrap();
    let guard = db.load_guardrails().unwrap().remove(0);
    db.disable_rule(&guard.subject, &guard.predicate, &guard.object)
        .unwrap();
    results(
        &f.replay(
            json!([row(
                "disabled",
                json!({"tool_name":"Bash","tool_input":{"command":"git push"}}),
                json!({"decision":"allow","max_matches":0})
            )]),
            None,
        ),
        true,
    );
    // A nested instruction source is authoritative only within its real source scope.
    let scoped = Fixture::new();
    fs::remove_file(scoped.0.join("project/CLAUDE.md")).unwrap();
    fs::create_dir_all(scoped.0.join("project/nested")).unwrap();
    fs::write(
        scoped.0.join("project/nested/CLAUDE.md"),
        include_bytes!("fixtures/card_a_alembic/CLAUDE.md"),
    )
    .unwrap();
    assert!(scoped.command(&["scan"]).output().unwrap().status.success());
    results(
        &scoped.replay(
            json!([row(
                "out-of-scope",
                hook("alembic/versions/001_example.py"),
                json!({"decision":"allow","max_matches":0})
            )]),
            None,
        ),
        true,
    );
}
