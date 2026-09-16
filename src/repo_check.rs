//! Repo-layer enforcement: run the live matcher against a git diff.
//!
//! Universal across AI tools because it sits at the repo, not the host hook
//! surface.  Added files synthesise a `Write` call; modified / renamed files
//! synthesise an `Edit`. Renames also check the source and destination file
//! actions; copies count as additions. Deleted files are reported but not matched (no
//! schema for "this path went away" yet — see `docs/repo-layer-scope.md`).
//!
//! Pure — no audit write, no telemetry.  The CLI owns git invocation, stdout,
//! and the exit code.

use crate::config::Config;
use crate::hooks;
use crate::intent::Severity;
use crate::store::{Guardrail, Store};
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
    /// Primary canonical tool (`Write` / `Edit`), empty when skipped.
    /// Renames additionally check source Edit and destination Write scopes.
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

struct DiffFile {
    change: FileChange,
    old_header: bool,
    new_header: bool,
    has_hunk: bool,
    has_metadata: bool,
    binary: bool,
    copied: bool,
}

impl DiffFile {
    fn finish(self) -> Result<FileChange, String> {
        if self.old_header != self.new_header
            || (self.old_header && !self.has_hunk)
            || (!self.has_hunk && !self.has_metadata)
        {
            return Err(format!("incomplete diff for {}", self.change.path));
        }
        Ok(self.change)
    }
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
    let mut current: Option<DiffFile> = None;
    // Remaining old/new lines in the current hunk. Header-looking content
    // must be consumed here before any file-level metadata is considered.
    let mut hunk: Option<(usize, usize)> = None;
    let mut last_was_content = false;

    for (index, line) in diff.lines().enumerate() {
        let invalid = || format!("invalid or truncated diff at line {}", index + 1);
        if line == "\\ No newline at end of file" && last_was_content {
            last_was_content = false;
            continue;
        }
        if let Some((old_left, new_left)) = hunk.as_mut() {
            if *old_left != 0 || *new_left != 0 {
                let c = &mut current.as_mut().ok_or_else(invalid)?.change;
                match line.as_bytes().first() {
                    Some(b'+') if *new_left > 0 => {
                        *new_left -= 1;
                        c.new_content.push_str(&line[1..]);
                        c.new_content.push('\n');
                    }
                    Some(b'-') if *old_left > 0 => {
                        *old_left -= 1;
                        c.old_content.push_str(&line[1..]);
                        c.old_content.push('\n');
                    }
                    Some(b' ') if *old_left > 0 && *new_left > 0 => {
                        *old_left -= 1;
                        *new_left -= 1;
                        c.old_content.push_str(&line[1..]);
                        c.old_content.push('\n');
                        c.new_content.push_str(&line[1..]);
                        c.new_content.push('\n');
                    }
                    _ => return Err(invalid()),
                }
                last_was_content = true;
                continue;
            }
        }
        last_was_content = false;
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if let Some(c) = current.take() {
                files.push(c.finish()?);
            }
            let (old, new) = parse_git_paths(rest)?;
            current = Some(DiffFile {
                change: FileChange {
                    path: new,
                    old_path: Some(old),
                    kind: ChangeKind::Modified,
                    old_content: String::new(),
                    new_content: String::new(),
                },
                old_header: false,
                new_header: false,
                has_hunk: false,
                has_metadata: false,
                binary: false,
                copied: false,
            });
            hunk = None;
            continue;
        }
        let Some(c) = current.as_mut() else {
            if line.trim().is_empty() {
                continue;
            }
            return Err(invalid());
        };
        if c.binary {
            // Git's base85 binary body has no textual content to match.
            // The next diff --git header is still handled above.
            continue;
        }
        if line.starts_with("@@") {
            if !c.old_header || !c.new_header {
                return Err(invalid());
            }
            let counts = parse_hunk_header(line).ok_or_else(invalid)?;
            if (c.change.kind == ChangeKind::Added && !c.copied && counts.0 != 0)
                || (c.change.kind == ChangeKind::Deleted && counts.1 != 0)
            {
                return Err(invalid());
            }
            hunk = Some(counts);
            c.has_hunk = true;
            continue;
        }
        // Once hunks have started, metadata/header syntax is no longer
        // valid until the next file. Extra +/- lines are malformed, not
        // alternative headers that may rewrite the path being enforced.
        if c.has_hunk {
            return Err(invalid());
        }
        if line.starts_with("new file mode ") {
            c.change.kind = ChangeKind::Added;
            c.has_metadata = true;
        } else if line.starts_with("deleted file mode ") {
            c.change.kind = ChangeKind::Deleted;
            c.has_metadata = true;
        } else if let Some(path) = line.strip_prefix("rename from ") {
            c.change.old_path = Some(decode_git_path(path)?);
            c.change.kind = ChangeKind::Renamed;
            c.has_metadata = true;
        } else if let Some(path) = line.strip_prefix("rename to ") {
            c.change.path = decode_git_path(path)?;
            c.has_metadata = true;
        } else if let Some(path) = line.strip_prefix("copy from ") {
            c.change.old_path = Some(decode_git_path(path)?);
            c.change.kind = ChangeKind::Added;
            c.copied = true;
            c.has_metadata = true;
        } else if let Some(path) = line.strip_prefix("copy to ") {
            c.change.path = decode_git_path(path)?;
            c.change.kind = ChangeKind::Added;
            c.copied = true;
            c.has_metadata = true;
        } else if let Some(path) = line.strip_prefix("+++ ") {
            if !c.old_header || c.new_header {
                return Err(invalid());
            }
            c.new_header = true;
            if path == "/dev/null" {
                c.change.kind = ChangeKind::Deleted;
            } else {
                c.change.path = strip_diff_prefix(&decode_git_path(path)?, "b/")?;
            }
        } else if let Some(path) = line.strip_prefix("--- ") {
            if c.old_header {
                return Err(invalid());
            }
            c.old_header = true;
            if path == "/dev/null" {
                c.change.kind = ChangeKind::Added;
            } else {
                c.change.old_path = Some(strip_diff_prefix(&decode_git_path(path)?, "a/")?);
            }
        } else if line.starts_with("index ")
            || line.starts_with("old mode ")
            || line.starts_with("new mode ")
            || line.starts_with("similarity index ")
            || line.starts_with("dissimilarity index ")
        {
            c.has_metadata = true;
        } else if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            c.binary = true;
            c.has_metadata = true;
        } else {
            return Err(invalid());
        }
    }
    if hunk.is_some_and(|(old, new)| old != 0 || new != 0) {
        return Err("truncated diff hunk at end of input".to_string());
    }
    if let Some(c) = current {
        files.push(c.finish()?);
    }
    Ok(files)
}

