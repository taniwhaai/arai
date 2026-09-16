//! Crash-recovery and real multiprocess regressions for canonical audit writes.
use arai::{audit, config::Config};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Project(PathBuf);

impl Project {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "arai_audit_concurrency_{}_{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn config(&self) -> Config {
        config(&self.0)
    }

    fn bucket(&self) -> audit::AuditBucket {
        let cfg = self.config();
        let buckets = audit::list_buckets(&cfg.arai_base_dir, &cfg.project_slug()).unwrap();
        assert_eq!(buckets.len(), 1);
        buckets[0].clone()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn config(root: &Path) -> Config {
    Config {
        project_root: root.join("project"),
        home_dir: root.join("home"),
        arai_base_dir: root.join("state"),
        extra_sources: Vec::new(),
        guardrails_mode: "advise".into(),
        llm_command: None,
        api_url: None,
        api_key_env: None,
        api_model: None,
    }
}

fn entries(bucket: &audit::AuditBucket) -> Vec<Value> {
    String::from_utf8(bucket.jsonl_bytes().unwrap())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn stale_and_missing_sidecars_recover_from_the_last_complete_record() {
    let project = Project::new();
    let cfg = project.config();
    audit::record_event(&cfg, "Test", "Bash", "session", json!({"i": 0}));
    let first_head = project.bucket().head;
    audit::record_event(&cfg, "Test", "Bash", "session", json!({"i": 1}));
    let bucket = project.bucket();
    let head_path = bucket
        .jsonl_path
        .with_file_name(format!(".head.{}", bucket.day));
    // Simulate a crash after the second append but before the sidecar update.
    fs::write(&head_path, format!("{first_head}\n")).unwrap();
    audit::record_event(&cfg, "Test", "Bash", "session", json!({"i": 2}));
    fs::remove_file(&head_path).unwrap();
    audit::record_event(&cfg, "Test", "Bash", "session", json!({"i": 3}));

    let bucket = project.bucket();
    let records = entries(&bucket);
    assert_eq!(records.len(), 4);
    assert_eq!(records[3]["hash"].as_str().unwrap(), bucket.head);
    let issues = audit::verify_chain(&cfg.arai_base_dir, &cfg.project_slug()).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn predecessor_recovery_handles_records_larger_than_the_read_chunk() {
    let project = Project::new();
    let cfg = project.config();
    for i in 0..3 {
        audit::record_event(
            &cfg,
            "LargeRecord",
            "Bash",
            "session",
            json!({"i": i, "text": "é".repeat(20_000)}),
        );
    }
    assert_eq!(entries(&project.bucket()).len(), 3);
    let issues = audit::verify_chain(&cfg.arai_base_dir, &cfg.project_slug()).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn torn_tail_is_preserved_and_does_not_restart_the_chain() {
    let project = Project::new();
    let cfg = project.config();
    audit::record_event(&cfg, "Test", "Bash", "session", json!({"i": 0}));
    let bucket = project.bucket();
    fs::OpenOptions::new()
        .append(true)
        .open(&bucket.jsonl_path)
        .unwrap()
        .write_all(b"{\"payload\":\"torn")
        .unwrap();
    let before = bucket.jsonl_bytes().unwrap();
    audit::record_event(&cfg, "Test", "Bash", "session", json!({"i": 1}));
    assert_eq!(bucket.jsonl_bytes().unwrap(), before);
    assert_eq!(project.bucket().head, bucket.head);
    let issues = audit::verify_chain(&cfg.arai_base_dir, &cfg.project_slug()).unwrap();
    assert!(issues.iter().any(|issue| issue.kind == "incomplete_record"));
    assert!(issues.iter().any(|issue| issue.kind == "malformed_json"));
}

#[test]
fn even_valid_json_without_its_record_terminator_is_incomplete() {
    let project = Project::new();
    let cfg = project.config();
    audit::record_event(&cfg, "Test", "Bash", "session", json!({"i": 0}));
    let bucket = project.bucket();
    let mut raw = bucket.jsonl_bytes().unwrap();
    assert_eq!(raw.pop(), Some(b'\n'));
    fs::write(&bucket.jsonl_path, &raw).unwrap();
    audit::record_event(&cfg, "Test", "Bash", "session", json!({"i": 1}));
    assert_eq!(bucket.jsonl_bytes().unwrap(), raw);
    let issues = audit::verify_chain(&cfg.arai_base_dir, &cfg.project_slug()).unwrap();
    assert!(issues.iter().any(|issue| issue.kind == "incomplete_record"));
}

#[test]
fn bucket_read_errors_are_not_reported_as_empty_evidence() {
    let project = Project::new();
    let cfg = project.config();
    assert!(audit::list_buckets(&cfg.arai_base_dir, &cfg.project_slug())
        .unwrap()
        .is_empty());
    let audit_dir = cfg.arai_base_dir.join("audit");
    fs::create_dir_all(&audit_dir).unwrap();
    fs::write(audit_dir.join(cfg.project_slug()), b"not a directory").unwrap();
    assert!(audit::list_buckets(&cfg.arai_base_dir, &cfg.project_slug()).is_err());
    assert!(audit::verify_chain(&cfg.arai_base_dir, &cfg.project_slug()).is_err());
}

#[test]
fn unchained_legacy_tail_requires_explicit_migration() {
    let project = Project::new();
    let cfg = project.config();
    audit::record_event(&cfg, "Test", "Bash", "session", json!({"i": 0}));
    let bucket = project.bucket();
    let legacy = b"{\"event\":\"Legacy\"}\n";
    fs::write(&bucket.jsonl_path, legacy).unwrap();
    audit::record_event(&cfg, "Test", "Bash", "session", json!({"i": 1}));
    assert_eq!(bucket.jsonl_bytes().unwrap(), legacy);
    let issues = audit::verify_chain(&cfg.arai_base_dir, &cfg.project_slug()).unwrap();
    assert!(issues.iter().any(|issue| issue.kind == "unchained_legacy"));
}

#[test]
fn concurrent_processes_produce_one_complete_chain() {
    let project = Project::new();
    let cfg = project.config();
    // Initialize permissions before starting the processes to concentrate the
    // regression on competing read/hash/append/head operations.
    audit::record_event(&cfg, "Seed", "Bash", "session", json!({}));
    let mut workers = Vec::new();
    for worker in 0..6 {
        workers.push(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "subprocess_audit_writer", "--nocapture"])
                .env("ARAI_AUDIT_TEST_ROOT", &project.0)
                .env("ARAI_AUDIT_TEST_WORKER", worker.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while !(0..6).all(|worker| project.0.join(format!("ready-{worker}")).exists()) {
        assert!(Instant::now() < deadline, "audit workers did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    fs::write(project.0.join("go"), b"start").unwrap();
    for child in workers {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let records = entries(&project.bucket());
    assert_eq!(records.len(), 1 + 6 * 40);
    let identifiers: HashSet<_> = records
        .iter()
        .skip(1)
        .map(|event| {
            (
                event["payload"]["worker"].as_u64().unwrap(),
                event["payload"]["sequence"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(identifiers.len(), 6 * 40);
    let issues = audit::verify_chain(&cfg.arai_base_dir, &cfg.project_slug()).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn subprocess_audit_writer() {
    let Some(root) = std::env::var_os("ARAI_AUDIT_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let worker: u64 = std::env::var("ARAI_AUDIT_TEST_WORKER")
        .unwrap()
        .parse()
        .unwrap();
    let cfg = config(&root);
    fs::write(root.join(format!("ready-{worker}")), b"ready").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !root.join("go").exists() {
        assert!(
            Instant::now() < deadline,
            "audit worker start gate timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    for sequence in 0..40 {
        audit::record_event(
            &cfg,
            "Concurrent",
            "Bash",
            "session",
            json!({"worker": worker, "sequence": sequence}),
        );
    }
}
