use crate::config::Config;
use crate::extends;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    pub path: String,
    pub source_type: String,
    pub confidence: f64,
    pub content: String,
    #[allow(dead_code)]
    pub frontmatter: HashMap<String, String>,
}

/// Discover all instruction files for the current project.
pub fn discover(cfg: &Config) -> Result<Vec<DiscoveredFile>, String> {
    let mut files = Vec::new();

    // Modern Cursor uses `.cursor/rules/` as a directory of `.md` / `.mdc`
    // rule files (per-feature splits like `auth.mdc`, `api.mdc`).  Walk it
    // first; the per-file loop below would otherwise see a directory and
    // skip it via `try_read_file`'s read-as-string failure.
    let cursor_rules_dir = cfg.project_root.join(".cursor").join("rules");
    if cursor_rules_dir.is_dir() {
        for mut file in read_cursor_rules_dir(&cursor_rules_dir) {
            file.content = extends::resolve(&file.content, &cfg.arai_base_dir);
            files.push(file);
        }
    }

    // Project-level instruction files.  `.cursor/rules` stays in the list as
    // a fallback for older Cursor installs that wrote it as a single file —
    // when it's a directory the entry above already covered it and
    // `try_read_file` here returns None.
    let project_files: Vec<(PathBuf, &str, f64)> = vec![
        (cfg.project_root.join("CLAUDE.md"), "claude_md_project", 0.92),
        (cfg.project_root.join(".cursor").join("rules"), "cursor_rules", 0.90),
        (cfg.project_root.join(".cursorrules"), "cursor_rules", 0.90),
        (
            cfg.project_root.join(".github").join("copilot-instructions.md"),
            "copilot_instructions",
            0.90,
        ),
        (cfg.project_root.join(".windsurfrules"), "windsurf_rules", 0.90),
    ];

    for (path, source_type, confidence) in project_files {
        if let Some(mut file) = try_read_file(&path, source_type, confidence) {
            file.content = extends::resolve(&file.content, &cfg.arai_base_dir);
            files.push(file);
        }
    }

    // Global Claude.md
    let global_claude = cfg.home_dir.join(".claude").join("CLAUDE.md");
    if let Some(mut file) = try_read_file(&global_claude, "claude_md_global", 0.88) {
        file.content = extends::resolve(&file.content, &cfg.arai_base_dir);
        files.push(file);
    }

    // Claude Code memory files
    let memory_dir = cfg.claude_memory_dir();
    if memory_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&memory_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Some(file) = read_memory_file(&path) {
                        files.push(file);
                    }
                }
            }
        }
    }

    // Extra sources from config
    for extra in &cfg.extra_sources {
        let path = cfg.project_root.join(extra);
        if let Some(file) = try_read_file(&path, "extra", 0.85) {
            files.push(file);
        }
    }

    Ok(files)
}

fn try_read_file(path: &PathBuf, source_type: &str, confidence: f64) -> Option<DiscoveredFile> {
    let content = std::fs::read_to_string(path).ok()?;
    let (frontmatter, _body) = parse_frontmatter(&content);

    Some(DiscoveredFile {
        path: path.to_string_lossy().to_string(),
        source_type: source_type.to_string(),
        confidence,
        content,
        frontmatter,
    })
}

/// Walk `.cursor/rules/` and return one `DiscoveredFile` per `.md` / `.mdc`
/// file inside (recursively).  Modern Cursor splits rules into per-feature
/// files — we treat each as its own source so `arai status` and `arai diff`
/// can attribute rules to the right file.  Uses `ignore::WalkBuilder` so
/// `.gitignore` and friends are respected (a top-level CHANGELOG.md inside
/// `.cursor/rules/` won't accidentally bleed in if it's ignored).
fn read_cursor_rules_dir(dir: &std::path::Path) -> Vec<DiscoveredFile> {
    let mut out = Vec::new();
    let walker = ignore::WalkBuilder::new(dir).build();
    for entry in walker.flatten() {
        let path = entry.path();
        let is_rule_file = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e, "md" | "mdc"))
            .unwrap_or(false);
        if !is_rule_file || !path.is_file() {
            continue;
        }
        if let Some(file) = try_read_file(&path.to_path_buf(), "cursor_rules", 0.90) {
            out.push(file);
        }
    }
    out
}

