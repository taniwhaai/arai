//! Discovery is a replaceable local snapshot, not ownership of an embedder's store.
use arai::{config::Config, discovery, hooks, init, intent::Severity, parser, store::Store};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

struct Project(PathBuf);
impl Project {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("arai_lifecycle_{}_{stamp}", std::process::id()));
        fs::create_dir_all(root.join("project/.git")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        Self(root)
    }
    fn config(&self) -> Config {
        Config {
            project_root: self.0.join("project"),
            home_dir: self.0.join("home"),
            arai_base_dir: self.0.join("state"),
            extra_sources: vec![],
            guardrails_mode: "advise".into(),
            llm_command: None,
            api_url: None,
            api_key_env: None,
            api_model: None,
        }
    }
    fn write(&self, path: &str, body: &str) -> String {
        let path = self.config().project_root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
        path.to_string_lossy().into_owned()
    }
    fn scan(&self, store: &Store) -> Result<usize, String> {
        init::sync_discovered_rules(store, &discovery::discover(&self.config())?)
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn add_external(store: &Store, path: &str, content: &str, source_type: &str) {
    let triples = parser::extract_rules(content, source_type, 0.95);
    store
        .upsert_file(path, content, &triples, source_type)
        .unwrap();
}

#[test]
fn deleted_discovered_sources_are_pruned_but_external_policy_survives() {
    let p = Project::new();
    let store = Store::open(&p.config().db_path()).unwrap();
    let local = p.write("AGENTS.md", "- Never run cargo clean\n");
    p.scan(&store).unwrap();
    add_external(
        &store,
        "kete://org/verified-policy",
        "- Never execute docker\n",
        "kete",
    );
    add_external(
        &store,
        "manual://arai-add/example",
        "- Never run git push\n",
        "manual",
    );
    fs::remove_file(local).unwrap();
    p.scan(&store).unwrap();
    let paths = store.list_files().unwrap();
    assert_eq!(paths.len(), 2, "{paths:?}");
    assert!(paths.iter().all(|p| p.contains("://")));
    let result = hooks::match_hook(&json!({"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"docker run app"}}), &p.config(), &store).unwrap();
    assert_eq!(hooks::highest_severity(&result.matched), Severity::Block);
}

#[test]
fn public_upsert_transfers_custody_even_when_content_is_unchanged() {
    let p = Project::new();
    let store = Store::open(&p.config().db_path()).unwrap();
    let content = "- Never run cargo clean\n";
    let path = p.write("AGENTS.md", content);
    p.scan(&store).unwrap();
    add_external(&store, &path, content, "agents_md");
    p.write("AGENTS.md", "- Always run cargo build\n");
    p.scan(&store).unwrap();
    assert!(store
        .load_guardrails()
        .unwrap()
        .iter()
        .any(|g| g.object.contains("clean")));
    fs::remove_file(&path).unwrap();
    p.scan(&store).unwrap();
    assert_eq!(store.list_files().unwrap(), vec![path]);
}

#[test]
fn invalid_scope_rolls_back_entire_snapshot_and_preserves_old_policy() {
    let p = Project::new();
    let store = Store::open(&p.config().db_path()).unwrap();
    let old = p.write("AGENTS.md", "- Never run cargo clean\n");
    p.scan(&store).unwrap();
    // Bypass discovery's early validation to exercise the transactional store boundary.
    let malformed = discovery::DiscoveredFile {
        path: p
            .config()
            .project_root
            .join(".claude/rules/bad.md")
            .to_string_lossy()
            .into_owned(),
        source_type: "claude_rules".into(),
        confidence: 0.92,
        content: "---\npaths: [\"[\"]\n---\n- Never run git push\n".into(),
        frontmatter: Default::default(),
    };
    assert!(init::sync_discovered_rules(&store, &[malformed]).is_err());
    assert_eq!(store.list_files().unwrap(), vec![old]);
    assert!(store
        .load_guardrails()
        .unwrap()
        .iter()
        .any(|g| g.object.contains("clean")));
}

#[test]
fn severity_override_survives_unrelated_file_edits() {
    let p = Project::new();
    let store = Store::open(&p.config().db_path()).unwrap();
    p.write("AGENTS.md", "- Never run cargo clean\n");
    p.scan(&store).unwrap();
    assert_eq!(
        store
            .set_severity_override("cargo", Severity::Warn)
            .unwrap()
            .len(),
        1
    );
    p.write(
        "AGENTS.md",
        "# Updated commentary\n\n- Never run cargo clean\n",
    );
    p.scan(&store).unwrap();
    let guards = store.load_guardrails().unwrap();
    assert_eq!(guards.len(), 1);
    assert_eq!(guards[0].intent.as_ref().unwrap().severity, Severity::Warn);
}

#[test]
fn scan_does_not_reclassify_embedding_policy() {
    let p = Project::new();
    let store = Store::open(&p.config().db_path()).unwrap();
    add_external(
        &store,
        "kete://org/exact-bytes",
        "- Never execute docker\n",
        "kete",
    );
    let guard = store.load_guardrails().unwrap().pop().unwrap();
    let mut intent = arai::intent::classify_rule_with_subject(
        &guard.predicate,
        &guard.object,
        Some(&guard.subject),
    );
    intent.severity = Severity::Inform;
    intent.enriched_by = "verified-embedder".into();
    store.upsert_rule_intent(guard.triple_id, &intent).unwrap();
    assert_eq!(
        init::enrich_discovered_rules(&store, &p.config().arai_base_dir).unwrap(),
        0
    );
    assert!(!p.config().arai_base_dir.join("models").exists());
    p.write("AGENTS.md", "- Never run cargo clean\n");
    p.scan(&store).unwrap();
    let actual = store.get_rule_intent(guard.triple_id).unwrap().unwrap();
    assert_eq!(actual.severity, Severity::Inform);
    assert_eq!(actual.enriched_by, "verified-embedder");
    add_external(
        &store,
        "manual://arai-add/new",
        "- Never run git push\n",
        "manual",
    );
    init::classify_source(&store, "manual://arai-add/new").unwrap();
    let actual = store.get_rule_intent(guard.triple_id).unwrap().unwrap();
    assert_eq!(actual.severity, Severity::Inform);
    assert_eq!(actual.enriched_by, "verified-embedder");
}

#[test]
fn opt_in_adopts_only_present_legacy_sources_and_keeps_external_ones() {
    let p = Project::new();
    let cfg = p.config();
    let store = Store::open(&cfg.db_path()).unwrap();
    let path = p.write("AGENTS.md", "- Never run cargo clean\n");
    add_external(&store, &path, "- Never run git push\n", "agents_md");
    add_external(
        &store,
        "missing/CLAUDE.md",
        "- Never execute docker\n",
        "claude_md_project",
    );
    drop(store);
    let conn = rusqlite::Connection::open(cfg.db_path()).unwrap();
    conn.execute_batch("DROP TABLE source_metadata; PRAGMA user_version=4;")
        .unwrap();
    drop(conn);
    let store = Store::open(&cfg.db_path()).unwrap();
    let external_path = p.write("CLAUDE.md", "- Always run npm test\n");
    add_external(
        &store,
        &external_path,
        "- Never run npm publish\n",
        "claude_md_project",
    );
    let files = discovery::discover(&cfg).unwrap();
    init::sync_discovered_rules_with_adoption(&store, &files, true).unwrap();
    assert!(store
        .rules_for_file(&path)
        .unwrap()
        .iter()
        .any(|g| g.object.contains("clean")));
    assert!(store
        .rules_for_file(&external_path)
        .unwrap()
        .iter()
        .any(|g| g.object.contains("publish")));
    fs::remove_file(&path).unwrap();
    p.scan(&store).unwrap();
    assert!(!store.list_files().unwrap().contains(&path));
    assert!(store
        .list_files()
        .unwrap()
        .contains(&"missing/CLAUDE.md".to_string()));
    assert!(store.list_files().unwrap().contains(&external_path));
}

#[test]
fn discovery_preserves_upstream_provenance() {
    let p = Project::new();
    let store = Store::open(&p.config().db_path()).unwrap();
    let file = discovery::DiscoveredFile {
        path:p.config().project_root.join("AGENTS.md").to_string_lossy().into_owned(),
        source_type:"agents_md".into(),confidence:0.92,frontmatter:Default::default(),
        content:"<!-- arai:extends-block url=\"https://example.invalid/org.md\" tier=\"strict\" -->\n- Never run git push\n\n<!-- end arai:extends -->\n\n- Never run cargo clean\n".into()
    };
    init::sync_discovered_rules(&store, &[file]).unwrap();
    let guards = store.load_guardrails().unwrap();
    let upstream = guards.iter().find(|g| g.object.contains("push")).unwrap();
    assert_eq!(upstream.tier, Some(arai::extends::Tier::Strict));
    assert_eq!(
        upstream.source_label.as_deref(),
        Some("https://example.invalid/org.md")
    );
    let local = guards.iter().find(|g| g.object.contains("clean")).unwrap();
    assert!(local.tier.is_none());
    assert!(local.source_label.is_none());
}

#[test]
fn corrupt_stored_scope_does_not_allow_a_tool_call() {
    let p = Project::new();
    let cfg = p.config();
    let store = Store::open(&cfg.db_path()).unwrap();
    p.write("AGENTS.md", "- Never run cargo clean\n");
    p.scan(&store).unwrap();
    let conn = rusqlite::Connection::open(cfg.db_path()).unwrap();
    conn.execute("UPDATE source_metadata SET scope_json='invalid'", [])
        .unwrap();
    let result = hooks::match_hook(
        &json!({"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo clean"}}),
        &cfg,
        &store,
    );
    assert!(result.is_err());
}
