//! Integration test for `arai sync`.
//!
//! Sets up a tempdir with a hand-written CLAUDE.md and a minimal
//! arai.toml, runs the built binary, and asserts the managed block
//! was inserted while the hand-written prose was preserved.

use std::process::Command;

fn unique_tempdir(label: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let p = std::env::temp_dir().join(format!(
        "arai-sync-it-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&p).expect("create tempdir");
    p
}

#[test]
fn sync_inserts_managed_block_and_preserves_handwritten_prose() {
    let dir = unique_tempdir("insert");
    let claude = dir.join("CLAUDE.md");
    std::fs::write(
        &claude,
        "# Project rules\n\nHand-written guidance lives here.\n",
    )
    .expect("seed CLAUDE.md");
    let toml = dir.join("arai.toml");
    std::fs::write(
        &toml,
        r#"
[meta]
schema_version = 1
project = "test"

[[rule]]
id = "git-no-force-push"
description = "Migrated from CLAUDE.md:1 — git forbids"
severity = "block"

when = { tool = ["Bash"] }
then = { action = "block", message = "Never force-push to main." }
"#,
    )
    .expect("seed arai.toml");

    let bin = env!("CARGO_BIN_EXE_arai");
    let out = Command::new(bin)
        .arg("sync")
        .arg("--input")
        .arg(&toml)
        .current_dir(&dir)
        .output()
        .expect("spawn arai sync");
    assert!(
        out.status.success(),
        "arai sync exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let body = std::fs::read_to_string(&claude).expect("read CLAUDE.md");
    assert!(
        body.contains("<!-- BEGIN ARAI MANAGED RULES -->"),
        "managed block begin marker missing"
    );
    assert!(
        body.contains("<!-- END ARAI MANAGED RULES -->"),
        "managed block end marker missing"
    );
    assert!(
        body.contains("Never force-push to main."),
        "rule message not rendered"
    );
    assert!(
        body.contains("Hand-written guidance lives here."),
        "hand-written prose was clobbered"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sync_replaces_only_between_markers() {
    let dir = unique_tempdir("replace");
    let claude = dir.join("CLAUDE.md");
    let pre = format!(
        "# Top\nTop matter.\n\n<!-- BEGIN ARAI MANAGED RULES -->\nOLD CONTENT\n<!-- END ARAI MANAGED RULES -->\n\nBottom matter.\n"
    );
    std::fs::write(&claude, &pre).expect("seed CLAUDE.md");
    let toml = dir.join("arai.toml");
    std::fs::write(
        &toml,
        r#"
[meta]
schema_version = 1

[[rule]]
id = "test-rule"
description = "test"
severity = "warn"

when = { tool = ["Bash"] }
then = { action = "warn", message = "NEW CONTENT" }
"#,
    )
    .expect("seed arai.toml");

    let bin = env!("CARGO_BIN_EXE_arai");
    let out = Command::new(bin)
        .arg("sync")
        .arg("--input")
        .arg(&toml)
        .current_dir(&dir)
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let body = std::fs::read_to_string(&claude).expect("read");
    assert!(body.contains("Top matter."), "top hand-edit lost");
    assert!(body.contains("Bottom matter."), "bottom hand-edit lost");
    assert!(body.contains("NEW CONTENT"), "new rule not written");
    assert!(!body.contains("OLD CONTENT"), "old block content survived");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sync_skips_files_that_do_not_exist() {
    let dir = unique_tempdir("skip");
    let toml = dir.join("arai.toml");
    std::fs::write(
        &toml,
        r#"
[meta]
schema_version = 1

[[rule]]
id = "x"
description = "x"
severity = "block"

when = { tool = ["Bash"] }
then = { action = "block", message = "x" }
"#,
    )
    .expect("seed arai.toml");

    // No CLAUDE.md, no AGENTS.md, nothing to update.  Sync should
    // succeed but write nothing.
    let bin = env!("CARGO_BIN_EXE_arai");
    let out = Command::new(bin)
        .arg("sync")
        .arg("--input")
        .arg(&toml)
        .current_dir(&dir)
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("No existing instruction files"),
        "expected guidance about creating instruction files first: {stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
