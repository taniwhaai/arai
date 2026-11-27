//! Independent Card B contract tests. These exercise public APIs and CLI only.
use arai::{config::Config, hooks, intent::Severity, scenarios, session, store::Store};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let s =
            Self(std::env::temp_dir().join(format!("b-independent-{}-{id}", std::process::id())));
        for p in ["repo/.git", "home", "policy"] {
            fs::create_dir_all(s.0.join(p)).unwrap();
        }
        fs::write(
            s.0.join("repo/CLAUDE.md"),
            b"## Alembic\n\n- Never hand-write migration files\n",
        )
        .unwrap();
        success(&s.cmd(&["scan"]).output().unwrap());
        s
    }
    fn cfg(&self) -> Config {
        Config {
            project_root: self.0.join("repo"),
            home_dir: self.0.join("home"),
            arai_base_dir: self.0.join("policy"),
            extra_sources: vec![],
            guardrails_mode: "inject".into(),
            llm_command: None,
            api_url: None,
            api_key_env: None,
            api_model: None,
        }
    }
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_arai"));
        c.current_dir(self.0.join("repo")).args(args);
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy().to_ascii_uppercase();
            if ["ARAI_", "CLAUDE_", "CODEX_", "GROK_", "CURSOR_", "COPILOT_"]
                .iter()
                .any(|p| name.starts_with(p))
                || ["OPENAI_API_KEY", "ANTHROPIC_API_KEY"].contains(&name.as_str())
            {
                c.env_remove(key);
            }
        }
        c.env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("ARAI_BASE_DIR", self.0.join("policy"))
            .env("DO_NOT_TRACK", "1");
        c
    }
    fn file(&self, label: &str, doc: Value) -> PathBuf {
        let p = self.0.join(format!("{label}.json"));
        fs::write(&p, serde_json::to_vec(&doc).unwrap()).unwrap();
        p
    }
    fn replay_file(&self, p: &Path, env: &[(&str, &str)]) -> Output {
        let out = self
            .cmd(&["test", p.to_str().unwrap(), "--json"])
            .envs(env.iter().copied())
            .output()
            .unwrap();
        capture("replay", &out);
        out
    }
    fn replay(&self, rows: Value, env: &[(&str, &str)]) -> Output {
        self.replay_file(
            &self.file("input", json!({"_comment":"independent", "scenarios":rows})),
            env,
        )
    }
    fn native(&self, h: &Value, env: &[(&str, &str)]) -> Output {
        let mut payload = h.clone();
        payload["cwd"] = json!(self.0.join("repo"));
        println!("native stdin: {payload}");
        let mut child = self
            .cmd(&["guardrails", "--match-stdin"])
            .envs(env.iter().copied())
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
        capture("native", &out);
        out
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn capture(label: &str, out: &Output) {
    println!(
        "{label} exit={:?}\nstdout={}\nstderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
fn success(out: &Output) {
    assert!(out.status.success(), "{out:?}");
}
fn rows(out: &Output, ok: bool) -> Value {
    assert_eq!(out.status.code(), Some(if ok { 0 } else { 1 }), "{out:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}
fn write_hook(path: &str) -> Value {
    json!({"hook_event_name":"PreToolUse","tool_name":"Write","tool_input":{"file_path":path,"content":""},"session_id":"verify-b"})
}
fn scenario(name: &str, hook: Value, expect: Value) -> Value {
    json!({"name":name,"hook":hook,"expect":expect})
}
fn migration() -> Value {
    write_hook("alembic/versions/001_example.py")
}
fn decision(s: &Sandbox, h: Value, expected: &str, env: &[(&str, &str)]) {
    let r = rows(
        &s.replay(
            json!([scenario(expected, h, json!({"decision":expected}))]),
            env,
        ),
        true,
    );
    assert_eq!(r[0]["decision"], expected);
}
fn tree(at: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, at: &Path, result: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for e in fs::read_dir(at).unwrap() {
            let p = e.unwrap().path();
            let key = p.strip_prefix(root).unwrap().to_path_buf();
            if p.is_dir() {
                result.insert(key, None);
                visit(root, &p, result);
            } else {
                result.insert(key, Some(fs::read(p).unwrap()));
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(at, at, &mut result);
    result
}

#[test]
fn b1_strict_whole_file_validation_and_legacy_shapes() {
    // B1: Invalid assertions fail before any row runs; metadata/defaults remain valid.
    let s = Sandbox::new();
    let invalid = vec![
        json!({"decsion":"deny"}),
        json!({"rule_ids":["x"]}),
        json!({"other":false}),
        json!({"decision":"Deny"}),
        json!({"decision":"block"}),
        json!({"decision":" deny"}),
        json!({"decision":null}),
        json!({"decision":7}),
        json!({"decision":true}),
        json!({"decision":[]}),
        json!({"min_matches":-1}),
        json!({"max_matches":1.5}),
        json!({"min_matches":"1"}),
        json!({"matches_subject":null}),
        json!({"does_not_match_subject":[9]}),
        json!({"matches_subject":"Alembic"}),
        json!([]),
        json!(null),
        json!(false),
    ];
    for expect in invalid {
        let p = s.file("invalid", json!({"scenarios":[scenario("would-fail-evaluation",json!(null),json!({})),scenario("bad-assertion",migration(),expect)]}));
        let out = s.replay_file(&p, &[]);
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("bad-assertion") || err.contains(p.to_str().unwrap()),
            "{err}"
        );
    }
    for event in [
        "PostToolUse",
        "post_tool_use",
        "UserPromptSubmit",
        "user_prompt_submit",
        "Stop",
        "SessionStart",
        "FileChanged",
        "PermissionDenied",
    ] {
        let mut h = migration();
        h["hook_event_name"] = json!(event);
        let out = s.replay(
            json!([scenario("unsupported", h, json!({"decision":"allow"}))]),
            &[],
        );
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        assert!(String::from_utf8_lossy(&out.stderr).contains("unsupported"));
    }
    for doc in [
        json!([]),
        json!({"scenarios":null}),
        json!({"scenarios":[["array",migration(),{}]]}),
    ] {
        let out = s.replay_file(&s.file("shape", doc), &[]);
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
    }
    for doc in [
        json!({"_comment":"metadata","future_metadata":true}),
        json!({"scenarios":[]}),
        json!({"_comment":{},"scenarios":[{"name":"omitted","hook":migration()},scenario("null-bounds",migration(),json!({"min_matches":null,"max_matches":null}))]}),
    ] {
        let out = s.replay_file(&s.file("valid", doc), &[]);
        rows(&out, true);
    }
}

#[test]
fn b2_ordered_matrix_combined_assertions_and_real_override() {
    // B2: Empty-deny mismatches, near miss/count checks and real override demotion.
    let s = Sandbox::new();
    let matrix = json!([
        scenario(
            "empty-deny",
            write_hook("app/example.py"),
            json!({"decision":"deny"})
        ),
        scenario(
            "block",
            migration(),
            json!({"decision":"deny","matches_subject":["Alembic"],"does_not_match_subject":["alembic"],"min_matches":1,"max_matches":1})
        ),
        scenario(
            "near",
            write_hook("app/example.py"),
            json!({"decision":"allow","max_matches":0})
        ),
        scenario("old-count", migration(), json!({"min_matches":1}))
    ]);
    let r = rows(&s.replay(matrix, &[]), false);
    assert_eq!(
        r.as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["empty-deny", "block", "near", "old-count"]
    );
    assert_eq!(r[0]["decision"], "allow");
    assert_eq!(r[0]["passed"], false);
    assert!(r[0]["failures"][0].as_str().unwrap().contains("got allow"));
    for i in 1..4 {
        assert_eq!(r[i]["passed"], true);
    }
    let db = Store::open(&s.cfg().db_path()).unwrap();
    db.set_severity_override("Alembic", Severity::Warn).unwrap();
    let r = rows(
        &s.replay(
            json!([scenario(
                "demoted",
                migration(),
                json!({"decision":"deny","min_matches":1})
            )]),
            &[],
        ),
        false,
    );
    assert_eq!(r[0]["decision"], "advise");
    assert!(r[0]["failures"][0].as_str().unwrap().contains("got advise"));
    let r = rows(
        &s.replay(
            json!([scenario(
                "case",
                migration(),
                json!({"decision":"advise","matches_subject":["alembic"]})
            )]),
            &[],
        ),
        false,
    );
    assert_eq!(r[0]["passed"], false);
}

#[test]
fn b3_modes_severity_skips_scope_inactive_and_seen() {
    // B3: Effective severity/modes/suppression determine decisions; seen stays active.
    let s = Sandbox::new();
    decision(&s, migration(), "deny", &[]);
    for value in ["off", "0", "FALSE", "No"] {
        decision(&s, migration(), "advise", &[("ARAI_DENY_MODE", value)]);
    }
    for value in ["1", "TRUE", "on", "Yes"] {
        decision(&s, migration(), "allow", &[("ARAI_DISABLED", value)]);
    }
    decision(
        &s,
        migration(),
        "deny",
        &[("ARAI_DENY_MODE", "on"), ("ARAI_DISABLED", "off")],
    );
    let mut read = migration();
    read["tool_name"] = json!("Read");
    decision(&s, read, "allow", &[]);
    decision(
        &s,
        json!({"tool_name":"Bash","tool_input":{"command":"arai why alembic"}}),
        "allow",
        &[],
    );
    decision(&s, write_hook("app/example.py"), "allow", &[]);
    let cfg = s.cfg();
    let db = Store::open(&cfg.db_path()).unwrap();
    let guards = db.load_guardrails().unwrap();
    session::mark_rules_seen(
        &cfg.arai_base_dir,
        "verify-b",
        &guards.iter().map(|g| g.triple_id).collect::<Vec<_>>(),
    );
    decision(&s, migration(), "deny", &[]);
    for severity in [Severity::Warn, Severity::Inform, Severity::Block] {
        db.set_severity_override("Alembic", severity).unwrap();
        decision(
            &s,
            migration(),
            if severity == Severity::Block {
                "deny"
            } else {
                "advise"
            },
            &[],
        );
    }
    let g = &guards[0];
    db.disable_rule(&g.subject, &g.predicate, &g.object)
        .unwrap();
    decision(&s, migration(), "allow", &[]);
    let nested = Sandbox::new();
    fs::remove_file(nested.0.join("repo/CLAUDE.md")).unwrap();
    fs::create_dir_all(nested.0.join("repo/service")).unwrap();
    fs::write(
        nested.0.join("repo/service/CLAUDE.md"),
        b"## Alembic\n\n- Never hand-write migration files\n",
    )
    .unwrap();
    success(&nested.cmd(&["scan"]).output().unwrap());
    decision(&nested, migration(), "allow", &[]);
    decision(
        &nested,
        write_hook("service/alembic/versions/001_example.py"),
        "deny",
        &[],
    );
}

#[test]
fn b4_native_protocol_parity_aliases_and_compound_destinations() {
    // B4: Native policy and replay agree; protocol and exit are separate assertions.
    let s = Sandbox::new();
    for (h, expected) in [
        (migration(), "deny"),
        (write_hook("app/example.py"), "allow"),
    ] {
        decision(&s, h.clone(), expected, &[]);
        let out = s.native(&h, &[]);
        assert_eq!(out.status.code(), Some(0));
        if expected == "deny" {
            let v: Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
            assert!(String::from_utf8_lossy(&out.stdout).contains("CLAUDE.md:3"));
        } else {
            assert!(out.stdout.is_empty());
        }
    }
    for mode in [Some(("ARAI_DENY_MODE", "off")), None] {
        if mode.is_none() {
            Store::open(&s.cfg().db_path())
                .unwrap()
                .set_severity_override("Alembic", Severity::Warn)
                .unwrap();
        }
        let env: Vec<_> = mode.into_iter().collect();
        decision(&s, migration(), "advise", &env);
        let out = s.native(&migration(), &env);
        assert_eq!(out.status.code(), Some(0));
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
        assert!(v["hookSpecificOutput"]["additionalContext"].is_string());
    }
    Store::open(&s.cfg().db_path())
        .unwrap()
        .set_severity_override("Alembic", Severity::Block)
        .unwrap();
    let mut absent = migration();
    absent.as_object_mut().unwrap().remove("hook_event_name");
    decision(&s, absent, "deny", &[]);
    let alias = json!({"hookEventName":"pre_tool_use","toolName":"Write","toolInput":{"file_path":"alembic/versions/001_example.py"}});
    decision(&s, alias, "deny", &[]);
    for patch in ["*** Begin Patch\n*** Add File: app/safe.py\n+x\n*** Add File: alembic/versions/001_example.py\n+x\n*** End Patch",
        "*** Begin Patch\n*** Update File: app/safe.py\n@@\n-old\n+new\n*** Update File: app/from.py\n*** Move to: alembic/versions/001_example.py\n@@\n-old\n+new\n*** End Patch"] {
        let h=json!({"hook_event_name":"PreToolUse","tool_name":"apply_patch","tool_input":{"command":patch}});
        decision(&s,h.clone(),"deny",&[]);let out=s.native(&h,&[]);assert_eq!(out.status.code(),Some(0));
        assert_eq!(serde_json::from_slice::<Value>(&out.stdout).unwrap()["hookSpecificOutput"]["permissionDecision"],"deny");
    }
}

#[test]
fn b5_repeat_replay_preserves_all_state_and_reads_prerequisites() {
    // B5 (+ B3 fallback): Byte/membership snapshots include real audit/seen/session state.
    let s = Sandbox::new();
    success(&s.native(&migration(), &[]));
    let cfg = s.cfg();
    session::record_tool_call(
        &cfg.arai_base_dir,
        "verify-b",
        "Bash",
        &["cargo".into(), "test".into()],
    );
    fs::write(
        cfg.arai_base_dir.join("telemetry_queue.jsonl"),
        b"sentinel\n",
    )
    .unwrap();
    fs::write(cfg.arai_base_dir.join(".telemetry_id"), b"sentinel-id").unwrap();
    fs::create_dir_all(s.0.join("repo/.codex")).unwrap();
    fs::write(s.0.join("repo/.codex/hooks.json"), b"host-sentinel").unwrap();
    let mut post = migration();
    post["hook_event_name"] = json!("PostToolUse");
    let good=s.file("good",json!({"scenarios":[scenario("seen-still-denies",migration(),json!({"decision":"deny"})),scenario("post",post,json!({})),scenario("prompt",json!({"hook_event_name":"UserPromptSubmit","prompt":"alembic"}),json!({}))]}));
    let bad=s.file("bad",json!({"scenarios":[scenario("assertion-fail",migration(),json!({"decision":"allow"})),scenario("evaluation-fail",json!(null),json!({}))]}));
    let invalid = s.file(
        "invalid-no-effects",
        json!({"scenarios":[scenario("unknown",migration(),json!({"rule_ids":[]}))]}),
    );
    let before = tree(&s.0);
    assert!(before.keys().any(|p| p.to_string_lossy().contains("audit")));
    assert!(before
        .values()
        .filter_map(|v| v.as_ref())
        .any(|v| String::from_utf8_lossy(v).contains("seen_rules")));
    for _ in 0..3 {
        let r = rows(&s.replay_file(&good, &[]), true);
        assert_eq!(r[0]["decision"], "deny");
        assert!(r[1].get("decision").is_none());
        assert!(r[2].get("decision").is_none());
        let r = rows(&s.replay_file(&bad, &[]), false);
        assert_eq!(r[0]["decision"], "deny");
        assert!(r[1].get("decision").is_none());
        for row in r.as_array().unwrap() {
            assert!(row.get("hookSpecificOutput").is_none());
        }
        assert_eq!(s.replay_file(&invalid, &[]).status.code(), Some(1));
        assert_eq!(tree(&s.0), before);
    }
    let p = Sandbox::new();
    let cfg = p.cfg();
    let db = Store::open(&cfg.db_path()).unwrap();
    let mut t = arai::parser::extract_rules_from_resolved(
        "- Never run git push without running cargo test first\n",
        "claude_md_project",
        0.99,
    )
    .remove(0);
    t.subject = "git".into();
    t.predicate = "never".into();
    t.object = "run git push without running cargo test first".into();
    db.upsert_file(
        cfg.project_root.join("CLAUDE.md").to_str().unwrap(),
        "claude_md_project",
        &[t],
        "verifier-fallback",
    )
    .unwrap();
    assert!(db
        .load_guardrails()
        .unwrap()
        .iter()
        .all(|g| g.intent.is_none()));
    // Quiesce fixture writes before replay and byte snapshots; an open writer can
    // otherwise cause a POSIX WAL checkpoint when another connection closes.
    drop(db);
    let h = json!({"tool_name":"Bash","tool_input":{"command":"git push"},"session_id":"prereq-b"});
    decision(&p, h.clone(), "deny", &[]);
    session::record_tool_call(
        &cfg.arai_base_dir,
        "prereq-b",
        "Bash",
        &["cargo".into(), "test".into()],
    );
    let file = p.file(
        "met",
        json!({"scenarios":[scenario("met",h,json!({"decision":"allow","max_matches":0}))]}),
    );
    let before = tree(&p.0);
    for _ in 0..2 {
        rows(&p.replay_file(&file, &[]), true);
        assert_eq!(tree(&p.0), before);
    }
}

#[test]
fn b6_exact_checked_in_fixture_and_ci_contract() {
    // B6: Exact fresh bytes/path/decisions and explicit two-OS CI invocations.
    assert_eq!(
        include_bytes!("fixtures/card_a_alembic/CLAUDE.md"),
        b"## Alembic\n\n- Never hand-write migration files\n"
    );
    let s = Sandbox::new();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios/alembic-migration.json");
    let r = rows(&s.replay_file(&path, &[]), true);
    assert_eq!(r.as_array().unwrap().len(), 3);
    assert_eq!(r[0]["decision"], "deny");
    assert_eq!(r[1]["decision"], "allow");
    assert_eq!(r[2]["decision"], "allow");
    let ci = include_str!("../.github/workflows/ci.yml");
    assert!(ci.contains("Fresh Alembic quick-start decision regression (Linux)"));
    assert!(ci.contains("Fresh Alembic quick-start decision regression (Windows)"));
    assert_eq!(ci.matches("cargo test --locked --test card_a_commands exact_alembic_native_denial_allowed_control_and_current_scenario").count(),2);
}

#[test]
fn b7_documented_schema_and_projection_help() {
    // B7: Public help and guidance identify projection/replay limits consistently.
    let s = Sandbox::new();
    for command in ["canonicalize", "sync"] {
        let out = s.cmd(&[command, "--help"]).output().unwrap();
        success(&out);
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("native"));
        assert!(text.contains("prose") || text.contains("projection"));
    }
    let guide = include_str!("../docs/rule-testing.md");
    for term in [
        "case-sensitive",
        "rule_ids",
        "decision: null",
        "whole file",
        "nonzero",
        "seen-rule",
        "transport",
        "host trust",
        "ARAI_DENY_MODE",
        "ARAI_DISABLED",
    ] {
        assert!(guide.contains(term), "{term}");
    }
    let spec = include_str!("../docs/rules-file-spec.md");
    for term in [
        "projection data",
        "no native glob execution",
        "no native regex execution",
        "no most-specific",
        "No stable authored audit-ID guarantee",
    ] {
        assert!(spec.contains(term), "{term}");
    }
    assert!(include_str!("../README.md").contains("allow for the unrelated Write and skipped Read"));
}

#[test]
fn b8_public_construction_and_error_distinction() {
    // B8: Legacy public construction/signatures compile and evaluation errors lack decision.
    let s = Sandbox::new();
    let cfg = s.cfg();
    let db = Store::open(&cfg.db_path()).unwrap();
    let expect = scenarios::Expect {
        matches_subject: vec![],
        does_not_match_subject: vec![],
        min_matches: None,
        max_matches: None,
    };
    let scenario = scenarios::Scenario {
        name: "public".into(),
        hook: migration(),
        expect,
    };
    let _file = scenarios::ScenarioFile {
        scenarios: vec![scenario],
    };
    let _result = scenarios::ScenarioResult {
        name: "public".into(),
        passed: true,
        failures: vec![],
        matched_subjects: vec![],
    };
    let _match = hooks::HookMatch {
        event: "PreToolUse".into(),
        tool_name: "Write".into(),
        session_id: String::new(),
        terms: vec![],
        matched: vec![],
        skipped: false,
        is_prompt_summary: false,
        domain_rules: vec![],
    };
    let _guard = arai::store::Guardrail {
        triple_id: 1,
        subject: "x".into(),
        predicate: "never".into(),
        object: "x".into(),
        confidence: 1.0,
        source_file: "x".into(),
        file_path: "x".into(),
        layer: None,
        line_start: None,
        expires_at: None,
        noenrich: false,
        intent: None,
        tier: None,
        source_label: None,
    };
    let matched = hooks::match_hook(&migration(), &cfg, &db).unwrap();
    assert!(!matched.matched.is_empty());
    let p = s.file(
        "public-call",
        json!({"scenarios":[{"name":"public","hook":migration()}]}),
    );
    assert!(scenarios::run_file(&p, &cfg, &db).unwrap()[0].passed);
    let r=rows(&s.replay(json!([{"name":"error","hook":{"tool_name":"Write","tool_input":{}},"expect":{"decision":"deny"}}]),&[]),false);
    assert_eq!(r[0]["passed"], false);
    assert!(r[0].get("decision").is_none());
    let out = s.native(
        &json!({"hook_event_name":"PreToolUse","tool_name":"Write","tool_input":{}}),
        &[],
    );
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["hookSpecificOutput"]
            ["permissionDecision"],
        "deny"
    );
    let fresh = Sandbox::new();
    fs::remove_dir_all(fresh.0.join("policy")).unwrap();
    fs::create_dir_all(fresh.0.join("policy")).unwrap();
    let out = fresh.replay(json!([{"name":"missing-store","hook":migration()}]), &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("database"));
}
