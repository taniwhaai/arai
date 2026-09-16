//! Repo-layer enforcement: run the live matcher against a git diff.
//!
//! Universal across AI tools because it sits at the repo, not the host hook
//! surface.  Added files synthesise a `Write` call; modified / renamed files
//! synthesise an `Edit`.  Deleted files are reported but not matched (no
//! schema for "this path went away" yet — see `docs/repo-layer-scope.md`).
//!
//! Pure — no audit write, no telemetry.  The CLI owns git invocation, stdout,
//! and the exit code.

use crate::config::Config;
use crate::hooks;
use crate::intent::Severity;
use crate::store::Store;
use serde_json::json;

/// Hard cap on a diff piped into [`check_diff`].  Monorepo PRs can be large;
/// 10 MiB is well above a normal commit and keeps the parser from OOM-ing.
const MAX_DIFF_BYTES: usize = 10 * 1024 * 1024;

/// How a path changed in the diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Modified,
    Renamed,
    Deleted,
}

/// One file in a parsed unified diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// Path after the change (the `b/` side).
    pub path: String,
    /// Path before the change, when known.
    pub old_path: Option<String>,
    pub kind: ChangeKind,
    /// Reconstructed old side from hunks (minus + context lines).
    pub old_content: String,
    /// Reconstructed new side from hunks (plus + context lines).
    pub new_content: String,
}

/// One matched rule on a file, shaped for `--json` and human output.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MatchedRule {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<i64>,
    pub match_pct: u8,
    pub severity: String,
}

/// Per-file verdict from [`check_diff`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct FileVerdict {
    pub path: String,
    pub kind: ChangeKind,
    /// Canonical tool the change was synthesised as (`Write` / `Edit`),
    /// empty when the file was skipped.
    pub tool: String,
    pub skipped: bool,
    pub severity: String,
    pub matched: Vec<MatchedRule>,
}

/// Aggregated result of matching every file in a diff.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CheckDiffReport {
    /// True when any file produced a `Block`-severity match.
    pub blocked: bool,
    pub files: Vec<FileVerdict>,
}

/// Parse a unified diff (`git diff`, `git diff --cached`, or stdin) into
/// per-file change descriptors.
pub fn parse_unified_diff(diff: &str) -> Result<Vec<FileChange>, String> {
    if diff.len() > MAX_DIFF_BYTES {
        return Err(format!(
            "diff exceeds {MAX_DIFF_BYTES}-byte cap (got {} bytes)",
            diff.len()
        ));
    }

    let mut files: Vec<FileChange> = Vec::new();
    let mut current: Option<FileChange> = None;

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if let Some(c) = current.take() {
                files.push(c);
            }
            let (old, new) = parse_git_paths(rest)
                .ok_or_else(|| format!("could not parse diff --git paths: {rest}"))?;
            current = Some(FileChange {
                path: new,
                old_path: Some(old),
                kind: ChangeKind::Modified,
                old_content: String::new(),
                new_content: String::new(),
            });
            continue;
        }
        let Some(c) = current.as_mut() else {
            continue;
        };
        if line.starts_with("new file mode") {
            c.kind = ChangeKind::Added;
        } else if line.starts_with("deleted file mode") {
            c.kind = ChangeKind::Deleted;
        } else if line.starts_with("rename from ") {
            c.kind = ChangeKind::Renamed;
        } else if let Some(path) = line.strip_prefix("+++ ") {
            if path != "/dev/null" {
                c.path = strip_diff_prefix(path);
            }
        } else if let Some(path) = line.strip_prefix("--- ") {
            if path != "/dev/null" {
                c.old_path = Some(strip_diff_prefix(path));
            }
        } else if line.starts_with("@@")
            || line.starts_with("index ")
            || line.starts_with("similarity ")
            || line.starts_with("dissimilarity ")
            || line.starts_with("rename to ")
            || line.starts_with("copy ")
            || line.starts_with('\\')
        {
            continue;
        } else if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            // Binary: keep the path, drop content.  Matching still sees the
            // path (alembic-style directory rules); content sniffing is empty.
            c.old_content.clear();
            c.new_content.clear();
        } else if let Some(rest) = line.strip_prefix('+') {
            if !line.starts_with("+++") {
                c.new_content.push_str(rest);
                c.new_content.push('\n');
            }
        } else if let Some(rest) = line.strip_prefix('-') {
            if !line.starts_with("---") {
                c.old_content.push_str(rest);
                c.old_content.push('\n');
            }
        } else if let Some(rest) = line.strip_prefix(' ') {
            c.new_content.push_str(rest);
            c.new_content.push('\n');
            c.old_content.push_str(rest);
            c.old_content.push('\n');
        }
    }
    if let Some(c) = current {
        files.push(c);
    }
    Ok(files)
}

