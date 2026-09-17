use crate::config::Config;
use crate::extends;
use std::collections::HashMap;
use std::path::PathBuf;

/// One instruction file found by [`discover`], with its content already
/// read and any `arai:extends` directives resolved.
#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    /// Filesystem path the file was read from.
    pub path: String,
    /// Source-type tag (`claude_md_project`, `cursor_rules`, `agents_md`, …)
    /// used as the extraction-confidence namespace.
    pub source_type: String,
    /// Base extraction confidence for rules from this file.
    pub confidence: f64,
    /// Full file content (post-`extends` resolution).
    pub content: String,
    /// Parsed YAML-ish frontmatter key/value pairs, empty when absent.
    pub frontmatter: HashMap<String, String>,
}

/// Discover instruction sources without flattening their activation scope.
/// Read/walk failures are surfaced: a partial snapshot must not prune policy.
pub fn discover(cfg: &Config) -> Result<Vec<DiscoveredFile>, String> {
    discover_with(cfg, true)
}

/// Same enumeration as [`discover`] but without resolving `arai:extends`.
/// For callers that only need paths and raw content (the session-start
/// change check), so a hook handler never performs a network fetch.
pub fn discover_unresolved(cfg: &Config) -> Result<Vec<DiscoveredFile>, String> {
    discover_with(cfg, false)
}

fn discover_with(cfg: &Config, resolve_extends: bool) -> Result<Vec<DiscoveredFile>, String> {
    let mut files = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Read known root files first for stable precedence and compatibility with
    // ignored local configuration. The recursive walk below adds scoped files.
    for (name, source_type, confidence) in [
        ("CLAUDE.md", "claude_md_project", 0.92),
        ("CLAUDE.local.md", "claude_md_project", 0.92),
        (".claude/CLAUDE.md", "claude_md_project", 0.92),
        ("AGENTS.md", "agents_md", 0.91),
        ("Agents.md", "agents_md", 0.91),
        ("AGENT.md", "agents_md", 0.91),
        ("agents.md", "agents_md", 0.90),
        (".cursor/rules", "cursor_rules", 0.90),
        (".cursorrules", "cursor_rules", 0.90),
        (
            ".github/copilot-instructions.md",
            "copilot_instructions",
            0.90,
        ),
        (".windsurfrules", "windsurf_rules", 0.90),
    ] {
        if let Some(file) = try_read_file(&cfg.project_root.join(name), source_type, confidence)? {
            add_discovered(file, cfg, resolve_extends, &mut files, &mut seen)?;
        }
    }
    for path in walk_instruction_tree(&cfg.project_root, true)? {
        let relative = native_path_string(path.strip_prefix(&cfg.project_root).unwrap_or(&path));
        let Some((source_type, confidence)) = project_source_type(&relative) else {
            continue;
        };
        if let Some(file) = try_read_file(&path, source_type, confidence)? {
            add_discovered(file, cfg, resolve_extends, &mut files, &mut seen)?;
        }
    }

    if let Some(file) = try_read_file(
        &cfg.home_dir.join(".claude/CLAUDE.md"),
        "claude_md_global",
        0.88,
    )? {
        add_discovered(file, cfg, resolve_extends, &mut files, &mut seen)?;
    }
    for path in walk_instruction_tree(&cfg.home_dir.join(".claude/rules"), false)? {
        if path.extension().is_some_and(|extension| extension == "md") {
            if let Some(file) = try_read_file(&path, "claude_rules_global", 0.88)? {
                add_discovered(file, cfg, resolve_extends, &mut files, &mut seen)?;
            }
        }
    }
    for name in ["AGENTS.md", "Agents.md", "AGENT.md", "agents.md"] {
        if let Some(file) = try_read_file(
            &cfg.home_dir.join(".grok").join(name),
            "agents_md_global",
            0.87,
        )? {
            add_discovered(file, cfg, resolve_extends, &mut files, &mut seen)?;
        }
    }
    let memory_dir = cfg.claude_memory_dir();
    if directory_exists(&memory_dir)? {
        let mut entries = std::fs::read_dir(&memory_dir)
            .map_err(|error| format!("Could not read {}: {error}", memory_dir.display()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Could not enumerate {}: {error}", memory_dir.display()))?;
        entries.sort();
        for path in entries {
            if path.extension().is_some_and(|extension| extension == "md") {
                if let Some(file) = read_memory_file(&path)? {
                    add_discovered(file, cfg, false, &mut files, &mut seen)?;
                }
            }
        }
    }
    for extra in &cfg.extra_sources {
        if let Some(file) = try_read_file(&cfg.project_root.join(extra), "extra", 0.85)? {
            add_discovered(file, cfg, false, &mut files, &mut seen)?;
        }
    }
    Ok(files)
}

fn add_discovered(
    mut file: DiscoveredFile,
    cfg: &Config,
    resolve_extends: bool,
    files: &mut Vec<DiscoveredFile>,
    seen: &mut std::collections::HashSet<(String, String)>,
) -> Result<(), String> {
    let path = std::path::Path::new(&file.path);
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        format!(
            "Could not resolve instruction file {}: {error}",
            path.display()
        )
    })?;
    let mut identity = native_path_string(&canonical);
    // Preserve distinct logical aliases under different scoped directories.
    let mut parent = native_path_string(path.parent().unwrap_or(path));
    if cfg!(windows) {
        identity = identity.to_lowercase();
        parent = parent.to_lowercase();
    }
    if !seen.insert((identity, parent)) {
        return Ok(());
    }
    if resolve_extends {
        file.content = extends::resolve(&file.content, &cfg.arai_base_dir);
    }
    let scope = crate::source_scope::SourceScope::from_source(
        &file.path,
        &file.source_type,
        &file.content,
    )?;
    if let Some(reason) = scope.inactive_reason() {
        eprintln!("  [notice] {}: {reason}", file.path);
    }
    files.push(file);
    Ok(())
}

