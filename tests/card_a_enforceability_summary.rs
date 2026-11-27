//! Card A's read-only operation, exercised in isolated child environments.
use arai::{
    config::Config,
    discovery, init,
    init::{EnforceabilityClass as Class, InactivityReason as Reason},
    intent::{Action, RuleIntent, Severity, Timing},
    parser::Triple,
    store::Store,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, process::Command};

struct TempRoot(PathBuf);

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    db: Store,
    cfg: Config,
    _root: TempRoot,
}

impl Fixture {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = TempRoot(
            std::env::temp_dir().join(format!("arai_card_a_{}_{stamp}", std::process::id())),
        );
        let cfg = Config {
            project_root: root.0.join("project"),
            home_dir: root.0.join("home"),
            arai_base_dir: root.0.join("state"),
            extra_sources: Vec::new(),
            guardrails_mode: "advise".into(),
            llm_command: None,
            api_url: None,
            api_key_env: None,
            api_model: None,
        };
        fs::create_dir_all(cfg.project_root.join(".git")).unwrap();
        fs::create_dir_all(&cfg.home_dir).unwrap();
        let db = Store::open(&cfg.db_path()).unwrap();
        Self {
            db,
            cfg,
            _root: root,
        }
    }

    fn sql(&self) -> Connection {
        Connection::open(self.cfg.db_path()).unwrap()
    }

    fn add(&self, path: &str, content: &str, kind: &str, rows: &[Triple]) -> Vec<i64> {
        self.db.upsert_file(path, content, rows, kind).unwrap();
        self.db
            .rules_for_file(path)
            .unwrap()
            .iter()
            .map(|g| g.triple_id)
            .collect()
    }

    fn summary(&self) -> init::EnforceabilitySummary {
        let summary = init::enforceability_summary(&self.cfg, &self.db).unwrap();
        assert_eq!(summary.stored_actionable_rule_count, summary.rules.len());
        assert_eq!(
            summary.stored_actionable_rule_count,
            summary.block_eligible_count
                + summary.advise_eligible_count
                + summary.documentary_inert_count
                + summary.inactive_count
        );
        summary
    }
}

fn triple(subject: &str, predicate: &str) -> Triple {
    Triple {
        subject: subject.into(),
        predicate: predicate.into(),
        object: "fixture policy".into(),
        confidence: 0.9,
        domain: "fixture".into(),
        source_file: "original.md".into(),
        line_start: Some(7),
        line_end: Some(9),
        layer: Some(1),
        expires_at: None,
        noenrich: false,
        tier: None,
        source_label: None,
    }
}

fn intent(severity: Severity, timing: Timing) -> RuleIntent {
    RuleIntent {
        action: Action::General,
        timing,
        tools: vec!["*".into()],
        allow_inverse: false,
        enriched_by: "fixture".into(),
        severity,
    }
}

// Child-only environment settings test production mode semantics without
// disabling this checkout's hooks or racing other tests' process environment.
fn child_command(name: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", name, "--nocapture"]);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("ARAI_") {
            command.env_remove(key);
        }
    }
    command.env("CARD_A_CHILD", name).env("DO_NOT_TRACK", "1");
    command
}