/// Match a unified diff against the live guardrail set.
///
/// Deleted files are skipped (reported with `skipped: true`).  Everything
/// else is synthesised as a PreToolUse `Write` or `Edit` and run through
/// [`hooks::match_hook`].
pub fn check_diff(diff: &str, cfg: &Config, db: &Store) -> Result<CheckDiffReport, String> {
    let changes = parse_unified_diff(diff)?;
    let mut files = Vec::with_capacity(changes.len());
    let mut blocked = false;

    for change in changes {
        if change.kind == ChangeKind::Deleted {
            files.push(FileVerdict {
                path: change.path,
                kind: ChangeKind::Deleted,
                tool: String::new(),
                skipped: true,
                severity: Severity::Inform.as_str().to_string(),
                matched: Vec::new(),
            });
            continue;
        }

        let (tool, tool_input) = match change.kind {
            ChangeKind::Added => (
                "Write",
                json!({
                    "file_path": change.path,
                    "content": change.new_content,
                }),
            ),
            ChangeKind::Modified | ChangeKind::Renamed => (
                "Edit",
                json!({
                    "file_path": change.path,
                    "old_string": change.old_content,
                    "new_string": change.new_content,
                }),
            ),
            ChangeKind::Deleted => unreachable!(),
        };

        let hook = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": tool,
            "tool_input": tool_input,
            "session_id": "check-diff",
        });
        let result = hooks::match_hook(&hook, cfg, db)?;
        let top = hooks::highest_severity(&result.matched);
        if top == Severity::Block && !result.matched.is_empty() {
            blocked = true;
        }
        let matched = result
            .matched
            .iter()
            .map(|(g, pct)| {
                let severity = match g.intent.as_ref() {
                    Some(intent) => intent.severity,
                    None => Severity::from_predicate(&g.predicate),
                };
                MatchedRule {
                    subject: g.subject.clone(),
                    predicate: g.predicate.clone(),
                    object: g.object.clone(),
                    source: if g.file_path.is_empty() {
                        g.source_file.clone()
                    } else {
                        g.file_path.clone()
                    },
                    line: g.line_start,
                    match_pct: *pct,
                    severity: severity.as_str().to_string(),
                }
            })
            .collect();
        let severity = if result.matched.is_empty() {
            "allow".to_string()
        } else {
            top.as_str().to_string()
        };
        files.push(FileVerdict {
            path: change.path,
            kind: change.kind,
            tool: tool.to_string(),
            skipped: false,
            severity,
            matched,
        });
    }

    Ok(CheckDiffReport { blocked, files })
}

fn parse_git_paths(rest: &str) -> Option<(String, String)> {
    let rest = rest.trim();
    if rest.starts_with('"') {
        // `"a/foo bar" "b/foo bar"`
        let trimmed = rest.trim_matches('"');
        let (a, b) = trimmed.split_once("\" \"")?;
        return Some((strip_diff_prefix(a), strip_diff_prefix(b)));
    }
    let idx = rest.find(" b/")?;
    let old = strip_diff_prefix(&rest[..idx]);
    let new = strip_diff_prefix(&rest[idx + 1..]);
    Some((old, new))
}