/// Match a unified diff against the live guardrail set.
///
/// Deleted files are skipped (reported with `skipped: true`).  Everything
/// else is synthesised as PreToolUse file actions and run through
/// [`hooks::match_hook`]. Renames check both paths and both destination
/// scopes, so moving a scratch file cannot bypass a creation prohibition.
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

        let mut actions = vec![(tool, tool_input)];
        if change.kind == ChangeKind::Renamed {
            actions.push((
                "Write",
                json!({"file_path": change.path, "content": change.new_content}),
            ));
            if let Some(old_path) = &change.old_path {
                if old_path != &change.path {
                    actions.push((
                        "Edit",
                        json!({
                            "file_path": old_path,
                            "old_string": change.old_content,
                            "new_string": "",
                        }),
                    ));
                }
            }
        }
        let mut matches: Vec<(Guardrail, u8)> = Vec::new();
        for (action_tool, action_input) in actions {
            let hook = json!({
                "hook_event_name": "PreToolUse",
                "tool_name": action_tool,
                "tool_input": action_input,
                "cwd": cfg.project_root,
            });
            for (guard, score) in hooks::match_hook(&hook, cfg, db)?.matched {
                if let Some((_, previous)) = matches
                    .iter_mut()
                    .find(|(previous, _)| previous.triple_id == guard.triple_id)
                {
                    *previous = (*previous).max(score);
                } else {
                    matches.push((guard, score));
                }
            }
        }
        matches.sort_by_key(|(_, score)| std::cmp::Reverse(*score));
        let top = hooks::highest_severity(&matches);
        if top == Severity::Block && !matches.is_empty() {
            blocked = true;
        }
        let matched = matches
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
        let severity = if matches.is_empty() {
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

fn parse_hunk_header(line: &str) -> Option<(usize, usize)> {
    let ranges = line.strip_prefix("@@ -")?;
    let (old, rest) = ranges.split_once(" +")?;
    let (new, context) = rest.split_once(" @@")?;
    if !context.is_empty() && !context.starts_with(' ') {
        return None;
    }
    let range_count = |range: &str| {
        let (start, count) = range.split_once(',').unwrap_or((range, "1"));
        if start.is_empty()
            || count.is_empty()
            || !start.bytes().all(|b| b.is_ascii_digit())
            || !count.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let start = start.parse::<usize>().ok()?;
        let count = count.parse::<usize>().ok()?;
        start.checked_add(count)?;
        (count == 0 || start > 0).then_some(count)
    };
    let old = range_count(old)?;
    let new = range_count(new)?;
    (old != 0 || new != 0).then_some((old, new))
}

fn parse_git_paths(rest: &str) -> Result<(String, String), String> {
    let invalid = || format!("could not parse diff --git paths: {rest}");
    let (old, new) = if rest.starts_with('"') {
        let (old, remaining) = decode_quoted_path(rest)?;
        let new = decode_git_path(remaining.strip_prefix(' ').ok_or_else(invalid)?)?;
        (old, new)
    } else {
        // Git may quote either side independently, and leaves spaces in
        // ordinary filenames unquoted. File headers/rename metadata later
        // supply the unambiguous names for paths containing " b/".
        let idx = rest
            .find(" \"b/")
            .or_else(|| rest.find(" b/"))
            .ok_or_else(invalid)?;
        (
            decode_git_path(&rest[..idx])?,
            decode_git_path(&rest[idx + 1..])?,
        )
    };
    Ok((
        strip_diff_prefix(&old, "a/")?,
        strip_diff_prefix(&new, "b/")?,
    ))
}

fn strip_diff_prefix(path: &str, prefix: &str) -> Result<String, String> {
    path.strip_prefix(prefix)
        .filter(|p| !p.is_empty())
        .map(String::from)
        .ok_or_else(|| format!("expected {prefix} diff path: {path}"))
}

fn decode_git_path(path: &str) -> Result<String, String> {
    let decoded = if path.starts_with('"') {
        let (decoded, trailing) = decode_quoted_path(path)?;
        if !trailing.is_empty() && !trailing.starts_with('\t') {
            return Err("unexpected text after quoted diff path".to_string());
        }
        decoded
    } else {
        path.split('\t').next().unwrap_or(path).to_string()
    };
    if decoded.is_empty() || decoded.contains('\0') {
        return Err("empty or NUL-containing diff path".to_string());
    }
    Ok(decoded)
}

/// Git uses C quoting, including octal UTF-8 bytes, rather than JSON
/// escaping. Decode bytes first so non-ASCII paths retain their identity.
fn decode_quoted_path(path: &str) -> Result<(String, &str), String> {
    let invalid = || "invalid quoted diff path".to_string();
    if !path.starts_with('"') {
        return Err(invalid());
    }
    let bytes = path.as_bytes();
    let mut decoded = Vec::new();
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let value = String::from_utf8(decoded)
                    .map_err(|_| "diff path is not valid UTF-8".to_string())?;
                if value.is_empty() || value.contains('\0') {
                    return Err(invalid());
                }
                return Ok((value, &path[i + 1..]));
            }
            b'\\' => {
                i += 1;
                let escaped = *bytes.get(i).ok_or_else(invalid)?;
                let value = match escaped {
                    b'"' | b'\\' => escaped,
                    b'a' => 7,
                    b'b' => 8,
                    b't' => b'\t',
                    b'n' => b'\n',
                    b'v' => 11,
                    b'f' => 12,
                    b'r' => b'\r',
                    b'0'..=b'7' => {
                        let octal = bytes.get(i..i + 3).ok_or_else(invalid)?;
                        if !octal.iter().all(|b| matches!(b, b'0'..=b'7')) {
                            return Err(invalid());
                        }
                        let value = u16::from(octal[0] - b'0') * 64
                            + u16::from(octal[1] - b'0') * 8
                            + u16::from(octal[2] - b'0');
                        i += 2;
                        u8::try_from(value).map_err(|_| invalid())?
                    }
                    _ => return Err(invalid()),
                };
                decoded.push(value);
            }
            byte => decoded.push(byte),
        }
        i += 1;
    }
    Err(invalid())
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
@@ -1,3 +1,3 @@
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
    fn header_looking_hunk_lines_remain_content() {
        let diff = "diff --git a/alembic/notes.txt b/alembic/notes.txt\n\
--- a/alembic/notes.txt\n\
+++ b/alembic/notes.txt\n\
@@ -1,2 +1,2 @@\n\
--- a/old-header-lookalike\n\
-old\n\
+++ b/new-header-lookalike\n\
+new\n";
        let files = parse_unified_diff(diff).unwrap();
        assert_eq!(files[0].path, "alembic/notes.txt");
        assert_eq!(files[0].old_path.as_deref(), Some("alembic/notes.txt"));
        assert_eq!(files[0].old_content, "-- a/old-header-lookalike\nold\n");
        assert_eq!(files[0].new_content, "++ b/new-header-lookalike\nnew\n");
    }

    #[test]
    fn parse_multiple_hunks_and_files() {
        let diff =
            format!("{MODIFIED}@@ -10,0 +11,2 @@ def another_function():\n+one\n+two\n{ADDED}");
        let files = parse_unified_diff(&diff).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files[0].new_content.ends_with("one\ntwo\n"));
        assert_eq!(files[1].kind, ChangeKind::Added);
    }

    #[test]
    fn parse_no_newline_markers_without_counting_them() {
        let diff = "diff --git a/foo b/foo\n--- a/foo\n+++ b/foo\n\
@@ -1 +1 @@\n-old\n\\ No newline at end of file\n\
+new\n\\ No newline at end of file\n";
        let files = parse_unified_diff(diff).unwrap();
        assert_eq!(files[0].old_content, "old\n");
        assert_eq!(files[0].new_content, "new\n");
    }

    #[test]
    fn parse_git_c_quoted_paths_and_independently_quoted_sides() {
        let escaped = r#"alembic/caf\303\251\t\".txt"#;
        let diff = format!(
            "diff --git \"a/{escaped}\" \"b/{escaped}\"\nnew file mode 100644\n\
--- /dev/null\n+++ \"b/{escaped}\"\n@@ -0,0 +1 @@\n+hello\n"
        );
        let files = parse_unified_diff(&diff).unwrap();
        assert_eq!(files[0].path, "alembic/caf\u{e9}\t\".txt");
        assert_eq!(
            parse_git_paths(r#"a/old.txt "b/new\tname.txt""#).unwrap(),
            ("old.txt".to_string(), "new\tname.txt".to_string())
        );
        assert_eq!(
            parse_git_paths(r#""a/old\tname.txt" b/new file.txt"#).unwrap(),
            ("old\tname.txt".to_string(), "new file.txt".to_string())
        );
    }

    #[test]
    fn parse_metadata_only_and_binary_changes() {
        let diff = "diff --git a/empty b/empty\nnew file mode 100644\nindex 000..111\n\
diff --git a/image.png b/image.png\nindex 111..222 100644\n\
Binary files a/image.png and b/image.png differ\n\
diff --git a/script b/script\nold mode 100644\nnew mode 100755\n\
diff --git a/old.txt b/new.txt\nsimilarity index 100%\n\
rename from old.txt\nrename to new.txt\n";
        let files = parse_unified_diff(diff).unwrap();
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].kind, ChangeKind::Added);
        assert!(files[1].new_content.is_empty());
        assert_eq!(files[2].path, "script");
        assert_eq!(files[3].kind, ChangeKind::Renamed);
        assert_eq!(files[3].path, "new.txt");
    }

    #[test]
    fn malformed_or_unsupported_nonempty_diffs_fail_closed() {
        for diff in [
            "not a diff\n".to_string(),
            "diff --cc foo\n".to_string(),
            "diff --git a/foo b/foo\n".to_string(),
            ADDED.replace("+def upgrade():\n", ""),
            format!("{ADDED}+one extra line\n"),
            ADDED.replace("@@ -0,0 +1,3 @@", "@@ -0,0 +1,4 @@"),
            ADDED.replace("@@ -0,0 +1,3 @@", "@@@ -0,0 +1,3 @@@"),
            ADDED.replace("@@ -0,0 +1,3 @@", "@@ -0,0 +1,wat @@"),
            ADDED.replace("--- /dev/null\n", ""),
            ADDED.replace("--- /dev/null\n", "--- /dev/null\n--- /dev/null\n"),
            "diff --git a/foo b/foo\n--- a/foo\n+++ b/foo\n".to_string(),
            format!("{}{}", ADDED.replace("+def upgrade():\n", ""), MODIFIED),
        ] {
            assert!(parse_unified_diff(&diff).is_err(), "accepted {diff:?}");
        }
        for path in [r#""b/unfinished"#, r#""b/bad\q""#, r#""b/bad\777""#] {
            assert!(decode_git_path(path).is_err(), "accepted {path}");
        }
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