fn project_source_type(path: &str) -> Option<(&'static str, f64)> {
    let basename = path.rsplit('/').next().unwrap_or("");
    let rooted = format!("/{path}");
    #[cfg(windows)]
    let rooted = rooted.to_ascii_lowercase();
    if rooted.contains("/.claude/rules/") && rooted.ends_with(".md") {
        Some(("claude_rules", 0.92))
    } else if rooted.contains("/.cursor/rules/")
        && (rooted.ends_with(".md") || rooted.ends_with(".mdc"))
    {
        Some(("cursor_rules", 0.90))
    } else {
        match basename {
            "CLAUDE.md" | "CLAUDE.local.md" => Some(("claude_md_project", 0.92)),
            "AGENTS.md" | "Agents.md" | "AGENT.md" => Some(("agents_md", 0.91)),
            "agents.md" => Some(("agents_md", 0.90)),
            ".cursorrules" => Some(("cursor_rules", 0.90)),
            ".windsurfrules" => Some(("windsurf_rules", 0.90)),
            "copilot-instructions.md" if rooted.ends_with("/.github/copilot-instructions.md") => {
                Some(("copilot_instructions", 0.90))
            }
            "rules" if rooted.ends_with("/.cursor/rules") => Some(("cursor_rules", 0.90)),
            _ => None,
        }
    }
}

fn native_path_string(path: &std::path::Path) -> String {
    let path = path.to_string_lossy();
    if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.into_owned()
    }
}

fn directory_exists(path: &std::path::Path) -> Result<bool, String> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_dir()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Could not inspect {}: {error}", path.display())),
    }
}