fn strip_diff_prefix(path: &str) -> String {
    let path = path.trim().trim_matches('"');
    let path = path.split('\t').next().unwrap_or(path);
    if let Some(s) = path.strip_prefix("a/") {
        s.to_string()
    } else if let Some(s) = path.strip_prefix("b/") {
        s.to_string()
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDED: &str = "\
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

    const MODIFIED: &str = "\
diff --git a/src/foo.py b/src/foo.py
index 1111111..2222222 100644
--- a/src/foo.py
+++ b/src/foo.py
@@ -1,3 +1,4 @@
 def hello():
-    return 1
+    return 2
     pass
";

    const RENAMED: &str = "\
diff --git a/old.py b/new.py
similarity index 90%
rename from old.py
rename to new.py
index 1111111..2222222 100644
--- a/old.py
+++ b/new.py
@@ -1 +1 @@
-x = 1
+x = 2
";

    const DELETED: &str = "\
diff --git a/gone.py b/gone.py
deleted file mode 100644
index 1111111..0000000
--- a/gone.py
+++ /dev/null
@@ -1 +0,0 @@
-print('bye')
";

    #[test]
    fn parse_added_file() {
        let files = parse_unified_diff(ADDED).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].kind, ChangeKind::Added);
        assert_eq!(files[0].path, "migrations/versions/001_add_users.py");
        assert!(files[0].new_content.contains("from alembic import op"));
        assert!(files[0].old_content.is_empty());
    }

    #[test]
    fn parse_modified_file() {
        let files = parse_unified_diff(MODIFIED).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].kind, ChangeKind::Modified);
        assert_eq!(files[0].path, "src/foo.py");
        assert!(files[0].old_content.contains("return 1"));
        assert!(files[0].new_content.contains("return 2"));
        assert!(files[0].new_content.contains("def hello"));
    }

    #[test]
    fn parse_renamed_file() {
        let files = parse_unified_diff(RENAMED).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].kind, ChangeKind::Renamed);
        assert_eq!(files[0].path, "new.py");
        assert_eq!(files[0].old_path.as_deref(), Some("old.py"));
        assert!(files[0].new_content.contains("x = 2"));
    }

    #[test]
    fn parse_deleted_file() {
        let files = parse_unified_diff(DELETED).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].kind, ChangeKind::Deleted);
        assert_eq!(files[0].path, "gone.py");
        assert!(files[0].old_content.contains("print('bye')"));
    }

    #[test]
    fn parse_multi_file_and_quoted_paths() {
        let diff = "\
diff --git \"a/foo bar.txt\" \"b/foo bar.txt\"
new file mode 100644
--- /dev/null
+++ b/foo bar.txt
@@ -0,0 +1 @@
+hello
";
        let files = parse_unified_diff(diff).unwrap();
        assert_eq!(files[0].path, "foo bar.txt");
        assert_eq!(files[0].kind, ChangeKind::Added);
    }

    #[test]
    fn parse_empty_is_ok() {
        assert!(parse_unified_diff("").unwrap().is_empty());
    }

    #[test]
    fn parse_rejects_oversized_diff() {
        let huge = "x".repeat(MAX_DIFF_BYTES + 1);
        assert!(parse_unified_diff(&huge).is_err());
    }

    #[test]
    fn check_diff_blocks_added_alembic_migration() {
        use crate::config::Config;
        use crate::parser::Triple;
        use crate::store::Store;
        use std::sync::atomic::{AtomicU64, Ordering};

        static N: AtomicU64 = AtomicU64::new(0);
        let id = N.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("arai_repo_check_{}_{}", std::process::id(), id));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Store::open(&dir.join("test.db")).unwrap();
        let triple = Triple {
            subject: "Alembic".to_string(),
            predicate: "never".to_string(),
            object: "hand-write migration files".to_string(),
            confidence: 0.95,
            domain: "test".to_string(),
            source_file: "CLAUDE.md".to_string(),
            line_start: Some(1),
            line_end: Some(1),
            layer: Some(1),
            expires_at: None,
            noenrich: false,
            tier: None,
            source_label: None,
        };
        db.upsert_file(
            "CLAUDE.md",
            "- Never hand-write migration files",
            &[triple],
            "test",
        )
        .unwrap();
        db.classify_all_guardrails().unwrap();
        let cfg = Config {
            project_root: dir.clone(),
            home_dir: dir.clone(),
            arai_base_dir: dir.clone(),
            extra_sources: Vec::new(),
            guardrails_mode: "advise".to_string(),
            llm_command: None,
            api_url: None,
            api_key_env: None,
            api_model: None,
        };
        let report = check_diff(ADDED, &cfg, &db).unwrap();
        assert!(report.blocked, "expected block, got: {:?}", report.files);
        assert_eq!(report.files[0].tool, "Write");
        assert_eq!(report.files[0].kind, ChangeKind::Added);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
