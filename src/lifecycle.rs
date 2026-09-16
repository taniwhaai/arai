//! Local lifecycle diagnostics for CLI adapters, separate from rule evidence.
//!
//! Startup reads accepted SQLite state without migrations, discovery, model
//! loading or network access. Invocation receipts describe this adapter process;
//! they do not authenticate a host or prove that another hook is activated.

use crate::{config::Config, platforms::Platform};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const EVENTS: &[(&str, &str)] = &[
    ("SessionStart", "session-start"),
    ("SubagentStart", "subagent-start"),
    ("PreToolUse", "pre-tool-use"),
    ("PostToolUse", "post-tool-use"),
];
const MAX_RECEIPT_BYTES: u64 = 16 * 1024;

#[derive(Clone, Copy)]
enum Mode {
    Disabled,
    Advisory,
    Enforcing,
}

impl Mode {
    fn from_values(disabled: &str, deny: &str) -> Self {
        if matches!(
            disabled.to_ascii_lowercase().as_str(),
            "1" | "true" | "on" | "yes"
        ) {
            Self::Disabled
        } else if matches!(
            deny.to_ascii_lowercase().as_str(),
            "off" | "0" | "false" | "no"
        ) {
            Self::Advisory
        } else {
            Self::Enforcing
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled (ARAI_DISABLED)",
            Self::Advisory => "advisory (ARAI_DENY_MODE)",
            Self::Enforcing => "deny mode enabled",
        }
    }
}

/// Brief lifecycle availability, using only an existing local policy store.
/// Unsupported platforms/events produce an empty object, never a tool decision.
pub fn startup(cfg: &Config, platform: Platform, event: &str) -> Value {
    let mode = Mode::from_values(
        &std::env::var("ARAI_DISABLED").unwrap_or_default(),
        &std::env::var("ARAI_DENY_MODE").unwrap_or_default(),
    );
    startup_with_mode(cfg, platform, event, mode)
}

fn startup_with_mode(cfg: &Config, platform: Platform, event: &str, mode: Mode) -> Value {
    if !matches!(event, "SessionStart" | "SubagentStart") || platform == Platform::Cursor {
        return json!({});
    }
    let state = match stored_policy(cfg) {
        Ok(None) => "policy store missing; run arai init to set up this project".to_string(),
        Ok(Some((count, scan))) => {
            let now = now_millis() / 1000;
            let scan = match scan {
                Some(timestamp) if timestamp <= now => {
                    format!("last scan {}s ago", now - timestamp)
                }
                Some(_) => "last scan timestamp is in the future".to_string(),
                None => "last scan unknown".to_string(),
            };
            format!("{count} stored guardrail(s); {scan}; disk freshness not checked")
        }
        Err(_) => "policy store unavailable; inspect arai status before relying on enforcement"
            .to_string(),
    };
    let message = format!(
        "Arai {}: {} adapter invoked; {}; {}. Tool-hook activation is not proven by this startup. Use arai status / arai why to inspect policy; run arai scan after instruction edits.",
        env!("CARGO_PKG_VERSION"), platform.id(), mode.label(), state
    );
    let mut output = json!({});
    if matches!(platform, Platform::Claude | Platform::Codex) {
        output["hookSpecificOutput"] = json!({
            "hookEventName": event,
            "additionalContext": message,
        });
    }
    // This is a user diagnostic, not a claim of model-context delivery. Grok's
    // passive-event stdout does not provide an additionalContext surface.
    if event == "SessionStart" {
        output["systemMessage"] = Value::String(message);
    }
    output
}

fn stored_policy(cfg: &Config) -> Result<Option<(i64, Option<u64>)>, String> {
    let path = cfg.db_path();
    match fs::metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        Ok(metadata) if !metadata.is_file() => return Err("policy store is not a file".into()),
        Ok(_) => {}
    }
    // Store::open also initializes/migrates. Lifecycle availability must not do
    // either. This count deliberately mirrors guardrail_count's stored count,
    // not load_guardrails' expiry, disabled, timing or per-action scope filters.
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| error.to_string())?;
    conn.busy_timeout(Duration::from_millis(25))
        .map_err(|error| error.to_string())?;
    let count = conn.query_row(
        "SELECT COUNT(*) FROM triples WHERE p IN ('forbids','must_not','never','always','requires','enforces','prefers')",
        [], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    let scan: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'last_scan'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(Some((count, scan.and_then(|value| value.parse().ok()))))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u8,
    platform: String,
    event: String,
    outcome: String,
    observed_at_unix_ms: u64,
    version: String,
    executable: String,
}

