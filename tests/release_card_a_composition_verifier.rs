//! Independent release-card-a-composition v1 verification. No implementor tests read.
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
    env: BTreeMap<String, String>,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("arai-parent-verifier-{label}-{id}"));
        let root = base.join("project");
        let home = base.join("home");
        let state = base.join("state");
        for path in [root.join(".git"), home.clone(), state.clone()] {
            fs::create_dir_all(path).unwrap();
        }
        let mut env = BTreeMap::new();
        for name in [
            "PATH",
            "SystemRoot",
            "WINDIR",
            "PATHEXT",
            "TEMP",
            "TMP",
            "COMSPEC",
        ] {
            if let Ok(value) = std::env::var(name) {
                env.insert(name.to_string(), value);
            }
        }
        for (name, value) in [
            ("HOME", home.clone()),
            ("USERPROFILE", home.clone()),
            ("APPDATA", home.join("AppData/Roaming")),
            ("LOCALAPPDATA", home.join("AppData/Local")),
            ("ARAI_BASE_DIR", state),
        ] {
            fs::create_dir_all(&value).unwrap();
            env.insert(name.to_string(), value.to_string_lossy().into_owned());
        }
        env.insert("DO_NOT_TRACK".into(), "1".into());
        env.insert("NO_COLOR".into(), "1".into());
        Self { root, env }
    }

    fn put(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn run(&self, label: &str, args: &[&str], input: Option<&Value>) -> Output {
        let binary = env!("CARGO_BIN_EXE_arai");
        let mut child = Command::new(binary)
            .args(args)
            .current_dir(&self.root)
            .env_clear()
            .envs(&self.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let payload = input.map(|v| serde_json::to_vec(v).unwrap());
        if let Some(bytes) = &payload {
            child.stdin.take().unwrap().write_all(bytes).unwrap();
        } else {
            drop(child.stdin.take());
        }
        let output = child.wait_with_output().unwrap();
        if let Ok(log) = std::env::var("ARAI_VERIFIER_LOG_DIR") {
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let stem = PathBuf::from(log).join(format!("{sequence:03}-{label}"));
            fs::write(stem.with_extension("stdout"), &output.stdout).unwrap();
            fs::write(stem.with_extension("stderr"), &output.stderr).unwrap();
            if let Some(bytes) = &payload {
                fs::write(stem.with_extension("stdin.json"), bytes).unwrap();
            }
            fs::write(
                stem.with_extension("record.json"),
                serde_json::to_vec_pretty(&json!({
                    "binary": binary, "args": args, "cwd": self.root,
                    "environment": self.env, "environment_policy": "env_clear then exact recorded map; no host identities or inherited Arai/API configuration",
                    "exit": output.status.code(), "fixture_retained": true,
                })).unwrap(),
            ).unwrap();
        }
        output
    }
}

fn text(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn ok(output: &Output) -> String {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        text(output),
        String::from_utf8_lossy(&output.stderr)
    );
    text(output)
}

fn summary(output: &str) -> Vec<String> {
    let start = output
        .find("Effective policy summary")
        .expect("missing summary");
    output[start..]
        .lines()
        .take(9)
        .map(|s| s.trim().to_owned())
        .collect()
}

fn inventory(f: &Fixture) -> Value {
    serde_json::from_str(&ok(&f.run("inventory", &["guardrails", "--json"], None))).unwrap()
}

fn native(f: &Fixture, path: &str) -> Value {
    json!({"hook_event_name":"PreToolUse", "tool_name":"Write", "tool_input":{"file_path":path,"content":""}, "session_id":"", "cwd": f.root})
}

#[test]
fn c1_exact_dependency_surface_compiles() {
    // C1: Compile the canonical records and seams; historical noninspection is separately assessed.
    let _: fn(
        &arai::config::Config,
        &arai::store::Store,
    ) -> Result<arai::init::EnforceabilitySummary, String> = arai::init::enforceability_summary;
    let _: fn(&arai::init::EnforceabilitySummary) = arai::init::render_enforceability_summary;
    let f = Fixture::new("seams");
    let cfg = arai::config::Config {
        project_root: f.root.clone(),
        home_dir: PathBuf::from(&f.env["HOME"]),
        arai_base_dir: PathBuf::from(&f.env["ARAI_BASE_DIR"]),
        extra_sources: vec![],
        guardrails_mode: "advise".into(),
        llm_command: None,
        api_url: None,
        api_key_env: None,
        api_model: None,
    };
    let mut outcomes: Vec<arai::init::HostRegistrationOutcome> = Vec::new();
    arai::init::register_default_hosts(&cfg, |record| outcomes.push(record.clone())).unwrap();
    assert_eq!(outcomes.len(), 5);
    assert!(outcomes
        .iter()
        .all(|o| o.state == arai::init::HostRegistrationState::Registered));
}

#[test]
fn c2_summary_parity_sources_scope_modes_and_stored_status() {
    // C2: Same final state yields equal summaries; extraction counts stay separate; scope is conditional.
    let mut f = Fixture::new("parity");
    f.put(
        "CLAUDE.md",
        "## Alembic\n\n- Never hand-write migration files\n- Never force-push to main\n",
    );
    f.put(
        ".claude/rules/scoped.md",
        "---\npaths:\n  - \"src/**/*.py\"\n---\n## Alembic\n\n- Never hand-write migration files\n",
    );
    for (mode, value, expected) in [
        ("normal", None, "deny enabled"),
        (
            "advisory",
            Some(("ARAI_DENY_MODE", "off")),
            "advisory only (deny disabled)",
        ),
        (
            "bypass",
            Some(("ARAI_DISABLED", "1")),
            "hard bypass (no enforcement)",
        ),
    ] {
        // Explicit isolated process mode probes authorized by context; checkout hooks remain active.
        f.env.remove("ARAI_DENY_MODE");
        f.env.remove("ARAI_DISABLED");
        if let Some((key, value)) = value {
            f.env.insert(key.into(), value.into());
        }
        let init = ok(&f.run(&format!("{mode}-init"), &["init"], None));
        let scan = ok(&f.run(&format!("{mode}-scan"), &["scan"], None));
        let status = ok(&f.run(&format!("{mode}-status"), &["status"], None));
        assert_eq!(summary(&init), summary(&scan));
        assert_eq!(summary(&scan), summary(&status));
        assert!(scan.contains("Extracted: 3 rule(s) from 2 file(s)"));
        assert!(scan.contains("Stored sources: 2"));
        assert!(scan.contains("Stored actionable rules: 3"));
        assert!(scan.contains(&format!("Runtime mode: {expected}")));
        assert!(scan.contains("Conditional paths: src/**/*.py"));
        assert!(scan.contains("Conditional directory:"));
        assert!(scan.contains("CLAUDE.md:3"));
        assert!(scan.contains(
            "Eligibility is conditional on scope and the actual tool call; it is not a match."
        ));
        if mode == "normal" {
            assert!(scan.contains("Block eligible: 2"));
            assert!(scan.contains("Documentary/inert: 1"));
        } else {
            assert!(scan.contains("Block eligible: 0"));
        }
        if mode == "advisory" {
            assert!(scan.contains("Advise eligible: 2"));
        }
        if mode == "bypass" {
            assert!(scan.contains("Advise eligible: 0"));
            assert!(scan.contains("Inactive: 3"));
        }
    }
    f.env.remove("ARAI_DISABLED");
    let before = summary(&ok(&f.run("stored-before", &["status"], None)));
    f.put("CLAUDE.md", "# Changed on disk\n");
    assert_eq!(
        before,
        summary(&ok(&f.run("stored-after", &["status"], None)))
    );
    let hook = f.root.join(".codex/hooks.json");
    fs::remove_file(&hook).unwrap();
    let status = ok(&f.run("no-repair", &["status"], None));
    assert!(!hook.exists());
    assert!(status.contains(".codex/hooks.json (config missing)"));
}

#[test]
fn c2_empty_inert_and_requested_failure_do_not_emit_stale_success() {
    // C2/C6: Empty/inert success; requested enrichment failure exits one without a premature summary.
    for (label, prose, inert) in [
        ("empty", "", 0),
        ("inert", "- Never force-push to main\n", 1),
    ] {
        let f = Fixture::new(label);
        if !prose.is_empty() {
            f.put("CLAUDE.md", prose);
        }
        for command in ["init", "scan", "status"] {
            let output = ok(&f.run(&format!("{label}-{command}"), &[command], None));
            assert!(output.contains("Block eligible: 0"));
            assert!(output.contains(&format!("Documentary/inert: {inert}")));
        }
        let failure = f.run(
            "missing-enrichment",
            &["scan", "--enrich-file", "absent.json"],
            None,
        );
        assert_eq!(failure.status.code(), Some(1));
        assert!(!text(&failure).contains("Effective policy summary"));
        assert!(String::from_utf8_lossy(&failure.stderr).contains("arai:"));
    }
}

#[test]
fn c3_optional_skip_mandatory_stop_and_trust_are_observable() {
    // C3: Real init must continue only after optional Grok errors, preserve diagnostics/earlier writes.
    let optional = Fixture::new("optional");
    optional.put(".grok/hooks/arai.json", "{broken");
    let output = optional.run("grok-skip", &["init"], None);
    let stdout = ok(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Grok registration skipped"));
    assert!(stdout.contains("Copilot registered"));
    assert!(optional.root.join(".github/hooks/arai.json").is_file());
    assert_eq!(
        fs::read_to_string(optional.root.join(".grok/hooks/arai.json")).unwrap(),
        "{broken"
    );
    assert!(stdout.contains("writes do not establish trust or hook execution"));
    assert!(!stdout.contains("All five"));
    assert!(stdout.contains("/hooks"));
    let mandatory = Fixture::new("mandatory");
    mandatory.put(".codex/hooks.json", "{broken");
    let output = mandatory.run("codex-stop", &["init"], None);
    assert_eq!(output.status.code(), Some(1));
    let stdout = text(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Codex registration failed"));
    let lines: Vec<_> = stderr.lines().collect();
    let returned = lines.iter().find_map(|s| s.strip_prefix("arai: ")).unwrap();
    assert!(stderr
        .lines()
        .find(|s| s.contains("registration failed"))
        .unwrap()
        .ends_with(returned));
    assert!(mandatory.root.join(".claude/settings.json").is_file());
    for path in [
        ".cursor/hooks.json",
        ".grok/hooks/arai.json",
        ".github/hooks/arai.json",
    ] {
        assert!(!mandatory.root.join(path).exists());
    }
    assert!(!stdout.contains("attempts completed"));
    assert!(!stdout.contains("Cursor registered"));
}

#[test]
fn c2_successful_import_precedes_the_only_final_summary() {
    // C2: A requested successful import changes eligibility before the sole summary is rendered.
    let f = Fixture::new("final-import");
    f.put(
        "CLAUDE.md",
        "## Alembic\n\n- Never hand-write migration files\n",
    );
    let before = ok(&f.run("import-before", &["scan"], None));
    assert!(before.contains("Block eligible: 1"));
    let rows = inventory(&f);
    let id = rows[0]["triple_id"].as_i64().unwrap();
    let input = json!([{"id":id,"action":"general","timing":"principle","tools":["*"]}]);
    f.put("import.json", serde_json::to_vec_pretty(&input).unwrap());
    let imported = ok(&f.run(
        "import-after",
        &["scan", "--enrich-file", "import.json"],
        None,
    ));
    assert!(imported.contains("1 rules imported"));
    assert_eq!(imported.matches("Effective policy summary").count(), 1);
    assert!(!imported.contains("Block eligible: 1"));
    assert!(imported.contains("Block eligible: 0"));
    assert!(imported.contains("Documentary/inert: 1"));
    assert!(
        imported.find("rules imported").unwrap()
            < imported.find("Effective policy summary").unwrap()
    );
    assert_eq!(
        summary(&imported),
        summary(&ok(&f.run("import-status", &["status"], None)))
    );
}

#[test]
fn c2_frozen_exact_archived_bytes_reproduce_twenty_two_inert_rules() {
    // C2: Frozen baseline identity is three exact archived instruction blobs, never the changing checkout.
    use sha2::{Digest, Sha256};
    const BASE: &str = "3609164161dc5334c3adf94976b417cc32376cdf";
    let f = Fixture::new("frozen");
    let mut evidence = Vec::new();
    for (path, expected) in [
        (
            "CLAUDE.md",
            "FA752284BF2D055BE4AFF5F423DD5CC900CE47F3E08D4FA552C2C7502A88247D",
        ),
        (
            "AGENTS.md",
            "35F70EA23244AECCF451CFA786108C957C57BF0AC00378F4D13D4EDECA5CCFC8",
        ),
        (
            ".taniwha/kupu/orchestrator/handoff/01KSGB5549QPQDQMSH6B8NS077/outputs/files/CLAUDE.md",
            "B0E34F81E8AD8566D82ED67FD21A4CAA47170E8667AF5C9F40F67C2467592224",
        ),
    ] {
        // These checked-in bytes were archived from BASE:path. The digest pins their
        // provenance without requiring git history or a host-specific worktree pointer.
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/card_a_baseline")
            .join(path);
        let bytes = fs::read(&fixture).unwrap();
        let hash: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect();
        assert_eq!(hash, expected);
        evidence.push(json!({"archived_from":format!("{BASE}:{path}"),"checked_in_fixture":fixture,"relative_path":path,"sha256":hash,"bytes":bytes.len()}));
        f.put(path, bytes);
    }
    if let Ok(log) = std::env::var("ARAI_VERIFIER_LOG_DIR") {
        fs::write(PathBuf::from(log).join("frozen-inputs.json"), serde_json::to_vec_pretty(&json!({"baseline":BASE,"fixture":f.root,"inputs":evidence,"scope":"archived discovered instruction inputs only; no historical handoff/design/manifest read"})).unwrap()).unwrap();
    }
    let scan = ok(&f.run("frozen-scan", &["scan"], None));
    for needle in [
        "Extracted: 22 rule(s) from 3 file(s)",
        "Stored sources: 3",
        "Stored actionable rules: 22",
        "Block eligible: 0",
        "Advise eligible: 0",
        "Documentary/inert: 22",
        "Inactive: 0",
    ] {
        assert!(scan.contains(needle), "{needle}: {scan}");
    }
    let rows = inventory(&f);
    assert_eq!(rows.as_array().unwrap().len(), 22);
    assert_eq!(
        summary(&scan),
        summary(&ok(&f.run("frozen-status", &["status"], None)))
    );
}

#[test]
fn c4_exact_fresh_alembic_native_and_checked_scenario() {
    // C4: Exact checked-in bytes, native deny/reason/transport, silent unrelated control, every checked case.
    let f = Fixture::new("alembic");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let bytes = fs::read(repository.join("tests/fixtures/card_a_alembic/CLAUDE.md")).unwrap();
    assert_eq!(
        String::from_utf8(bytes.clone())
            .unwrap()
            .replace("\r\n", "\n"),
        "## Alembic\n\n- Never hand-write migration files\n"
    );
    f.put("CLAUDE.md", bytes);
    ok(&f.run("alembic-scan", &["scan"], None));
    let denied = f.run(
        "alembic-deny",
        &["guardrails", "--match-stdin"],
        Some(&native(&f, "alembic/versions/001_example.py")),
    );
    assert_eq!(denied.status.code(), Some(0));
    let response: Value = serde_json::from_slice(&denied.stdout).unwrap();
    assert_eq!(response["hookSpecificOutput"]["permissionDecision"], "deny");
    let reason = response["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap();
    assert!(reason.contains("Alembic"));
    assert!(reason.contains("CLAUDE.md:3"));
    let allowed = f.run(
        "alembic-control",
        &["guardrails", "--match-stdin"],
        Some(&native(&f, "app/example.py")),
    );
    assert_eq!(allowed.status.code(), Some(0));
    assert!(allowed.stdout.is_empty());
    let scenario = repository.join("scenarios/alembic-migration.json");
    let result = ok(&f.run(
        "checked-scenario",
        &["test", scenario.to_str().unwrap(), "--json"],
        None,
    ));
    let cases: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 3);
    assert!(cases
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["passed"] == true));
    f.put("CLAUDE.md", "- Never force-push to main\n");
    let scanned = ok(&f.run("inert-regression-scan", &["scan"], None));
    assert!(scanned.contains("Documentary/inert: 1"));
    let hook = json!({"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git push --force origin main"},"cwd":f.root,"session_id":""});
    let inert = f.run(
        "inert-native",
        &["guardrails", "--match-stdin"],
        Some(&hook),
    );
    assert_eq!(inert.status.code(), Some(0));
    assert!(inert.stdout.is_empty());
}

#[test]
fn c5_guidance_and_public_help_are_consistent() {
    // C5: Owned docs/help teach the proven fixture, supported install routes and evidence limits.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for path in ["README.md", "site/index.html", "docs/rule-testing.md"] {
        let doc = fs::read_to_string(root.join(path)).unwrap();
        for needle in [
            "Alembic",
            "Never hand-write migration files",
            "app/example.py",
            "scope",
        ] {
            assert!(doc.contains(needle), "{path}: {needle}");
        }
        assert!(doc.contains("replay"));
        assert!(doc.contains("trust"));
        assert!(doc.contains("inert"));
    }
    for path in ["README.md", "site/index.html"] {
        let doc = fs::read_to_string(root.join(path)).unwrap();
        for needle in [
            "https://arai.taniwha.ai/install",
            "npm install -g @taniwhaai/arai",
            "cargo install arai",
            "ARM64",
            "x64",
            "sign-in",
        ] {
            assert!(doc.contains(needle), "{path}: {needle}");
        }
        assert!(!doc.contains("install.ps1"));
    }
    let f = Fixture::new("guidance");
    let help = ok(&f.run("add-help", &["add", "--help"], None));
    assert!(help.contains("Never hand-write alembic migration files"));
    assert!(!help.contains("Never force-push"));
    f.put("empty.md", "Plain descriptive paragraph.\n");
    let lint = ok(&f.run("lint-empty", &["lint", "empty.md"], None));
    assert!(lint.contains("Never hand-write alembic migration files"));
    let add = f.run(
        "add-unextractable",
        &["add", "Plain descriptive paragraph."],
        None,
    );
    assert_eq!(add.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&add.stderr).contains("Never hand-write alembic migration files")
    );
}

