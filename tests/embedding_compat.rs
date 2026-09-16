//! Public-library contracts exercised by Kete's composition spike.
//! Keep this independent of CLI-only modules, global CWD changes and network services.
use arai::{audit, config::Config, hooks, intent::Severity, parser, store::Store};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const KETE_SPIKE_RULES: &str = "# Project instructions\n\
- Never run destructive git commands like `git push --force` on shared branches.\n\
- Always run the test suite before committing.\n";

struct Project(PathBuf);

impl Project {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("arai_embedding_{}_{stamp}", std::process::id()));
        fs::create_dir_all(root.join("project/.git")).unwrap();
        Self(root)
    }

    fn config(&self) -> Config {
        Config {
            project_root: self.0.join("project"),
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
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn force_push() -> Value {
    json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "session_id": "spike-session",
        "tool_input": {"command": "git push --force origin main"}
    })
}

#[test]
fn kete_spike_pipeline_remains_a_public_offline_composition() {
    let project = Project::new();
    let cfg = project.config();
    let db = Store::open(&cfg.db_path()).unwrap();
    let triples = parser::extract_rules(KETE_SPIKE_RULES, "CLAUDE.md", 0.9);
    assert!(!triples.is_empty());
    assert!(db
        .upsert_file("CLAUDE.md", KETE_SPIKE_RULES, &triples, "CLAUDE.md")
        .unwrap());

    // The real downstream spike does not classify first. Legacy stores must
    // still enforce prohibitions through the shared severity fallback.
    let result = hooks::match_hook(&force_push(), &cfg, &db).unwrap();
    assert_eq!(result.event, "PreToolUse");
    assert_eq!(result.tool_name, "Bash");
    assert_eq!(result.session_id, "spike-session");
    assert!(!result.skipped);
    assert!(!result.matched.is_empty());
    assert!(result.matched.iter().all(|(g, _)| g.intent.is_none()));
    assert_eq!(hooks::highest_severity(&result.matched), Severity::Block);
    assert!(!cfg.arai_base_dir.join("audit").exists());
    assert!(!cfg.arai_base_dir.join("sessions").exists());

    // The embedder explicitly decides when evidence is recorded; match_hook
    // must not append a second event or invoke the CLI's shipping path.
    audit::record_firing(
        &cfg,
        &result.event,
        &result.tool_name,
        &result.session_id,
        "git push --force origin main",
        &result.matched,
        "deny",
        Some(&db),
        &HashSet::new(),
    );
    let buckets = audit::list_buckets(&cfg.arai_base_dir, &cfg.project_slug()).unwrap();
    assert_eq!(buckets.len(), 1);
    let bucket = &buckets[0];
    let raw = bucket.jsonl_bytes().unwrap();
    assert_eq!(raw, fs::read(&bucket.jsonl_path).unwrap());
    assert_eq!(raw.len() as u64, bucket.bytes);
    let lines: Vec<_> = raw
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(lines.len(), 1);
    let event: Value = serde_json::from_slice(lines[0]).unwrap();
    assert_eq!(event["decision"], "deny");
    assert_eq!(event["hash"].as_str().unwrap(), bucket.head);
    assert!(event["rules"]
        .as_array()
        .unwrap()
        .iter()
        .any(|rule| { rule["severity"] == "block" && rule["source"] == "CLAUDE.md" }));
    assert!(audit::verify_chain(&cfg.arai_base_dir, &cfg.project_slug())
        .unwrap()
        .is_empty());
}

#[test]
fn embedder_uses_shared_severity_after_classification_and_override() {
    let project = Project::new();
    let cfg = project.config();
    let db = Store::open(&cfg.db_path()).unwrap();
    let triples = parser::extract_rules(KETE_SPIKE_RULES, "CLAUDE.md", 0.9);
    db.upsert_file("CLAUDE.md", KETE_SPIKE_RULES, &triples, "CLAUDE.md")
        .unwrap();
    db.classify_all_guardrails().unwrap();
    let before = hooks::match_hook(&force_push(), &cfg, &db).unwrap();
    assert_eq!(hooks::highest_severity(&before.matched), Severity::Block);

    assert!(!db
        .set_severity_override("git", Severity::Warn)
        .unwrap()
        .is_empty());
    let after = hooks::match_hook(&force_push(), &cfg, &db).unwrap();
    assert!(!after.matched.is_empty());
    assert!(after.matched.iter().any(|(g, _)| g.predicate == "never"));
    assert_eq!(hooks::highest_severity(&after.matched), Severity::Warn);
    assert_eq!(hooks::highest_severity(&[]), Severity::Inform);
}