fn in_child(name: &str) -> bool {
    if std::env::var("CARD_A_CHILD").as_deref() == Ok(name) {
        return true;
    }
    let output = child_command(name).output().unwrap();
    assert!(
        output.status.success(),
        "{name}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

#[test]
fn empty_and_documentary_populations() {
    if !in_child("empty_and_documentary_populations") {
        return;
    }
    let f = Fixture::new();
    let empty = f.summary();
    assert_eq!(
        (
            empty.stored_source_count,
            empty.stored_actionable_rule_count
        ),
        (0, 0)
    );
    assert_eq!(
        (
            empty.block_eligible_count,
            empty.advise_eligible_count,
            empty.documentary_inert_count,
            empty.inactive_count
        ),
        (0, 0, 0, 0)
    );
    for (index, timing) in [Timing::Start, Timing::Stop, Timing::Principle]
        .into_iter()
        .enumerate()
    {
        let path = format!("inert{index}");
        let ids = f.add(&path, &path, "manual", &[triple(&path, "never")]);
        f.db.upsert_rule_intent(ids[0], &intent(Severity::Block, timing))
            .unwrap();
    }
    let summary = f.summary();
    assert_eq!(
        (
            summary.block_eligible_count,
            summary.advise_eligible_count,
            summary.documentary_inert_count,
            summary.inactive_count
        ),
        (0, 0, 3, 0)
    );
    assert!(summary
        .rules
        .iter()
        .all(|r| r.classification == Class::DocumentaryInert));
    assert_eq!(summary, f.summary());
}

#[test]
fn mixed_effective_inventory_scope_and_provenance() {
    if !in_child("mixed_effective_inventory_scope_and_provenance") {
        return;
    }
    let f = Fixture::new();
    let mut rows = vec![
        triple("block", "never"),
        triple("warn", "always"),
        triple("inform", "prefers"),
        triple("demote", "never"),
        triple("promote", "prefers"),
        triple("yesterday", "never"),
        triple("today", "never"),
        triple("ignored", "mentions"),
    ];
    let sql = f.sql();
    rows[5].expires_at = Some(
        sql.query_row("SELECT date('now','-1 day')", [], |r| r.get(0))
            .unwrap(),
    );
    rows[6].expires_at = Some(
        sql.query_row("SELECT date('now')", [], |r| r.get(0))
            .unwrap(),
    );
    f.add("CLAUDE.md", "root", "claude_md_project", &rows);
    for guard in f.db.rules_for_file("CLAUDE.md").unwrap() {
        if guard.subject != "block" {
            f.db.upsert_rule_intent(
                guard.triple_id,
                &intent(Severity::from_predicate(&guard.predicate), Timing::ToolCall),
            )
            .unwrap();
        }
    }
    f.db.set_severity_override("demote", Severity::Inform)
        .unwrap();
    f.db.set_severity_override("promote", Severity::Block)
        .unwrap();
    let duplicate = triple("duplicate", "never");
    f.add(
        "nested/AGENTS.md",
        "nested",
        "agents_md",
        std::slice::from_ref(&duplicate),
    );
    f.add(
        "external://duplicate",
        "external",
        "manual",
        std::slice::from_ref(&duplicate),
    );
    f.db.disable_rule(&duplicate.subject, &duplicate.predicate, &duplicate.object)
        .unwrap();
    let mut overlapping = triple("overlap", "never");
    overlapping.expires_at = rows[5].expires_at.clone();
    f.add(
        ".cursor/rules/manual.mdc",
        "---\nalwaysApply: false\n---\n",
        "cursor_rules",
        std::slice::from_ref(&overlapping),
    );
    f.db.disable_rule(
        &overlapping.subject,
        &overlapping.predicate,
        &overlapping.object,
    )
    .unwrap();
    f.add(
        "nested/.claude/rules/files.md",
        "---\npaths: [\"src/**/*.rs\"]\n---\n",
        "claude_rules",
        &[triple("patterns", "requires")],
    );
    f.add(
        "legacy://rule",
        "legacy",
        "manual",
        &[triple("legacy", "must_not")],
    );
    sql.execute("DELETE FROM source_metadata WHERE file_id=(SELECT id FROM files WHERE path='legacy://rule')", []).unwrap();
    f.add("empty-source", "empty", "manual", &[]);
    f.add(
        "",
        "absent",
        "manual",
        &[Triple {
            source_file: String::new(),
            line_start: None,
            line_end: None,
            ..triple("absent", "forbids")
        }],
    );
    f.add(
        "attached-empty",
        "fallback",
        "manual",
        &[triple("fallback", "enforces")],
    );
    let summary = f.summary();
    assert_eq!(
        (
            summary.stored_source_count,
            summary.stored_actionable_rule_count
        ),
        (9, 14)
    );
    assert_eq!(
        (
            summary.block_eligible_count,
            summary.advise_eligible_count,
            summary.documentary_inert_count,
            summary.inactive_count
        ),
        (5, 5, 0, 4)
    );
    let by_subject = |subject: &str| {
        let id: i64 = sql
            .query_row(
                "SELECT id FROM triples WHERE s=?1 LIMIT 1",
                [subject],
                |r| r.get(0),
            )
            .unwrap();
        summary.rules.iter().find(|r| r.triple_id == id).unwrap()
    };
    assert_eq!(by_subject("demote").effective_severity, "inform");
    assert_eq!(by_subject("demote").classification, Class::AdviseEligible);
    assert_eq!(by_subject("promote").classification, Class::BlockEligible);
    assert_eq!(by_subject("today").classification, Class::BlockEligible);
    assert_eq!(
        by_subject("yesterday").inactive_reasons,
        vec![Reason::Expired]
    );
    assert_eq!(
        by_subject("overlap").inactive_reasons,
        vec![
            Reason::Disabled,
            Reason::Expired,
            Reason::PermanentSourceInactive
        ]
    );
    assert!(by_subject("overlap").source_inactive_reason.is_some());
    for row in summary
        .rules
        .iter()
        .filter(|r| r.inactive_reasons == vec![Reason::Disabled])
    {
        assert!(row
            .source_path
            .as_deref()
            .is_some_and(|s| s.contains("duplicate") || s == "nested/AGENTS.md"));
    }
    assert_eq!(
        by_subject("block").source_path.as_deref(),
        Some("CLAUDE.md")
    );
    assert_eq!(
        (by_subject("block").line_start, by_subject("block").line_end),
        (Some(7), Some(9))
    );
    assert_eq!(by_subject("patterns").scope_patterns, ["src/**/*.rs"]);
    assert_eq!(
        by_subject("patterns").scope_directory.as_deref(),
        Some("nested")
    );
    assert_eq!(by_subject("patterns").classification, Class::AdviseEligible);
    assert_eq!(
        by_subject("duplicate").scope_directory.as_deref(),
        Some("nested")
    );
    assert_eq!(by_subject("block").scope_directory.as_deref(), Some(""));
    assert!(by_subject("legacy").scope_directory.is_none());
    assert!(by_subject("legacy").scope_patterns.is_empty());
    assert_eq!(by_subject("absent").source_path, None);
    assert_eq!(
        (
            by_subject("absent").line_start,
            by_subject("absent").line_end
        ),
        (None, None)
    );
    assert!(f
        .db
        .rules_for_file("CLAUDE.md")
        .unwrap()
        .iter()
        .find(|g| g.subject == "block")
        .unwrap()
        .intent
        .is_none());
    // Attachment wins above; a usable original source is the fallback when
    // the legacy attachment is empty. Neither absent paths nor lines are made up.
    sql.execute(
        "UPDATE files SET path='absent-now-attached' WHERE path=''",
        [],
    )
    .unwrap();
    sql.execute("UPDATE files SET path='' WHERE path='attached-empty'", [])
        .unwrap();
    sql.execute(
        "UPDATE triples SET source_file='original.md' WHERE s='fallback'",
        [],
    )
    .unwrap();
    let summary = f.summary();
    let fallback_id: i64 = sql
        .query_row("SELECT id FROM triples WHERE s='fallback'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        summary
            .rules
            .iter()
            .find(|r| r.triple_id == fallback_id)
            .unwrap()
            .source_path
            .as_deref(),
        Some("original.md")
    );
}

#[test]
fn runtime_modes_are_production_modes() {
    for deny in [
        None,
        Some("off"),
        Some("0"),
        Some("false"),
        Some("no"),
        Some("OFF"),
        Some("FaLsE"),
        Some(""),
        Some("on"),
        Some("yes"),
        Some("1"),
        Some(" off "),
    ] {
        let advisory =
            deny.is_some_and(|s| matches!(s.to_lowercase().as_str(), "off" | "0" | "false" | "no"));
        let mut command = child_command("runtime_mode_child");
        if let Some(value) = deny {
            command.env("ARAI_DENY_MODE", value);
        }
        command.env("CARD_A_EXPECT_ADVISORY", if advisory { "1" } else { "0" });
        command.env("CARD_A_EXPECT_BYPASS", "0");
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "deny={deny:?}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for bypass in [
        None,
        Some("1"),
        Some("true"),
        Some("on"),
        Some("yes"),
        Some("TRUE"),
        Some("YeS"),
        Some("0"),
        Some("false"),
        Some("off"),
        Some("no"),
        Some(" true "),
    ] {
        let hard_bypass = bypass
            .is_some_and(|s| matches!(s.to_lowercase().as_str(), "1" | "true" | "on" | "yes"));
        for deny in ["on", "off"] {
            let mut command = child_command("runtime_mode_child");
            if let Some(value) = bypass {
                command.env("ARAI_DISABLED", value);
            }
            command.env("ARAI_DENY_MODE", deny);
            command.env(
                "CARD_A_EXPECT_ADVISORY",
                if deny == "off" && !hard_bypass {
                    "1"
                } else {
                    "0"
                },
            );
            command.env("CARD_A_EXPECT_BYPASS", if hard_bypass { "1" } else { "0" });
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "bypass={bypass:?}, deny={deny}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn runtime_mode_child() {
    if std::env::var("CARD_A_CHILD").as_deref() != Ok("runtime_mode_child") {
        return;
    }
    let mut f = Fixture::new();
    let rows = [
        triple("block", "never"),
        triple("warn", "always"),
        triple("inert", "never"),
        triple("disabled", "never"),
    ];
    f.add("modes", "modes", "manual", &rows);
    for guard in f.db.rules_for_file("modes").unwrap() {
        f.db.upsert_rule_intent(
            guard.triple_id,
            &intent(
                Severity::from_predicate(&guard.predicate),
                if guard.subject == "inert" {
                    Timing::Start
                } else {
                    Timing::ToolCall
                },
            ),
        )
        .unwrap();
    }
    f.db.disable_rule("disabled", "never", "fixture policy")
        .unwrap();
    let advisory = std::env::var("CARD_A_EXPECT_ADVISORY").unwrap() == "1";
    let bypass = std::env::var("CARD_A_EXPECT_BYPASS").unwrap() == "1";
    let summary = f.summary();
    assert_eq!(
        (summary.advisory_only, summary.hard_bypass),
        (advisory, bypass)
    );
    assert_eq!(
        (
            summary.block_eligible_count,
            summary.advise_eligible_count,
            summary.documentary_inert_count,
            summary.inactive_count
        ),
        if bypass {
            (0, 0, 0, 4)
        } else if advisory {
            (0, 2, 1, 1)
        } else {
            (1, 1, 1, 1)
        }
    );
    assert_eq!(
        summary
            .rules
            .iter()
            .filter(|r| r.inactive_reasons == vec![Reason::RuntimeBypass])
            .count(),
        if bypass { 3 } else { 0 }
    );
    assert_eq!(
        summary
            .rules
            .iter()
            .filter(|r| r.inactive_reasons == vec![Reason::Disabled])
            .count(),
        1
    );
    for mode in ["block", "off", "advise", "unknown"] {
        f.cfg.guardrails_mode = mode.into();
        assert_eq!(summary, f.summary());
    }
}

#[test]
fn read_failures_preserve_diagnostics_and_summary_does_not_write() {
    if !in_child("read_failures_preserve_diagnostics_and_summary_does_not_write") {
        return;
    }
    let f = Fixture::new();
    f.add("policy", "policy", "manual", &[triple("policy", "never")]);
    let sql = f.sql();
    // Monitor commits from the Store connection and compare a complete logical
    // dump, covering policy, metadata, sessions and every other database table.
    fn dump(sql: &Connection) -> Vec<String> {
        let tables: Vec<String> = sql
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        tables
            .into_iter()
            .map(|table| {
                let mut statement = sql
                    .prepare(&format!("SELECT * FROM \"{}\"", table.replace('"', "\"\"")))
                    .unwrap();
                let width = statement.column_count();
                let mut rows: Vec<String> = statement
                    .query_map([], |r| {
                        Ok((0..width)
                            .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                            .collect::<Vec<_>>()
                            .join("|"))
                    })
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
                rows.sort();
                format!("{table}: {rows:?}")
            })
            .collect()
    }
    fn files(path: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut result = Vec::new();
        if path.exists() {
            for entry in fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    result.extend(files(&path));
                } else {
                    result.push((path.clone(), fs::read(path).unwrap()));
                }
            }
        }
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }
    let version: i64 = sql
        .query_row("PRAGMA data_version", [], |r| r.get(0))
        .unwrap();
    let before = dump(&sql);
    let before_files = files(&f.cfg.arai_base_dir);
    f.summary();
    f.summary();
    assert_eq!(
        version,
        sql.query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
            .unwrap()
    );
    assert_eq!(before, dump(&sql));
    assert_eq!(before_files, files(&f.cfg.arai_base_dir));
    assert!(!f.cfg.project_root.join(".claude").exists());
    for json in [
        "{",
        r#"{"directory":null,"patterns":["[invalid"],"inactive":null}"#,
    ] {
        sql.execute("UPDATE source_metadata SET scope_json=?1", [json])
            .unwrap();
        let error = init::enforceability_summary(&f.cfg, &f.db).unwrap_err();
        assert!(
            if json == "{" {
                error.contains("EOF")
            } else {
                error.contains("Invalid scope glob")
            },
            "{error}"
        );
    }
    sql.execute_batch("ALTER TABLE triples RENAME TO unavailable_triples")
        .unwrap();
    let error = init::enforceability_summary(&f.cfg, &f.db).unwrap_err();
    assert!(error.contains("no such table: triples"), "{error}");
}

#[test]
fn frozen_three_source_baseline() {
    if !in_child("frozen_three_source_baseline") {
        return;
    }
    let f = Fixture::new();
    const NESTED: &str =
        ".taniwha/kupu/orchestrator/handoff/01KSGB5549QPQDQMSH6B8NS077/outputs/files/CLAUDE.md";
    let sources: [(&str, &[u8], &str); 3] = [
        ("CLAUDE.md", include_bytes!("fixtures/card_a_baseline/CLAUDE.md"), "FA752284BF2D055BE4AFF5F423DD5CC900CE47F3E08D4FA552C2C7502A88247D"),
        ("AGENTS.md", include_bytes!("fixtures/card_a_baseline/AGENTS.md"), "35F70EA23244AECCF451CFA786108C957C57BF0AC00378F4D13D4EDECA5CCFC8"),
        (NESTED, include_bytes!("fixtures/card_a_baseline/.taniwha/kupu/orchestrator/handoff/01KSGB5549QPQDQMSH6B8NS077/outputs/files/CLAUDE.md"), "B0E34F81E8AD8566D82ED67FD21A4CAA47170E8667AF5C9F40F67C2467592224"),
    ];
    for (path, bytes, hash) in sources {
        let digest: String = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect();
        assert_eq!(digest, hash);
        let path = f.cfg.project_root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    let discovered = discovery::discover(&f.cfg).unwrap();
    assert_eq!(discovered.len(), 3);
    assert_eq!(init::sync_discovered_rules(&f.db, &discovered).unwrap(), 22);
    let summary = f.summary();
    assert_eq!(
        (
            summary.stored_source_count,
            summary.stored_actionable_rule_count
        ),
        (3, 22)
    );
    assert_eq!(
        (
            summary.block_eligible_count,
            summary.advise_eligible_count,
            summary.documentary_inert_count,
            summary.inactive_count
        ),
        (0, 0, 22, 0)
    );
    let timings: Vec<_> =
        f.db.list_files()
            .unwrap()
            .iter()
            .flat_map(|path| f.db.rules_for_file(path).unwrap())
            .map(|g| g.intent.unwrap().timing)
            .collect();
    for (timing, count) in [
        (Timing::Principle, 18),
        (Timing::Start, 3),
        (Timing::Stop, 1),
    ] {
        assert_eq!(timings.iter().filter(|t| **t == timing).count(), count);
    }
    let mut lines: Vec<_> = summary
        .rules
        .iter()
        .filter(|r| {
            r.source_path
                .as_deref()
                .is_some_and(|s| s.ends_with("AGENTS.md"))
        })
        .map(|r| r.line_start.unwrap())
        .collect();
    lines.sort();
    assert_eq!(
        lines,
        [9, 10, 11, 16, 17, 19, 20, 21, 26, 28, 32, 33, 39, 44, 52, 58, 59, 61, 62, 63]
    );
    for (path, line) in [("CLAUDE.md", 121), (NESTED, 101)] {
        let explanation = summary
            .rules
            .iter()
            .find(|r| {
                r.source_path
                    .as_deref()
                    .is_some_and(|s| s.replace('\\', "/").ends_with(path))
            })
            .unwrap();
        assert_eq!(explanation.line_start, Some(line));
    }
}