fn read_memory_file(path: &PathBuf) -> Option<DiscoveredFile> {
    let content = std::fs::read_to_string(path).ok()?;
    let (frontmatter, _body) = parse_frontmatter(&content);

    // Classify by frontmatter type, then filename prefix, then default
    let file_stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let (source_type, confidence) =
        if let Some(fm_type) = frontmatter.get("type") {
            match fm_type.as_str() {
                "feedback" => ("feedback", 0.95),
                "user" => ("user", 0.90),
                "project" => ("project", 0.82),
                "reference" => ("reference", 0.85),
                _ => ("project", 0.82),
            }
        } else if file_stem.starts_with("feedback_") {
            ("feedback", 0.95)
        } else if file_stem.starts_with("user_") {
            ("user", 0.90)
        } else if file_stem.starts_with("project_") {
            ("project", 0.82)
        } else if file_stem.starts_with("reference_") {
            ("reference", 0.85)
        } else {
            ("project", 0.82)
        };

    Some(DiscoveredFile {
        path: path.to_string_lossy().to_string(),
        source_type: source_type.to_string(),
        confidence,
        content,
        frontmatter,
    })
}

/// Parse YAML-like frontmatter from markdown.
/// Returns (frontmatter_map, body_without_frontmatter).
pub fn parse_frontmatter(content: &str) -> (HashMap<String, String>, String) {
    let mut map = HashMap::new();

    if !content.starts_with("---") {
        return (map, content.to_string());
    }

    // Find closing ---
    let rest = &content[3..];
    let closing = rest.find("\n---");
    if let Some(pos) = closing {
        let fm_block = &rest[..pos];
        let body_start = pos + 4; // skip \n---
        let body = if body_start < rest.len() {
            rest[body_start..].trim_start_matches('\n').to_string()
        } else {
            String::new()
        };

        // Parse simple key: value pairs
        for line in fm_block.lines() {
            let line = line.trim();
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim().to_string();
                let value = value.trim().trim_matches('"').trim_matches('\'').to_string();
                if !key.is_empty() && !value.is_empty() {
                    map.insert(key, value);
                }
            }
        }

        (map, body)
    } else {
        (map, content.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter() {
        let content = "---\nname: Deploy rules\ntype: feedback\n---\n\n- Never force-push";
        let (fm, body) = parse_frontmatter(content);
        assert_eq!(fm.get("name").unwrap(), "Deploy rules");
        assert_eq!(fm.get("type").unwrap(), "feedback");
        assert!(body.contains("Never force-push"));
    }

    #[test]
    fn test_parse_no_frontmatter() {
        let content = "# Just a heading\n\n- Some content";
        let (fm, body) = parse_frontmatter(content);
        assert!(fm.is_empty());
        assert_eq!(body, content);
    }

    #[test]
    fn test_memory_classification_by_filename() {
        let dir = std::env::temp_dir().join("arai_test_memory");
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("feedback_testing.md");
        std::fs::write(&path, "- Don't mock the database").unwrap();

        let file = read_memory_file(&path).unwrap();
        assert_eq!(file.source_type, "feedback");
        assert_eq!(file.confidence, 0.95);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cursor_rules_dir_walks_recursively_for_md_and_mdc() {
        // Build a project root with `.cursor/rules/{a.md, b.mdc, sub/c.md, ignored.txt}`
        // then call `read_cursor_rules_dir` directly and assert all three
        // markdown / mdc files surface (and the .txt does not).
        let id = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("arai_cursor_dir_{id}_{nanos}"));
        let rules = root.join(".cursor").join("rules");
        let sub = rules.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(rules.join("a.md"), "- Never push to main").unwrap();
        std::fs::write(rules.join("b.mdc"), "- Always run tests").unwrap();
        std::fs::write(sub.join("c.md"), "- Never commit secrets").unwrap();
        std::fs::write(rules.join("ignored.txt"), "not a rule file").unwrap();

        let out = read_cursor_rules_dir(&rules);
        let names: Vec<String> = out.iter().map(|f| f.path.clone()).collect();
        assert_eq!(out.len(), 3, "should pick up all 3 .md/.mdc files: {names:?}");
        assert!(names.iter().any(|n| n.ends_with("a.md")), "a.md missing: {names:?}");
        assert!(names.iter().any(|n| n.ends_with("b.mdc")), "b.mdc missing: {names:?}");
        assert!(names.iter().any(|n| n.ends_with("c.md")), "sub/c.md missing: {names:?}");
        assert!(
            !names.iter().any(|n| n.ends_with("ignored.txt")),
            "txt leaked: {names:?}"
        );
        for f in &out {
            assert_eq!(f.source_type, "cursor_rules");
            assert!((f.confidence - 0.90).abs() < 1e-9);
        }

        std::fs::remove_dir_all(&root).ok();
    }
}