#[test]
fn explicit_project_configuration_never_changes_process_cwd() {
    let project = Project::new();
    let second = Project::new();
    fs::create_dir_all(project.0.join("project/src/nested")).unwrap();
    let before = std::env::current_dir().unwrap();
    let first_cfg = Config::load_from(&project.0.join("project/src/nested")).unwrap();
    let second_cfg = Config::load_from(&second.0.join("project")).unwrap();
    assert_eq!(std::env::current_dir().unwrap(), before);
    assert_eq!(first_cfg.project_root, project.0.join("project"));
    assert_eq!(second_cfg.project_root, second.0.join("project"));
    assert_ne!(first_cfg.project_slug(), second_cfg.project_slug());
}

#[test]
fn supplied_policy_bytes_and_provenance_survive_without_a_source_file() {
    let project = Project::new();
    let cfg = project.config();
    let db = Store::open(&cfg.db_path()).unwrap();
    let source = "kete:binding/project/policy-7";
    let label = "kete:binding/project/bundle-7";
    // Signature/scope/expiry verification belongs to the caller. Parsing this
    // already-accepted body neither fetches its URL nor grants it extra trust.
    let body = "---\narai:extends: https://policy.invalid/rules.md\n---\n\
- Never run destructive git commands like `git push --force` on shared branches.\n";
    let triples =
        parser::extract_rules_with_provenance(body, "bound_policy", 0.9, None, Some(label.into()));
    assert_eq!(triples.len(), 1);
    assert!(db
        .upsert_file(source, body, &triples, "bound_policy")
        .unwrap());
    assert!(!db
        .upsert_file(source, body, &triples, "bound_policy")
        .unwrap());
    db.classify_all_guardrails().unwrap();
    drop(db);

    // A new process can reopen the accepted local store while the original
    // instruction file and the policy server are both absent.
    let db = Store::open(&cfg.db_path()).unwrap();
    let result = hooks::match_hook(&force_push(), &cfg, &db).unwrap();
    assert_eq!(hooks::highest_severity(&result.matched), Severity::Block);
    assert_eq!(result.matched.len(), 1);
    let rule = &result.matched[0].0;
    assert_eq!(rule.file_path, source);
    assert_eq!(rule.source_label.as_deref(), Some(label));
    assert_eq!(rule.line_start, triples[0].line_start);
    assert_eq!(db.list_files().unwrap(), vec![source.to_string()]);
    assert!(!cfg.arai_base_dir.join("audit").exists());
}

#[test]
fn upgrading_a_v4_store_does_not_adopt_legacy_external_policy() {
    let project = Project::new();
    let cfg = project.config();
    fs::create_dir_all(&cfg.home_dir).unwrap();
    let path = cfg.project_root.join("AGENTS.md");
    let source = path.to_string_lossy();
    let accepted =
        "- Never run destructive git commands like `git push --force` on shared branches.\n";
    let triples = parser::extract_rules(accepted, "agents_md", 0.95);
    let db = Store::open(&cfg.db_path()).unwrap();
    db.upsert_file(&source, accepted, &triples, "agents_md")
        .unwrap();
    db.classify_all_guardrails().unwrap();
    drop(db);

    // Simulate the actual pre-upgrade schema, which had no custody metadata.
    // Its public caller could use exactly the same path and source namespace
    // as CLI discovery; the next version must not guess who owns this policy.
    let connection = rusqlite::Connection::open(cfg.db_path()).unwrap();
    connection
        .execute_batch("DROP TABLE source_metadata; PRAGMA user_version = 4;")
        .unwrap();
    drop(connection);
    fs::write(&path, "- Always run cargo test.\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_arai"))
        .arg("scan")
        .current_dir(&cfg.project_root)
        .env("HOME", &cfg.home_dir)
        .env("USERPROFILE", &cfg.home_dir)
        .env("ARAI_BASE_DIR", &cfg.arai_base_dir)
        .env("ARAI_TELEMETRY", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let db = Store::open(&cfg.db_path()).unwrap();
    let result = hooks::match_hook(&force_push(), &cfg, &db).unwrap();
    assert_eq!(hooks::highest_severity(&result.matched), Severity::Block);
    assert!(db
        .rules_for_file(&source)
        .unwrap()
        .iter()
        .any(|rule| rule.object.contains("push --force")));
    assert!(!db
        .rules_for_file(&source)
        .unwrap()
        .iter()
        .any(|rule| rule.object.contains("cargo test")));
}