/// Host instruction files can be intentionally gitignored, so don't apply
/// ignore patterns to policy discovery. Explicit generated/VCS directories and
/// nested repositories are pruned; symlinked directories are not traversed.
fn walk_instruction_tree(
    root: &std::path::Path,
    skip_nested_repos: bool,
) -> Result<Vec<PathBuf>, String> {
    if !directory_exists(root)? {
        return Ok(Vec::new());
    }
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .follow_links(false)
        .filter_entry(move |entry| {
            if entry.depth() == 0 {
                return true;
            }
            if !entry.file_type().is_some_and(|kind| kind.is_dir()) {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            !(matches!(
                name.as_ref(),
                ".git"
                    | ".hg"
                    | ".svn"
                    | "node_modules"
                    | "target"
                    | "dist"
                    | "build"
                    | ".venv"
                    | "venv"
                    | "__pycache__"
                    | ".next"
                    | ".svelte-kit"
            ) || skip_nested_repos && entry.path().join(".git").exists())
        })
        .build();
    let mut paths = Vec::new();
    for entry in walker {
        let entry = entry.map_err(|error| {
            format!(
                "Could not enumerate instruction files under {}: {error}",
                root.display()
            )
        })?;
        if entry
            .file_type()
            .is_some_and(|kind| kind.is_file() || kind.is_symlink())
        {
            paths.push(entry.into_path());
        }
    }
    paths.sort();
    Ok(paths)
}

fn try_read_file(
    path: &std::path::Path,
    source_type: &str,
    confidence: f64,
) -> Result<Option<DiscoveredFile>, String> {
    match std::fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() => return Ok(None),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Could not inspect instruction file {}: {error}",
                path.display()
            ))
        }
    }
    let content = std::fs::read_to_string(path).map_err(|error| {
        format!(
            "Could not read instruction file {}: {error}",
            path.display()
        )
    })?;
    let (frontmatter, _body) = parse_frontmatter(&content);
    Ok(Some(DiscoveredFile {
        path: path.to_string_lossy().to_string(),
        source_type: source_type.to_string(),
        confidence,
        content,
        frontmatter,
    }))
}

#[cfg(test)]
fn read_cursor_rules_dir(dir: &std::path::Path) -> Result<Vec<DiscoveredFile>, String> {
    let mut out = Vec::new();
    for path in walk_instruction_tree(dir, false)? {
        if path
            .extension()
            .is_some_and(|extension| extension == "md" || extension == "mdc")
        {
            if let Some(file) = try_read_file(&path, "cursor_rules", 0.90)? {
                out.push(file);
            }
        }
    }
    Ok(out)
}
fn read_memory_file(path: &std::path::Path) -> Result<Option<DiscoveredFile>, String> {
    let Some(mut file) = try_read_file(path, "project", 0.82)? else {
        return Ok(None);
    };
    let frontmatter = &file.frontmatter;

    // Classify by frontmatter type, then filename prefix, then default
    let file_stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let (source_type, confidence) = if let Some(fm_type) = frontmatter.get("type") {
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

    file.source_type = source_type.to_string();
    file.confidence = confidence;
    Ok(Some(file))
}

/// Parse YAML-like frontmatter from markdown.
/// Returns (frontmatter_map, body_without_frontmatter).
pub(crate) fn parse_frontmatter(content: &str) -> (HashMap<String, String>, String) {
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
                let value = value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
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

        let file = read_memory_file(&path).unwrap().unwrap();
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

        let out = read_cursor_rules_dir(&rules).unwrap();
        let names: Vec<String> = out.iter().map(|f| f.path.clone()).collect();
        assert_eq!(
            out.len(),
            3,
            "should pick up all 3 .md/.mdc files: {names:?}"
        );
        assert!(
            names.iter().any(|n| n.ends_with("a.md")),
            "a.md missing: {names:?}"
        );
        assert!(
            names.iter().any(|n| n.ends_with("b.mdc")),
            "b.mdc missing: {names:?}"
        );
        assert!(
            names.iter().any(|n| n.ends_with("c.md")),
            "sub/c.md missing: {names:?}"
        );
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
