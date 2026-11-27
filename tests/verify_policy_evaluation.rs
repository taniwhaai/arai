//! Independent verification of policy-evaluation v1. No implementor tests used.
use arai::config::Config;
use arai::init::{
    enforceability_summary, EnforceabilityClass as Class, EnforceabilitySummary,
    InactivityReason as Reason,
};
use arai::intent::{Action, RuleIntent, Severity, Timing};
use arai::parser::Triple;
use arai::store::Store;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn isolated(cmd: &mut Command, root: &Path) {
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy().to_ascii_uppercase();
        if ["ARAI_", "CLAUDE_", "GROK_", "CURSOR_", "COPILOT_", "CODEX_"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
            || ["ANTHROPIC_API_KEY", "OPENAI_API_KEY"].contains(&name.as_str())
        {
            cmd.env_remove(key);
        }
    }
    let home = root.join("home");
    let project = root.join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(project.join(".git")).unwrap();
    cmd.current_dir(project)
        .env("HOME", &home)
        .env("USERPROFILE", home)
        .env("ARAI_BASE_DIR", root.join("state"))
        .env("DO_NOT_TRACK", "1");
}

fn child(case: &str, deny: Option<&str>, bypass: Option<&str>, mode: &str) {
    let root = std::env::temp_dir().join(format!(
        "arai-policy-verifier-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    isolated(&mut cmd, &root);
    cmd.args(["--exact", "policy_verifier_worker", "--nocapture"])
        .env("POLICY_VERIFIER_CASE", case)
        .env("POLICY_VERIFIER_ROOT", &root)
        .env("POLICY_VERIFIER_CONFIG_MODE", mode);
    if let Some(value) = deny {
        cmd.env("ARAI_DENY_MODE", value);
    }
    if let Some(value) = bypass {
        cmd.env("ARAI_DISABLED", value);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "case={case} deny={deny:?} bypass={bypass:?} config={mode}\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    print!("{}", String::from_utf8_lossy(&out.stdout));
    std::fs::remove_dir_all(root).unwrap();
}

fn config(root: &Path) -> Config {
    // P6: the existing public embedding record remains directly constructible.
    Config {
        project_root: root.join("project"),
        home_dir: root.join("home"),
        arai_base_dir: root.join("state"),
        extra_sources: vec![],
        guardrails_mode: std::env::var("POLICY_VERIFIER_CONFIG_MODE").unwrap(),
        llm_command: None,
        api_url: None,
        api_key_env: None,
        api_model: None,
    }
}

fn triple(name: &str, predicate: &str) -> Triple {
    // Construct through the supported parser, then supply the fixture's policy.
    let mut rows = arai::parser::extract_rules_from_resolved(
        include_str!("fixtures/card_a_baseline/CLAUDE.md"),
        "claude_md_project",
        0.99,
    );
    let mut row = rows.remove(0);
    row.subject = name.into();
    row.predicate = predicate.into();
    row.object = "create sentinel secrets files".into();
    row.line_start = Some(12);
    row.line_end = Some(14);
    // P6: exhaustively reconstruct the existing supported public Triple shape.
    let Triple {
        subject,
        predicate,
        object,
        confidence,
        domain,
        source_file,
        line_start,
        line_end,
        layer,
        expires_at,
        noenrich,
        tier,
        source_label,
    } = row;
    Triple {
        subject,
        predicate,
        object,
        confidence,
        domain,
        source_file,
        line_start,
        line_end,
        layer,
        expires_at,
        noenrich,
        tier,
        source_label,
    }
}

fn insert(db: &Store, path: &str, row: &Triple, severity: Option<Severity>, timing: Timing) -> i64 {
    db.upsert_file(path, path, std::slice::from_ref(row), "manual")
        .unwrap();
    let id = db.rules_for_file(path).unwrap()[0].triple_id;
    if let Some(severity) = severity {
        db.upsert_rule_intent(
            id,
            &RuleIntent {
                action: Action::Create,
                timing,
                tools: vec!["Write".into()],
                allow_inverse: false,
                enriched_by: "independent-verifier".into(),
                severity,
            },
        )
        .unwrap();
    }
    id
}

fn partition(s: &EnforceabilitySummary, expected: [usize; 5]) {
    assert_eq!(
        [
            s.stored_actionable_rule_count,
            s.block_eligible_count,
            s.advise_eligible_count,
            s.documentary_inert_count,
            s.inactive_count,
        ],
        expected
    );
    assert_eq!(s.rules.len(), s.stored_actionable_rule_count);
    assert_eq!(
        s.block_eligible_count
            + s.advise_eligible_count
            + s.documentary_inert_count
            + s.inactive_count,
        s.stored_actionable_rule_count
    );
    assert_eq!(
        s.rules
            .iter()
            .map(|r| r.triple_id)
            .collect::<HashSet<_>>()
            .len(),
        s.rules.len()
    );
    for (class, count) in [
        (Class::BlockEligible, s.block_eligible_count),
        (Class::AdviseEligible, s.advise_eligible_count),
        (Class::DocumentaryInert, s.documentary_inert_count),
        (Class::Inactive, s.inactive_count),
    ] {
        assert_eq!(
            s.rules.iter().filter(|r| r.classification == class).count(),
            count
        );
    }
}

// P1: zero population and each non-hook timing succeed with complete explanations.
#[test]
fn p1_empty_and_all_documentary() {
    child("p1", None, None, "advise");
}

// P2: assert 22 only against hash-pinned archived instruction bytes.
#[test]
fn p2_frozen_three_source_population() {
    child("p2", None, None, "advise");
}

// P3: all seven predicates, exclusions, severity overrides and scope/provenance.
#[test]
fn p3_mixed_partition_effective_state_and_provenance() {
    child("p3", None, None, "advise");
}

// P4: every named spelling, casing, defaults and near-misses in fresh processes.
#[test]
fn p4_isolated_environment_truth_table() {
    for mode in ["advise", "enforce", "arbitrary"] {
        child("p4", None, None, mode);
        for value in ["off", "0", "false", "no", "OfF", "FALSE", "NO"] {
            child("p4", Some(value), None, mode);
        }
        for value in ["1", "true", "on", "yes", "TrUe", "ON", "YES"] {
            child("p4", None, Some(value), mode);
            child("p4", Some("off"), Some(value), mode);
        }
        for value in ["", " off", "false ", "disabled", "yes", "ON"] {
            child("p4", Some(value), None, mode);
        }
        for value in ["", "0", "false", "off", "no", " true", "yes ", "enabled"] {
            child("p4", None, Some(value), mode);
        }
    }
}

// P5: corrupt inventory/scope diagnostics propagate, no successful zero or writes.
#[test]
fn p5_read_failures_and_read_only_observations() {
    child("p5", None, None, "advise");
}

// P6: public API and native runtime shapes remain valid before parent wiring.
#[test]
fn p6_embedding_and_native_mode_compatibility() {
    child("p6", None, None, "advise");
}

#[test]
fn policy_verifier_worker() {
    let Ok(case) = std::env::var("POLICY_VERIFIER_CASE") else {
        return;
    };
    let root = PathBuf::from(std::env::var_os("POLICY_VERIFIER_ROOT").unwrap());
    let cfg = config(&root);
    let db = Store::open(&cfg.db_path()).unwrap();
    match case.as_str() {
        "p1" => {
            let empty = enforceability_summary(&cfg, &db).unwrap();
            partition(&empty, [0, 0, 0, 0, 0]);
            assert_eq!(empty.stored_source_count, 0);
            for (name, timing) in [
                ("start", Timing::Start),
                ("stop", Timing::Stop),
                ("principle", Timing::Principle),
            ] {
                insert(
                    &db,
                    name,
                    &triple(name, "never"),
                    Some(Severity::Block),
                    timing,
                );
            }
            db.upsert_file("empty-external", "", &[], "manual").unwrap();
            let s = enforceability_summary(&cfg, &db).unwrap();
            partition(&s, [3, 0, 0, 3, 0]);
            assert_eq!(s.stored_source_count, 4);
            assert!(s
                .rules
                .iter()
                .all(|r| r.effective_severity == "block" && r.inactive_reasons.is_empty()));
        }
        "p2" => frozen(&cfg, &db),
        "p3" => mixed(&cfg, &db),
        "p4" => modes(&cfg, &db),
        "p5" => failures(&cfg, &db),
        "p6" => native(&root, &cfg, &db),
        _ => panic!("unknown verifier case"),
    }
}

fn frozen(cfg: &Config, db: &Store) {
    let nested =
        ".taniwha/kupu/orchestrator/handoff/01KSGB5549QPQDQMSH6B8NS077/outputs/files/CLAUDE.md";
    for (relative, hash) in [
        (
            "CLAUDE.md",
            "FA752284BF2D055BE4AFF5F423DD5CC900CE47F3E08D4FA552C2C7502A88247D",
        ),
        (
            "AGENTS.md",
            "35F70EA23244AECCF451CFA786108C957C57BF0AC00378F4D13D4EDECA5CCFC8",
        ),
        (
            nested,
            "B0E34F81E8AD8566D82ED67FD21A4CAA47170E8667AF5C9F40F67C2467592224",
        ),
    ] {
        let bytes = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/card_a_baseline")
                .join(relative),
        )
        .unwrap();
        assert_eq!(
            Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<String>(),
            hash
        );
        let destination = cfg.project_root.join(relative);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(destination, bytes).unwrap();
    }
    let files = arai::discovery::discover(cfg).unwrap();
    assert_eq!(files.len(), 3);
    assert_eq!(arai::init::sync_discovered_rules(db, &files).unwrap(), 22);
    let s = enforceability_summary(cfg, db).unwrap();
    partition(&s, [22, 0, 0, 22, 0]);
    assert_eq!(s.stored_source_count, 3);
    let mut timings = BTreeMap::new();
    for source in db.list_files().unwrap() {
        let rows = db.rules_for_file(&source).unwrap();
        assert_eq!(
            rows.len(),
            if source.ends_with("AGENTS.md") { 20 } else { 1 }
        );
        let mut lines = vec![];
        for row in rows {
            let intent = row.intent.unwrap();
            assert!(arai::intent::is_inert(&intent));
            *timings
                .entry(intent.timing.as_str().to_owned())
                .or_insert(0) += 1;
            let explanation = s
                .rules
                .iter()
                .find(|r| r.triple_id == row.triple_id)
                .unwrap();
            assert_eq!(explanation.source_path.as_deref(), Some(source.as_str()));
            assert_eq!(explanation.line_start, row.line_start.map(|n| n as usize));
            lines.push(explanation.line_start.unwrap());
        }
        lines.sort_unstable();
        if source.ends_with("AGENTS.md") {
            assert_eq!(
                lines,
                [9, 10, 11, 16, 17, 19, 20, 21, 26, 28, 32, 33, 39, 44, 52, 58, 59, 61, 62, 63]
            );
        } else {
            assert_eq!(
                lines,
                [if source.replace('\\', "/").ends_with(nested) {
                    101
                } else {
                    121
                }]
            );
        }
    }
    assert_eq!(
        timings,
        BTreeMap::from([
            ("principle".into(), 18),
            ("start".into(), 3),
            ("stop".into(), 1)
        ])
    );
}

fn mixed(cfg: &Config, db: &Store) {
    let sql = Connection::open(cfg.db_path()).unwrap();
    let dates: (String, String) = sql
        .query_row("SELECT date('now','-1 day'), date('now')", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    let mut expected = BTreeMap::new();
    // Explicitly include all seven predicates without reclassifying absent intent.
    for (name, pred, severity) in [
        ("fallback-forbids", "forbids", "block"),
        ("fallback-must-not", "must_not", "block"),
        ("fallback-never", "never", "block"),
        ("fallback-always", "always", "warn"),
        ("fallback-requires", "requires", "warn"),
        ("fallback-enforces", "enforces", "warn"),
        ("fallback-prefers", "prefers", "inform"),
    ] {
        let id = insert(db, name, &triple(name, pred), None, Timing::ToolCall);
        expected.insert(id, severity);
        assert!(db.get_rule_intent(id).unwrap().is_none());
    }
    for (name, severity) in [
        ("classified-block", Severity::Block),
        ("classified-warn", Severity::Warn),
        ("classified-inform", Severity::Inform),
    ] {
        let id = insert(
            db,
            name,
            &triple(name, "prefers"),
            Some(severity),
            Timing::ToolCall,
        );
        expected.insert(id, severity.as_str());
    }
    let demoted = insert(
        db,
        "demoted",
        &triple("demoted", "never"),
        Some(Severity::Block),
        Timing::ToolCall,
    );
    db.set_severity_override("demoted", Severity::Inform)
        .unwrap();
    expected.insert(demoted, "inform");
    let promoted = insert(
        db,
        "promoted",
        &triple("promoted", "prefers"),
        Some(Severity::Inform),
        Timing::ToolCall,
    );
    db.set_severity_override("promoted", Severity::Block)
        .unwrap();
    expected.insert(promoted, "block");
    let duplicate = triple("disabled-duplicate", "never");
    let dup_a = insert(db, "duplicate-a", &duplicate, None, Timing::ToolCall);
    let dup_b = insert(db, "duplicate-b", &duplicate, None, Timing::ToolCall);
    db.disable_rule(&duplicate.subject, &duplicate.predicate, &duplicate.object)
        .unwrap();
    let mut yesterday = triple("yesterday", "never");
    yesterday.expires_at = Some(dates.0.clone());
    let expired = insert(db, "expired", &yesterday, None, Timing::ToolCall);
    let mut today = triple("today", "never");
    today.expires_at = Some(dates.1);
    let active_today = insert(db, "today", &today, None, Timing::ToolCall);
    expected.insert(active_today, "block");
    let mut overlap = triple("overlap", "never");
    overlap.expires_at = Some(dates.0);
    let overlap_path = "nested/.cursor/rules/manual.mdc";
    db.upsert_file(
        overlap_path,
        "---\ndescription: manual\nalwaysApply: false\n---\n",
        &[overlap.clone()],
        "cursor_rules",
    )
    .unwrap();
    let overlap_id = db.rules_for_file(overlap_path).unwrap()[0].triple_id;
    db.disable_rule(&overlap.subject, &overlap.predicate, &overlap.object)
        .unwrap();
    let inactive_path = ".claude/rules/empty.md";
    db.upsert_file(
        inactive_path,
        "---\npaths: []\n---\n",
        &[triple("permanent", "never")],
        "claude_rules",
    )
    .unwrap();
    let permanent = db.rules_for_file(inactive_path).unwrap()[0].triple_id;
    let mut scoped_ids = vec![];
    for (path, kind, content, directory, patterns) in [
        ("AGENTS.md", "agents_md", "root", "", vec![]),
        ("pkg/AGENTS.md", "agents_md", "nested", "pkg", vec![]),
        (
            "pkg/.claude/rules/files.md",
            "claude_rules",
            "---\npaths: [\"src/**/*.rs\", \"*.toml\"]\n---\n",
            "pkg",
            vec!["src/**/*.rs", "*.toml"],
        ),
    ] {
        db.upsert_file(path, content, &[triple(path, "never")], kind)
            .unwrap();
        let id = db.rules_for_file(path).unwrap()[0].triple_id;
        expected.insert(id, "block");
        scoped_ids.push((id, directory, patterns));
    }
    let missing_scope = insert(
        db,
        "legacy",
        &triple("legacy", "never"),
        None,
        Timing::ToolCall,
    );
    sql.execute(
        "DELETE FROM source_metadata WHERE file_id=(SELECT id FROM files WHERE path='legacy')",
        [],
    )
    .unwrap();
    expected.insert(missing_scope, "block");
    let missing_provenance = insert(
        db,
        "",
        &triple("unknown-origin", "always"),
        None,
        Timing::ToolCall,
    );
    sql.execute(
        "UPDATE triples SET source_file='',line_start=NULL,line_end=NULL WHERE id=?1",
        [missing_provenance],
    )
    .unwrap();
    expected.insert(missing_provenance, "warn");
    sql.execute(
        "UPDATE triples SET source_file='legacy-original.md' WHERE id=?1",
        [active_today],
    )
    .unwrap();
    db.upsert_file(
        "only-nonactionable",
        "nonaction",
        &[triple("nonaction", "mentions")],
        "manual",
    )
    .unwrap();
    db.upsert_file("zero-row-external", "empty", &[], "manual")
        .unwrap();
    let s = enforceability_summary(cfg, db).unwrap();
    partition(&s, [23, 10, 8, 0, 5]);
    assert_eq!(s.stored_source_count, 25);
    assert_eq!(s, enforceability_summary(cfg, db).unwrap());
    for (id, severity) in expected {
        let row = s.rules.iter().find(|r| r.triple_id == id).unwrap();
        assert_eq!(row.effective_severity, severity);
        assert_eq!(
            row.classification,
            if severity == "block" {
                Class::BlockEligible
            } else {
                Class::AdviseEligible
            }
        );
        assert!(row.inactive_reasons.is_empty());
    }
    for (id, reasons) in [
        (dup_a, vec![Reason::Disabled]),
        (dup_b, vec![Reason::Disabled]),
        (expired, vec![Reason::Expired]),
        (permanent, vec![Reason::PermanentSourceInactive]),
        (
            overlap_id,
            vec![
                Reason::Disabled,
                Reason::Expired,
                Reason::PermanentSourceInactive,
            ],
        ),
    ] {
        let row = s.rules.iter().find(|r| r.triple_id == id).unwrap();
        assert_eq!(row.classification, Class::Inactive);
        assert_eq!(row.inactive_reasons.len(), reasons.len());
        for reason in reasons {
            assert!(row.inactive_reasons.contains(&reason));
        }
    }
    let overlap_row = s.rules.iter().find(|r| r.triple_id == overlap_id).unwrap();
    assert_eq!(overlap_row.source_inactive_reason.as_deref(), Some("Cursor manual/agent-selected rule is not automatically enforced; set alwaysApply: true or globs for deterministic activation"));
    for (id, directory, patterns) in scoped_ids {
        let row = s.rules.iter().find(|r| r.triple_id == id).unwrap();
        assert_eq!(row.scope_directory.as_deref(), Some(directory));
        assert_eq!(row.scope_patterns, patterns);
        assert_eq!((row.line_start, row.line_end), (Some(12), Some(14)));
    }
    let legacy = s
        .rules
        .iter()
        .find(|r| r.triple_id == missing_scope)
        .unwrap();
    assert_eq!(legacy.scope_directory, None);
    assert!(legacy.scope_patterns.is_empty());
    let absent = s
        .rules
        .iter()
        .find(|r| r.triple_id == missing_provenance)
        .unwrap();
    assert_eq!(
        (&absent.source_path, absent.line_start, absent.line_end),
        (&None, None, None)
    );
    assert_eq!(
        s.rules
            .iter()
            .find(|r| r.triple_id == active_today)
            .unwrap()
            .source_path
            .as_deref(),
        Some("today")
    );
    // Summary cannot attach intents as a side effect.
    assert!(db.get_rule_intent(missing_scope).unwrap().is_none());
    // P3: a legacy original path remains usable when the attached path is absent.
    sql.execute(
        "UPDATE triples SET source_file='legacy-original-only.md' WHERE id=?1",
        [missing_provenance],
    )
    .unwrap();
    let legacy_summary = enforceability_summary(cfg, db).unwrap();
    partition(&legacy_summary, [23, 10, 8, 0, 5]);
    let legacy_origin = legacy_summary
        .rules
        .iter()
        .find(|r| r.triple_id == missing_provenance)
        .unwrap();
    assert_eq!(
        legacy_origin.source_path.as_deref(),
        Some("legacy-original-only.md")
    );
    assert_eq!(
        (legacy_origin.line_start, legacy_origin.line_end),
        (None, None)
    );
}

fn modes(cfg: &Config, db: &Store) {
    for (name, severity, timing) in [
        ("block", Severity::Block, Timing::ToolCall),
        ("warn", Severity::Warn, Timing::ToolCall),
        ("inform", Severity::Inform, Timing::ToolCall),
        ("inert", Severity::Block, Timing::Principle),
    ] {
        insert(db, name, &triple(name, "never"), Some(severity), timing);
    }
    let excluded = triple("already-disabled", "never");
    insert(db, "disabled", &excluded, None, Timing::ToolCall);
    db.disable_rule(&excluded.subject, &excluded.predicate, &excluded.object)
        .unwrap();
    // These are contract truth tables, separate from the implementation helpers.
    let deny = !matches!(
        std::env::var("ARAI_DENY_MODE")
            .unwrap_or_default()
            .to_lowercase()
            .as_str(),
        "off" | "0" | "false" | "no"
    );
    let bypass = matches!(
        std::env::var("ARAI_DISABLED")
            .unwrap_or_default()
            .to_lowercase()
            .as_str(),
        "1" | "true" | "on" | "yes"
    );
    let s = enforceability_summary(cfg, db).unwrap();
    assert_eq!((s.advisory_only, s.hard_bypass), (!deny && !bypass, bypass));
    partition(
        &s,
        if bypass {
            [5, 0, 0, 0, 5]
        } else if deny {
            [5, 1, 2, 1, 1]
        } else {
            [5, 0, 3, 1, 1]
        },
    );
    for row in &s.rules {
        if row.source_path.as_deref() == Some("disabled") {
            assert_eq!(row.inactive_reasons, [Reason::Disabled]);
        } else if bypass {
            assert_eq!(row.inactive_reasons, [Reason::RuntimeBypass]);
        } else {
            assert!(row.inactive_reasons.is_empty());
        }
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    let mut stack = vec![root.to_owned()];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if !path.to_string_lossy().ends_with("-shm") {
                result.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }
    result
}

fn failures(cfg: &Config, db: &Store) {
    insert(
        db,
        "read-only",
        &triple("read-only", "never"),
        None,
        Timing::ToolCall,
    );
    // Enrichment would leave a marker if accidentally invoked by summary.
    let marker = cfg.project_root.join("external-execution-marker");
    let mut dangerous = config(cfg.project_root.parent().unwrap());
    dangerous.llm_command = Some(format!("echo unexpected > {}", marker.display()));
    dangerous.api_url = Some("http://127.0.0.1:1/verifier-must-not-call".into());
    let monitor = Connection::open(cfg.db_path()).unwrap();
    let version: i64 = monitor
        .query_row("PRAGMA data_version", [], |r| r.get(0))
        .unwrap();
    let before = snapshot(cfg.project_root.parent().unwrap());
    let first = enforceability_summary(&dangerous, db).unwrap();
    for _ in 0..4 {
        assert_eq!(first, enforceability_summary(&dangerous, db).unwrap());
    }
    assert_eq!(before, snapshot(cfg.project_root.parent().unwrap()));
    assert_eq!(
        version,
        monitor
            .query_row::<i64, _, _>("PRAGMA data_version", [], |r| r.get(0))
            .unwrap()
    );
    assert!(!marker.exists());
    for bad in [
        "{",
        "{\"directory\":null,\"patterns\":[\"[\"],\"inactive\":null}",
        "{\"directory\":null,\"patterns\":[],\"inactive\":null,\"surprise\":true}",
    ] {
        monitor
            .execute("UPDATE source_metadata SET scope_json=?1", [bad])
            .unwrap();
        let hook = serde_json::json!({"hook_event_name":"PreToolUse","tool_name":"Write","tool_input":{"file_path":"sentinel-secrets.txt","content":"sentinel secrets"}});
        let existing = arai::hooks::match_hook(&hook, cfg, db)
            .err()
            .expect("existing native persisted-scope diagnostic");
        let before = snapshot(cfg.project_root.parent().unwrap());
        let error = enforceability_summary(cfg, db).unwrap_err();
        assert_eq!(error, existing);
        assert!(!error.is_empty());
        assert_eq!(before, snapshot(cfg.project_root.parent().unwrap()));
    }
    // A corrupted inventory table cannot be translated to a successful empty store.
    monitor.execute_batch("DROP TABLE rule_intent").unwrap();
    let existing = db.rules_for_file("read-only").unwrap_err().to_string();
    let before = snapshot(cfg.project_root.parent().unwrap());
    assert_eq!(enforceability_summary(cfg, db).unwrap_err(), existing);
    assert_eq!(before, snapshot(cfg.project_root.parent().unwrap()));
}

fn native(root: &Path, cfg: &Config, db: &Store) {
    use std::io::Write;
    // P6: unchanged direct construction and predicate fallback for embedders.
    let embedding = arai::store::Guardrail {
        triple_id: 42,
        subject: "git".into(),
        predicate: "never".into(),
        object: "git push --force".into(),
        confidence: 0.99,
        source_file: "original.md".into(),
        file_path: "attached.md".into(),
        layer: None,
        line_start: None,
        expires_at: None,
        noenrich: false,
        intent: None,
        tier: None,
        source_label: None,
    };
    assert_eq!(
        arai::hooks::highest_severity(&[(embedding, 100)]),
        Severity::Block
    );
    let mut rule = triple("git", "never");
    rule.object = "git push --force".into();
    let id = insert(
        db,
        "manual://verifier",
        &rule,
        Some(Severity::Block),
        Timing::ToolCall,
    );
    db.upsert_rule_intent(
        id,
        &RuleIntent {
            action: Action::Execute,
            timing: Timing::ToolCall,
            tools: vec!["Bash".into()],
            allow_inverse: false,
            enriched_by: "independent-verifier".into(),
            severity: Severity::Block,
        },
    )
    .unwrap();
    let hook = serde_json::json!({"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git push --force"},"cwd":cfg.project_root,"session_id":"verifier-native"});
    let matched = arai::hooks::match_hook(&hook, cfg, db).unwrap();
    assert!(!matched.matched.is_empty());
    assert_eq!(
        arai::hooks::highest_severity(&matched.matched),
        Severity::Block
    );
    let mut unrelated = hook.clone();
    unrelated["tool_input"]["command"] = "git status".into();
    assert!(arai::hooks::match_hook(&unrelated, cfg, db)
        .unwrap()
        .matched
        .is_empty());
    for host in ["claude", "codex", "grok"] {
        for mode in ["deny", "advice", "bypass"] {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_arai"));
            isolated(&mut cmd, root);
            cmd.args(["guardrails", "--match-stdin"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut payload = hook.clone();
            if host == "grok" {
                cmd.env("GROK_HOOK_EVENT", "pre_tool_use");
                payload["hook_event_name"] = "pre_tool_use".into();
                payload["tool_name"] = "run_terminal_command".into();
            }
            if host == "codex" {
                payload["tool_name"] = "Bash".into();
            }
            if mode == "advice" {
                cmd.env("ARAI_DENY_MODE", "off");
            }
            if mode == "bypass" {
                cmd.env("ARAI_DISABLED", "yes");
            }
            let mut process = cmd.spawn().unwrap();
            process
                .stdin
                .take()
                .unwrap()
                .write_all(payload.to_string().as_bytes())
                .unwrap();
            let out = process.wait_with_output().unwrap();
            assert_eq!(
                out.status.code(),
                Some(if host == "grok" && mode == "deny" {
                    2
                } else {
                    0
                }),
                "{host}/{mode}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            if mode == "bypass" {
                assert!(out.stdout.is_empty());
                continue;
            }
            let response: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            if host == "grok" {
                assert_eq!(
                    response["decision"],
                    if mode == "deny" { "deny" } else { "allow" }
                );
            } else {
                assert_eq!(
                    response["hookSpecificOutput"]["permissionDecision"],
                    if mode == "deny" { "deny" } else { "allow" }
                );
            }
        }
    }
}