#[test]
fn c6_json_scenario_errors_and_readonly_status_contracts() {
    // C6: Existing JSON fields/metadata and ordinary exits remain; status cannot repair.
    let f = Fixture::new("compatibility");
    f.put(
        "CLAUDE.md",
        "## Alembic\n\n- Never hand-write migration files\n",
    );
    ok(&f.run("compat-scan", &["scan"], None));
    let rows = inventory(&f);
    let row = &rows.as_array().unwrap()[0];
    for field in [
        "triple_id",
        "subject",
        "predicate",
        "object",
        "intent",
        "source_file",
        "line_start",
    ] {
        assert!(row.get(field).is_some(), "{field}");
    }
    let status = ok(&f.run("compat-status", &["status"], None));
    for needle in [
        "config missing",
        "Last scan:",
        "Last firing: none recorded",
        "Code graph:",
        "Host trust and hook activation",
    ] {
        assert!(status.contains(needle));
    }
    assert!(!f.root.join(".claude/settings.json").exists());
    f.put("permissive.json", serde_json::to_vec(&json!({"unrecognized":true,"scenarios":[{"name":"empty expect with unknown","hook":native(&f,"app/example.py"),"expect":{"future_field":true}}]})).unwrap());
    // Card B intentionally supersedes A's permissive assertion-key behavior.
    let invalid = f.run(
        "unknown-assertion",
        &["test", "permissive.json", "--json"],
        None,
    );
    assert_eq!(invalid.status.code(), Some(1));
    assert!(invalid.stdout.is_empty());
    let diagnostic = String::from_utf8_lossy(&invalid.stderr);
    assert!(
        diagnostic.contains("empty expect with unknown") && diagnostic.contains("future_field")
    );
    f.put("metadata.json", serde_json::to_vec(&json!({"_comment":"accepted metadata","unrecognized":true,"scenarios":[{"name":"empty expect","hook":native(&f,"app/example.py"),"expect":{}}]})).unwrap());
    let result = ok(&f.run("metadata", &["test", "metadata.json", "--json"], None));
    let results: Value = serde_json::from_str(&result).unwrap();
    for field in ["name", "passed", "failures", "matched_subjects"] {
        assert!(results[0].get(field).is_some());
    }
    assert_eq!(results[0]["passed"], true);
    f.put("wrong-case.json", serde_json::to_vec(&json!({"scenarios":[{"name":"case sensitive","hook":native(&f,"alembic/versions/001_example.py"),"expect":{"matches_subject":["alembic"]}}]})).unwrap());
    let failure = f.run(
        "case-sensitive-failure",
        &["test", "wrong-case.json", "--json"],
        None,
    );
    assert_eq!(failure.status.code(), Some(1));
    let failure = f.run("missing-scenario", &["test", "absent.json"], None);
    assert_eq!(failure.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&failure.stderr).contains("arai:"));
}

#[test]
fn c7_exact_gate_record_is_external_to_behavioral_tests() {
    // C7: The independent runner records seven exact gates, focused suites and source identity outside this test.
    // This test checks only our isolation discipline; the report additionally evaluates the runner results.
    let f = Fixture::new("isolation");
    assert!(!f
        .env
        .keys()
        .any(|k| k.starts_with("CODEX") || k.starts_with("CLAUDE") || k.starts_with("GROK")));
    assert!(!f.env.contains_key("ARAI_DISABLED"));
    assert!(!f.env.contains_key("ARAI_DENY_MODE"));
    assert!(Path::new(&f.env["ARAI_BASE_DIR"]).is_dir());
}
