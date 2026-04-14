use crate::config::Config;
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

    // Project-level instruction files
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
        if let Some(file) = try_read_file(&path, source_type, confidence) {
            files.push(file);
        }
    }

    // Global Claude.md
    let global_claude = cfg.home_dir.join(".claude").join("CLAUDE.md");
    if let Some(file) = try_read_file(&global_claude, "claude_md_global", 0.88) {
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
}