fn valid_outcome(outcome: &str) -> bool {
    matches!(
        outcome,
        "handled"
            | "allow"
            | "deny"
            | "error"
            | "disabled"
            | "bypassed"
            | "skipped"
            | "uninitialized"
    )
}

fn event_file(event: &str) -> Option<&'static str> {
    EVENTS
        .iter()
        .find_map(|(name, file)| (*name == event).then_some(*file))
}

fn receipt_dir(cfg: &Config, platform: Platform) -> PathBuf {
    cfg.arai_base_dir
        .join("projects")
        .join(cfg.project_slug())
        .join("hook-invocations")
        .join(platform.id())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

/// Record a bounded, best-effort CLI invocation receipt without policy changes.
/// Only canonical startup/pre/post events and fixed outcome labels are accepted.
/// Errors never change the hook decision or interrupt its response.
pub fn record_invocation(cfg: &Config, platform: Platform, event: &str, outcome: &str) {
    let Some(file) = event_file(event) else {
        return;
    };
    if !valid_outcome(outcome) {
        return;
    }
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let executable = executable.to_string_lossy().into_owned();
    if executable.len() > 4096 || executable.chars().any(char::is_control) {
        return;
    }
    let receipt = Receipt {
        schema: 1,
        platform: platform.id().into(),
        event: event.into(),
        outcome: outcome.into(),
        observed_at_unix_ms: now_millis(),
        version: env!("CARGO_PKG_VERSION").into(),
        executable,
    };
    let _ = write_receipt(&receipt_dir(cfg, platform), file, &receipt);
}

struct ReceiptLock(File);

impl Drop for ReceiptLock {
    fn drop(&mut self) {
        // Explicit unlock also prevents briefly inherited fork descriptors from
        // retaining the lock after this writer is done.
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

fn private_file(path: &Path, create_new: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true);
    if create_new {
        options.create_new(true);
    } else {
        options.create(true).truncate(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn write_receipt(directory: &Path, stem: &str, receipt: &Receipt) -> std::io::Result<()> {
    fs::create_dir_all(directory)?;
    let lock_path = directory.join(format!(".{stem}.lock"));
    if fs::symlink_metadata(&lock_path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(std::io::Error::other("receipt lock must not be a symlink"));
    }
    let lock = private_file(&lock_path, false)?;
    fs2::FileExt::try_lock_exclusive(&lock)?;
    let _lock = ReceiptLock(lock);
    let path = directory.join(format!("{stem}.json"));
    // A slow earlier invocation must not replace a more recent receipt.
    if let Ok(previous) = read_receipt(&path) {
        if previous.observed_at_unix_ms > receipt.observed_at_unix_ms {
            return Ok(());
        }
    }
    let temporary = directory.join(format!(".{stem}.tmp"));
    match fs::remove_file(&temporary) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let result = (|| {
        let bytes = serde_json::to_vec(receipt).map_err(std::io::Error::other)?;
        if bytes.len() as u64 > MAX_RECEIPT_BYTES {
            return Err(std::io::Error::other("receipt exceeds size limit"));
        }
        let mut file = private_file(&temporary, true)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn read_receipt(path: &Path) -> std::io::Result<Receipt> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_RECEIPT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err(std::io::Error::other("oversize receipt"));
    }
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}

/// Summarize local adapter evidence without executing a configured command or
/// inferring host trust. Missing/corrupt receipts never imply successful hooks.
pub fn invocation_summary(cfg: &Config, platform: Platform) -> String {
    let directory = receipt_dir(cfg, platform);
    let mut summaries = Vec::new();
    for (event, stem) in EVENTS {
        let path = directory.join(format!("{stem}.json"));
        match read_receipt(&path) {
            Ok(receipt)
                if receipt.schema == 1
                    && receipt.platform == platform.id()
                    && receipt.event == *event
                    && valid_outcome(&receipt.outcome)
                    && receipt.version.len() <= 64
                    && !receipt.version.chars().any(char::is_control)
                    && receipt.executable.len() <= 4096
                    && !receipt.executable.chars().any(char::is_control) =>
            {
                summaries.push(format!(
                    "{event}: {} at unix {} (v{})",
                    receipt.outcome,
                    receipt.observed_at_unix_ms / 1000,
                    receipt.version
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => summaries.push(format!("{event}: receipt unreadable")),
        }
    }
    if summaries.is_empty() {
        "Observed adapter invocations: none recorded; host activation unverified".into()
    } else {
        format!(
            "Observed adapter invocations: {}; host activation unverified",
            summaries.join("; ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parser, store::Store};
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(Config);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "arai_lifecycle_{}_{}_{}",
                std::process::id(),
                now_millis(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("project")).unwrap();
            Self(Config {
                project_root: root.join("project"),
                home_dir: root.join("home"),
                arai_base_dir: root.join("state"),
                extra_sources: vec![],
                guardrails_mode: "advise".into(),
                llm_command: None,
                api_url: None,
                api_key_env: None,
                api_model: None,
            })
        }
        fn seed(&self) {
            let db = Store::open(&self.0.db_path()).unwrap();
            let text = "- Never run cargo clean\n";
            db.upsert_file(
                "accepted://policy",
                text,
                &parser::extract_rules(text, "agents_md", 0.9),
                "agents_md",
            )
            .unwrap();
            db.set_meta("last_scan", "1").unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(self.0.project_root.parent().unwrap()).ok();
        }
    }

    #[test]
    fn missing_store_startup_never_initializes_policy_or_hooks() {
        let f = Fixture::new();
        let output = startup_with_mode(&f.0, Platform::Claude, "SessionStart", Mode::Enforcing);
        assert!(output["systemMessage"]
            .as_str()
            .unwrap()
            .contains("policy store missing"));
        assert_eq!(
            output["hookSpecificOutput"]["hookEventName"],
            "SessionStart"
        );
        assert!(output["hookSpecificOutput"]
            .get("permissionDecision")
            .is_none());
        assert!(!f.0.arai_base_dir.exists());
        assert!(!f.0.project_root.join(".claude").exists());
    }

    #[test]
    fn lifecycle_shapes_match_context_capabilities() {
        let f = Fixture::new();
        for host in [Platform::Claude, Platform::Codex] {
            let output = startup_with_mode(&f.0, host, "SubagentStart", Mode::Enforcing);
            assert!(output.get("systemMessage").is_none());
            assert_eq!(
                output["hookSpecificOutput"]["hookEventName"],
                "SubagentStart"
            );
            assert!(output["hookSpecificOutput"]["additionalContext"].is_string());
        }
        let output = startup_with_mode(&f.0, Platform::Grok, "SessionStart", Mode::Enforcing);
        assert!(output["systemMessage"].is_string());
        assert!(output.get("hookSpecificOutput").is_none());
        assert_eq!(
            startup_with_mode(&f.0, Platform::Grok, "SubagentStart", Mode::Enforcing),
            json!({})
        );
        assert_eq!(
            startup_with_mode(&f.0, Platform::Cursor, "SessionStart", Mode::Enforcing),
            json!({})
        );
        assert_eq!(
            startup_with_mode(&f.0, Platform::Claude, "PreToolUse", Mode::Enforcing),
            json!({})
        );
    }

    #[test]
    fn startup_reads_accepted_state_without_scan_model_network_or_migration() {
        let mut f = Fixture::new();
        f.seed();
        fs::write(
            f.0.project_root.join("AGENTS.md"),
            "<!-- arai:extends https://example.invalid/not-fetched -->\n- Never run git push\n",
        )
        .unwrap();
        let model = f.0.arai_base_dir.join("models/all-MiniLM-L6-v2");
        fs::create_dir_all(&model).unwrap();
        fs::write(model.join("model.onnx"), "invalid model must not be loaded").unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        f.0.api_url = Some(format!("http://{}", listener.local_addr().unwrap()));
        f.0.llm_command = Some("this-command-must-never-run".into());
        let conn = Connection::open(f.0.db_path()).unwrap();
        conn.execute_batch("PRAGMA user_version=4;").unwrap();
        let output = startup_with_mode(&f.0, Platform::Codex, "SessionStart", Mode::Enforcing);
        let text = output["systemMessage"].as_str().unwrap();
        assert!(text.contains("1 stored guardrail(s)"));
        assert!(text.contains("disk freshness not checked"));
        assert!(text.contains("Tool-hook activation is not proven"));
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            4
        );
        assert_eq!(
            conn.query_row("SELECT value FROM meta WHERE key='last_scan'", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
            "1"
        );
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert!(!f.0.arai_base_dir.join("cache").exists());
        assert!(!f.0.project_root.join(".codex").exists());
    }

    #[test]
    fn mode_and_unavailable_store_are_explicit_without_control_output() {
        let f = Fixture::new();
        fs::create_dir_all(f.0.db_path().parent().unwrap()).unwrap();
        fs::write(f.0.db_path(), b"broken SQLite").unwrap();
        for (disabled, deny, expected) in [
            ("YES", "", "disabled"),
            ("", "OFF", "advisory"),
            ("false", "true", "deny mode enabled"),
        ] {
            let output = startup_with_mode(
                &f.0,
                Platform::Claude,
                "SessionStart",
                Mode::from_values(disabled, deny),
            );
            let text = output["systemMessage"].as_str().unwrap();
            assert!(text.contains(expected));
            assert!(text.contains("policy store unavailable"));
            assert!(output.get("decision").is_none());
        }
        assert_eq!(fs::read(f.0.db_path()).unwrap(), b"broken SQLite");
    }

    #[test]
    fn invocation_receipts_are_bounded_and_do_not_create_policy_store() {
        let f = Fixture::new();
        assert!(invocation_summary(&f.0, Platform::Claude).contains("none recorded"));
        assert!(!f.0.arai_base_dir.exists());
        record_invocation(&f.0, Platform::Claude, "../escape", "handled");
        record_invocation(&f.0, Platform::Claude, "PreToolUse", "contents of an input");
        assert!(!f.0.arai_base_dir.exists());
        for (event, _) in EVENTS {
            record_invocation(&f.0, Platform::Claude, event, "handled");
            record_invocation(&f.0, Platform::Claude, event, "allow");
        }
        assert!(!f.0.db_path().exists());
        let directory = receipt_dir(&f.0, Platform::Claude);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), EVENTS.len() * 2);
        let receipt = read_receipt(&directory.join("pre-tool-use.json")).unwrap();
        assert_eq!(receipt.outcome, "allow");
        assert_eq!(
            receipt.executable,
            std::env::current_exe().unwrap().to_string_lossy()
        );
        let summary = invocation_summary(&f.0, Platform::Claude);
        assert!(summary.contains("PreToolUse: allow"));
        assert!(summary.contains("host activation unverified"));
        assert!(invocation_summary(&f.0, Platform::Grok).contains("none recorded"));
    }

    #[test]
    fn busy_or_bad_receipts_do_not_claim_success_or_change_existing_state() {
        let f = Fixture::new();
        record_invocation(&f.0, Platform::Grok, "PreToolUse", "deny");
        let directory = receipt_dir(&f.0, Platform::Grok);
        let lock = private_file(&directory.join(".pre-tool-use.lock"), false).unwrap();
        fs2::FileExt::lock_exclusive(&lock).unwrap();
        record_invocation(&f.0, Platform::Grok, "PreToolUse", "allow");
        assert_eq!(
            read_receipt(&directory.join("pre-tool-use.json"))
                .unwrap()
                .outcome,
            "deny"
        );
        fs2::FileExt::unlock(&lock).unwrap();
        fs::write(
            directory.join("pre-tool-use.json"),
            vec![b' '; MAX_RECEIPT_BYTES as usize + 1],
        )
        .unwrap();
        assert!(invocation_summary(&f.0, Platform::Grok).contains("receipt unreadable"));
        fs::write(directory.join(".pre-tool-use.tmp"), "interrupted write").unwrap();
        record_invocation(&f.0, Platform::Grok, "PreToolUse", "error");
        assert_eq!(
            read_receipt(&directory.join("pre-tool-use.json"))
                .unwrap()
                .outcome,
            "error"
        );
        assert!(!directory.join(".pre-tool-use.tmp").exists());
    }
}
