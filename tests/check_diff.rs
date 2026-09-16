//! End-to-end tests for `arai check-diff` and `arai init --pre-commit`.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn arai_bin() -> &'static str {
    env!("CARGO_BIN_EXE_arai")
}

fn fresh_env(label: &str) -> (PathBuf, PathBuf, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "arai_check_diff_{label}_{}_{}",
        std::process::id(),
        nanos,
    ));
    let project = root.join("project");
    let home = root.join("home");
    fs::create_dir_all(&project).expect("project");
    let git = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&project)
        .output()
        .expect("git init");
    assert!(git.status.success(), "git init: {:?}", git.stderr);
    fs::create_dir_all(&home).expect("home");
    (root, project, home)
}

fn init_with_alembic(project: &PathBuf, home: &PathBuf) {
    fs::write(
        project.join("CLAUDE.md"),
        // Section context supplies the Alembic subject; the object must not
        // name a `<tool> <subcommand>` phrase (Write has no command phrases,
        // so that gate would drop the match).
        "## Alembic\n\n- Never hand-write migration files\n",
    )
    .unwrap();
    let out = Command::new(arai_bin())
        .arg("init")
        .current_dir(project)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .output()
        .expect("init");
    assert!(
        out.status.success(),
        "init failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

const ALEMBIC_DIFF: &str = "\
diff --git a/migrations/versions/001_add_users.py b/migrations/versions/001_add_users.py
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/migrations/versions/001_add_users.py
@@ -0,0 +1,3 @@
+\"\"\"add users\"\"\"
+from alembic import op
+def upgrade():
";

const README_DIFF: &str = "\
diff --git a/README.md b/README.md
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/README.md
@@ -0,0 +1 @@
+# hello
";

fn run_check_diff(
    project: &PathBuf,
    home: &PathBuf,
    diff: &str,
    extra: &[&str],
) -> (String, String, i32) {
    let mut cmd = Command::new(arai_bin());
    cmd.arg("check-diff")
        .args(extra)
        .current_dir(project)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn check-diff");
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(diff.as_bytes()).expect("write diff");
    }
    let out = child.wait_with_output().expect("wait");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn check_diff_blocks_alembic_handwrite() {
    let (root, project, home) = fresh_env("alembic");
    init_with_alembic(&project, &home);

    let (stdout, stderr, code) = run_check_diff(&project, &home, ALEMBIC_DIFF, &[]);
    assert_eq!(code, 1, "expected block: stdout={stdout} stderr={stderr}");
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.to_lowercase().contains("alembic")
            || combined.contains("BLOCK")
            || combined.contains("migrations"),
        "expected alembic/BLOCK in output: {combined}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn check_diff_allows_unrelated_file() {
    let (root, project, home) = fresh_env("readme");
    init_with_alembic(&project, &home);

    let (stdout, stderr, code) = run_check_diff(&project, &home, README_DIFF, &[]);
    assert_eq!(
        code, 0,
        "unrelated add should pass: stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.is_empty(),
        "no-match path should be silent, got: {stdout}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn check_diff_json_reports_blocked() {
    let (root, project, home) = fresh_env("json");
    init_with_alembic(&project, &home);

    let (stdout, stderr, code) = run_check_diff(&project, &home, ALEMBIC_DIFF, &["--json"]);
    assert_eq!(code, 1, "json block: stdout={stdout} stderr={stderr}");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(v["blocked"], true);
    assert!(v["files"]
        .as_array()
        .map(|a| !a.is_empty())
        .unwrap_or(false));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn check_diff_header_like_content_cannot_rewrite_enforced_path() {
    let (root, project, home) = fresh_env("header_content");
    init_with_alembic(&project, &home);
    let diff = "diff --git a/alembic/notes.txt b/alembic/notes.txt\n\
new file mode 100644\n\
--- /dev/null\n\
+++ b/alembic/notes.txt\n\
@@ -0,0 +1 @@\n\
+++ harmless\n";
    let (stdout, stderr, code) = run_check_diff(&project, &home, diff, &["--json"]);
    assert_eq!(code, 1, "expected block: {stdout} {stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["blocked"], true);
    assert_eq!(report["files"][0]["path"], "alembic/notes.txt");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn check_diff_rejects_truncated_or_unsupported_input() {
    let (root, project, home) = fresh_env("malformed");
    init_with_alembic(&project, &home);
    for diff in [
        ALEMBIC_DIFF.replace("+def upgrade():\n", ""),
        "diff --cc alembic/notes.txt\n".to_string(),
        "--- a/alembic/notes.txt\n+++ b/alembic/notes.txt\n".to_string(),
    ] {
        let (stdout, stderr, code) = run_check_diff(&project, &home, &diff, &["--json"]);
        assert_ne!(code, 0, "malformed input must not pass: {diff:?}");
        assert!(stdout.is_empty(), "must not emit an allow report: {stdout}");
        assert!(stderr.contains("diff"), "missing diagnostic: {stderr}");
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn init_pre_commit_writes_hook() {
    let (root, project, home) = fresh_env("precommit");
    fs::write(project.join("CLAUDE.md"), "- Never force-push to main\n").unwrap();
    let out = Command::new(arai_bin())
        .args(["init", "--pre-commit"])
        .current_dir(&project)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .output()
        .expect("init --pre-commit");
    assert!(
        out.status.success(),
        "init --pre-commit failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let hook = project.join(".git/hooks/pre-commit");
    assert!(hook.is_file(), "missing pre-commit hook");
    let body = fs::read_to_string(&hook).unwrap();
    assert!(body.contains("check-diff --cached"), "hook body: {body}");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn lint_marks_inert_rules() {
    let (root, project, home) = fresh_env("lint");
    let md = project.join("NOTES.md");
    fs::write(&md, "- Never run echo hello_from_lint\n").unwrap();
    let out = Command::new(arai_bin())
        .args(["lint", md.to_str().unwrap()])
        .current_dir(&project)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .output()
        .expect("lint");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("inert") || combined.contains("never fire"),
        "expected inert warning: {combined}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn status_distinguishes_configuration_from_recorded_firings() {
    let (root, project, home) = fresh_env("status");
    init_with_alembic(&project, &home);
    let out = Command::new(arai_bin())
        .arg("status")
        .current_dir(&project)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .output()
        .expect("status");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Last firing: none recorded"),
        "expected last-firing line: {stdout}"
    );
    assert!(stdout.contains(".codex/hooks.json (config present)"));
    assert!(stdout.contains("Host activation/trust is unverified"));
    assert!(stdout.contains("owned handlers"));
    assert!(stdout.contains("startup receipt does not prove tool gating"));

    let _ = fs::remove_dir_all(&root);
}
