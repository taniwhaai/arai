//! Persisted source activation metadata. This is separate from public rule
//! structs so embedded callers keep their existing extraction/store contract.
use crate::config::Config;
use globset::GlobBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceScope {
    /// None means project-independent; relative bases resolve from the project.
    directory: Option<String>,
    patterns: Vec<String>,
    inactive: Option<String>,
}

impl SourceScope {
    pub(crate) fn from_source(
        path: &str,
        source_type: &str,
        content: &str,
    ) -> Result<Self, String> {
        // Opaque policy IDs and unknown namespaces belong to the embedder.
        // A local file named by an external policy must not acquire scope merely
        // because its name happens to resemble one of our instruction files.
        if path.contains("://") || source_type == "manual" {
            return Ok(Self::default());
        }
        let normalized = native_separators(path);
        #[cfg(windows)]
        let normalized = normalized.to_ascii_lowercase();
        let basename = normalized.rsplit('/').next().unwrap_or("");
        let global = matches!(
            source_type,
            "claude_md_global" | "agents_md_global" | "claude_rules_global"
        );
        let recognized = global
            || matches!(
                source_type,
                "claude_md_project"
                    | "agents_md"
                    | "claude_rules"
                    | "cursor_rules"
                    | "copilot_instructions"
                    | "windsurf_rules"
            );
        if !recognized {
            return Ok(Self::default());
        }
        let directory = if global {
            None
        } else {
            Some(source_directory(&normalized))
        };
        let mut scope = Self {
            directory,
            ..Self::default()
        };
        let claude_rule = source_type == "claude_rules"
            || source_type == "claude_rules_global"
            || rule_directory(&normalized, ".claude").is_some();
        let cursor_rule =
            source_type == "cursor_rules" && rule_directory(&normalized, ".cursor").is_some();
        if !claude_rule && !cursor_rule {
            scope.validate()?;
            return Ok(scope);
        }
        let metadata = local_scope_content(content)
            .and_then(ScopeMetadata::parse)
            .map_err(|error| format!("Invalid instruction scope in {path}: {error}"))?;
        if cursor_rule {
            if metadata.always_apply == Some(true) {
                scope.validate()?;
                return Ok(scope); // Cursor explicitly ignores globs in this mode.
            }
            let has_activation = metadata.globs.is_some()
                || metadata.always_apply.is_some()
                || metadata.has_description;
            let patterns = metadata.globs.unwrap_or_default();
            if !patterns.is_empty() {
                scope.patterns = validate_patterns(patterns, path)?;
            } else if has_activation || basename.ends_with(".mdc") {
                scope.inactive = Some("Cursor manual/agent-selected rule is not automatically enforced; set alwaysApply: true or globs for deterministic activation".into());
            }
            // Preserve Arai's historic unconditional plain .md rule behavior
            // when it has no activation metadata. Cursor itself uses .mdc.
        } else if let Some(patterns) = metadata.paths {
            if patterns.is_empty() {
                scope.inactive = Some("empty paths scope matches no files".into());
            } else {
                scope.patterns = validate_patterns(patterns, path)?;
            }
        }
        scope.validate()?;
        Ok(scope)
    }

    pub(crate) fn inactive_reason(&self) -> Option<&str> {
        self.inactive.as_deref()
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self
            .directory
            .as_deref()
            .is_some_and(|path| path.chars().any(char::is_control))
        {
            return Err("Instruction scope directory contains control characters".into());
        }
        validate_patterns(self.patterns.clone(), "stored source metadata")?;
        Ok(())
    }

    pub(crate) fn matches(&self, _source_path: &str, cfg: &Config, hook: &Value) -> bool {
        if self.inactive.is_some() {
            return false;
        }
        if self.directory.is_none() && self.patterns.is_empty() {
            return true;
        }
        let input = hook.get("tool_input").or_else(|| hook.get("toolInput"));
        let tool = crate::guardrails::normalize_tool_name(
            hook.get("tool_name")
                .or_else(|| hook.get("toolName"))
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        let project = normalize_path(&cfg.project_root.to_string_lossy());
        let session_cwd = hook
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(|path| resolve_path(path, &project))
            .unwrap_or_else(|| project.clone());
        let cwd = input
            .filter(|_| tool == "Bash")
            .and_then(|input| input.get("workdir").or_else(|| input.get("cwd")))
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(|path| resolve_path(path, &session_cwd))
            .unwrap_or(session_cwd);
        let file = input
            .filter(|_| {
                matches!(
                    tool.as_str(),
                    "Edit" | "Write" | "MultiEdit" | "NotebookEdit"
                )
            })
            .and_then(|input| {
                if tool == "NotebookEdit" {
                    input
                        .get("notebook_path")
                        .or_else(|| input.get("file_path"))
                } else {
                    input.get("file_path")
                }
            })
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty());
        let target = file.map(|file| resolve_path(file, &cwd)).unwrap_or(cwd);
        let base = self
            .directory
            .as_deref()
            .map(|directory| resolve_path(directory, &project))
            .unwrap_or(project);
        let Some(relative) = within_directory(&target, &base) else {
            return false;
        };
        if self.patterns.is_empty() {
            return true;
        }
        // Filename globs require an actual file action. Shell working-directory
        // context supports directory scopes; arbitrary shell targets and cd
        // expressions are deliberately not guessed or promoted to global scope.
        file.is_some()
            && self.patterns.iter().any(|pattern| {
                compile_glob(pattern).is_ok_and(|glob| glob.compile_matcher().is_match(relative))
            })
    }
}

/// Resolver blocks precede the original local bytes. Activation belongs to
/// the importing source, never to a fetched document's frontmatter. Peel only
/// the resolver's complete leading blocks; malformed framing is an error.
fn local_scope_content(content: &str) -> Result<&str, String> {
    let mut remaining = content.trim_start_matches('\u{feff}');
    let mut consumed = false;
    loop {
        let first = remaining.lines().next().unwrap_or("").trim();
        let modern = first.starts_with("<!-- arai:extends-block");
        let legacy = first.starts_with("<!-- arai:extends from");
        if !modern && !legacy {
            break;
        }
        let valid = if modern {
            first
                .strip_prefix("<!-- arai:extends-block url=\"")
                .and_then(|body| body.split_once("\" tier=\""))
                .is_some_and(|(url, tail)| {
                    !url.is_empty()
                        && !url.contains('"')
                        && tail.strip_suffix("\" -->").is_some_and(|tier| {
                            matches!(tier, "peer" | "strict" | "advisory" | "override")
                        })
                })
        } else {
            first
                .strip_prefix("<!-- arai:extends from ")
                .and_then(|body| body.strip_suffix(" -->"))
                .is_some_and(|url| !url.is_empty() && !url.contains(['<', '>']))
        };
        if !valid {
            return Err("Malformed resolved upstream block header".into());
        }
        let mut offset = 0;
        let mut closed = false;
        for (index, line) in remaining.split_inclusive('\n').enumerate() {
            offset += line.len();
            if index == 0 {
                continue;
            }
            let trimmed = line.trim();
            if trimmed.starts_with("<!-- arai:extends-block")
                || trimmed.starts_with("<!-- arai:extends from")
            {
                return Err("Nested resolved upstream block".into());
            }
            if trimmed == "<!-- end arai:extends -->" {
                closed = true;
                break;
            }
        }
        if !closed {
            return Err("Unclosed resolved upstream block".into());
        }
        remaining = remaining[offset..].trim_start_matches(['\n', '\r']);
        consumed = true;
    }
    if (consumed || remaining.starts_with("<!-- end arai:extends"))
        && remaining.lines().any(|line| {
            let line = line.trim();
            line.starts_with("<!-- arai:extends-block")
                || line.starts_with("<!-- arai:extends from")
                || line.starts_with("<!-- end arai:extends")
        })
    {
        return Err("Unexpected resolved upstream block marker in local content".into());
    }
    Ok(remaining)
}

fn compile_glob(pattern: &str) -> Result<globset::Glob, globset::Error> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(true)
        .case_insensitive(cfg!(windows))
        .build()
}

fn validate_patterns(patterns: Vec<String>, path: &str) -> Result<Vec<String>, String> {
    if patterns.len() > 256 {
        return Err(format!(
            "Too many instruction scope patterns in {path} (maximum 256)"
        ));
    }
    patterns
        .into_iter()
        .map(|pattern| {
            let pattern = pattern.strip_prefix("./").unwrap_or(&pattern).to_string();
            if pattern.is_empty()
                || pattern.len() > 4096
                || pattern.chars().any(char::is_control)
                || is_absolute(&pattern)
                || pattern.split('/').any(|part| part == "..")
                || pattern.starts_with('!')
            {
                return Err(format!(
                    "Invalid relative scope pattern in {path}: {pattern:?}"
                ));
            }
            compile_glob(&pattern)
                .map_err(|error| format!("Invalid scope glob in {path}: {error}"))?;
            Ok(pattern)
        })
        .collect()
}

fn rule_directory<'a>(path: &'a str, host: &str) -> Option<&'a str> {
    let prefix = format!("{host}/rules/");
    if path.starts_with(&prefix) {
        return Some("");
    }
    path.find(&format!("/{prefix}")).map(|index| &path[..index])
}

fn source_directory(path: &str) -> String {
    if let Some(root) = rule_directory(path, ".claude").or_else(|| rule_directory(path, ".cursor"))
    {
        return root.to_string();
    }
    for suffix in [
        ".claude/CLAUDE.md",
        ".claude/CLAUDE.local.md",
        ".claude/claude.md",
        ".claude/claude.local.md",
        ".cursor/rules",
        ".github/copilot-instructions.md",
    ] {
        if path == suffix {
            return String::new();
        }
        if let Some(root) = path.strip_suffix(&format!("/{suffix}")) {
            return root.to_string();
        }
    }
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default()
}

/// Shared recognition for discovery and host invalidation. Match complete
/// directory components so `not.claude/rules` is not mistaken for policy.
pub(crate) fn is_instruction_path(path: &str) -> bool {
    let path = path.replace('\\', "/");
    let basename = path.rsplit('/').next().unwrap_or("");
    if matches!(
        basename,
        "CLAUDE.md"
            | "CLAUDE.local.md"
            | "AGENTS.md"
            | "Agents.md"
            | "AGENT.md"
            | "agents.md"
            | ".cursorrules"
            | ".windsurfrules"
            | "copilot-instructions.md"
    ) {
        return true;
    }
    if rule_directory(&path, ".claude").is_some() && basename.ends_with(".md") {
        return true;
    }
    if rule_directory(&path, ".cursor").is_some()
        && (basename.ends_with(".md") || basename.ends_with(".mdc"))
    {
        return true;
    }
    let rooted = format!("/{path}");
    if rooted.ends_with("/.cursor/rules") {
        return true;
    }
    rooted.contains("/.claude/projects/")
        && rooted.contains("/memory/")
        && basename.ends_with(".md")
}

fn is_absolute(path: &str) -> bool {
    path.starts_with('/')
        || (cfg!(windows)
            && path.as_bytes().get(1) == Some(&b':')
            && path
                .as_bytes()
                .get(2)
                .is_some_and(|byte| matches!(byte, b'/' | b'\\')))
}

fn resolve_path(path: &str, base: &str) -> String {
    let path = native_separators(path);
    if is_absolute(&path) {
        normalize_path(&path)
    } else {
        normalize_path(&format!("{base}/{path}"))
    }
}

fn normalize_path(path: &str) -> String {
    let path = native_separators(path);
    #[cfg(windows)]
    let path = path.strip_prefix("//?/").unwrap_or(&path);
    let rooted = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts
                    .last()
                    .is_some_and(|part| *part != ".." && !part.ends_with(':'))
                {
                    parts.pop();
                } else if !rooted && !parts.first().is_some_and(|part| part.ends_with(':')) {
                    parts.push(part);
                }
            }
            _ => parts.push(part),
        }
    }
    let result = format!("{}{}", if rooted { "/" } else { "" }, parts.join("/"));
    if cfg!(windows) {
        result.to_lowercase()
    } else {
        result
    }
}

fn native_separators(path: &str) -> String {
    if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.to_string()
    }
}

fn within_directory<'a>(target: &'a str, base: &str) -> Option<&'a str> {
    if target == base {
        return Some("");
    }
    let prefix = format!("{}/", base.trim_end_matches('/'));
    target.strip_prefix(&prefix)
}

#[derive(Default)]
struct ScopeMetadata {
    paths: Option<Vec<String>>,
    globs: Option<Vec<String>>,
    always_apply: Option<bool>,
    has_description: bool,
}

impl ScopeMetadata {
    fn parse(content: &str) -> Result<Self, String> {
        let lines: Vec<_> = content.trim_start_matches('\u{feff}').lines().collect();
        if lines.first().map(|line| line.trim()) != Some("---") {
            return Ok(Self::default());
        }
        let end = lines
            .iter()
            .skip(1)
            .position(|line| line.trim() == "---")
            .map(|index| index + 1)
            .ok_or("Unclosed frontmatter")?;
        let mut out = Self::default();
        let mut index = 1;
        while index < end {
            let line = lines[index];
            index += 1;
            let Some((key, value)) = line.split_once(':') else {
                if line
                    .split_whitespace()
                    .next()
                    .is_some_and(|word| matches!(word, "paths" | "globs" | "alwaysApply"))
                {
                    return Err("Scope fields require key: value syntax".into());
                }
                continue;
            };
            let key = key.trim();
            if key == "<<" {
                return Err(
                    "YAML merge keys are not supported in instruction scope metadata".into(),
                );
            }
            if key == "description" {
                out.has_description = true;
                continue;
            }
            let key = key.trim_matches(['\'', '"']);
            if !matches!(key, "paths" | "globs" | "alwaysApply") {
                continue;
            }
            if line.starts_with(char::is_whitespace) {
                return Err(format!("{key} must be a top-level frontmatter field"));
            }
            let value = strip_comment(value.trim())?.trim();
            if key == "alwaysApply" {
                if out.always_apply.is_some() {
                    return Err("Duplicate alwaysApply".into());
                }
                out.always_apply = Some(match value {
                    "true" => true,
                    "false" => false,
                    _ => return Err("alwaysApply must be true or false".into()),
                });
                continue;
            }
            let slot = if key == "paths" {
                &mut out.paths
            } else {
                &mut out.globs
            };
            if slot.is_some() {
                return Err(format!("Duplicate {key}"));
            }
            let patterns = if value.is_empty() {
                let mut patterns = Vec::new();
                while index < end {
                    let entry = lines[index];
                    let trimmed = entry.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        index += 1;
                        continue;
                    }
                    if let Some(item) = trimmed.strip_prefix("- ") {
                        patterns.push(unquote(strip_comment(item)?.trim())?);
                        index += 1;
                    } else if entry.starts_with(char::is_whitespace) {
                        return Err(format!("{key} must be a list of glob strings"));
                    } else {
                        break;
                    }
                }
                patterns
            } else if value.starts_with('[') {
                let body = value
                    .strip_suffix(']')
                    .ok_or("Unclosed inline scope list")?;
                split_patterns(&body[1..], true)?
            } else {
                split_patterns(&unquote(value)?, false)?
            };
            *slot = Some(patterns);
        }
        Ok(out)
    }
}

fn strip_comment(value: &str) -> Result<&str, String> {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote == Some('"') {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character == '#' && (index == 0 || value[..index].ends_with(char::is_whitespace))
        {
            return Ok(value[..index].trim_end());
        }
    }
    if quote.is_some() {
        return Err("Unclosed quoted scope value".into());
    }
    Ok(value)
}

fn unquote(value: &str) -> Result<String, String> {
    if value.starts_with('"') {
        return serde_json::from_str::<String>(value)
            .map_err(|_| "Invalid double-quoted scope string".into());
    }
    if let Some(inner) = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        if inner.replace("''", "").contains('\'') {
            return Err("Invalid single-quoted scope string".into());
        }
        return Ok(inner.replace("''", "'"));
    }
    if value.is_empty()
        || matches!(value, "null" | "~" | "true" | "false" | "|" | ">")
        || value.parse::<f64>().is_ok()
        || value.starts_with(['[', '&'])
        || value.contains(": ")
    {
        return Err("Scope entries must be glob strings".into());
    }
    Ok(value.to_string())
}

fn split_patterns(value: &str, list: bool) -> Result<Vec<String>, String> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut depth: usize = 0;
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            }
        } else {
            match character {
                '\'' | '"' if list => quote = Some(character),
                '{' => depth += 1,
                '}' => depth = depth.checked_sub(1).ok_or("Unbalanced brace glob")?,
                ',' if depth == 0 => {
                    out.push(unquote(value[start..index].trim())?);
                    start = index + 1;
                }
                _ => {}
            }
        }
    }
    if quote.is_some() || depth != 0 {
        return Err("Unclosed scope string or brace glob".into());
    }
    out.push(unquote(value[start..].trim())?);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn config() -> Config {
        Config {
            project_root: PathBuf::from("/workspace/project"),
            home_dir: PathBuf::from("/home/test"),
            arai_base_dir: PathBuf::from("/state"),
            extra_sources: Vec::new(),
            guardrails_mode: "advise".into(),
            llm_command: None,
            api_url: None,
            api_key_env: None,
            api_model: None,
        }
    }

    fn write(path: &str) -> Value {
        json!({"hook_event_name":"PreToolUse","tool_name":"Write", "tool_input":{"file_path":path}})
    }

    #[test]
    fn nested_instructions_apply_to_descendants_and_explicit_shell_cwd() {
        let scope = SourceScope::from_source(
            "/workspace/project/packages/api/AGENTS.md",
            "agents_md",
            "- Never execute cargo",
        )
        .unwrap();
        let cfg = config();
        assert!(scope.matches("", &cfg, &write("packages/api/new.rs")));
        assert!(scope.matches("", &cfg, &write("packages/api/deep/new.rs")));
        assert!(!scope.matches("", &cfg, &write("packages/api2/new.rs")));
        assert!(!scope.matches("", &cfg, &write("packages/api/../../unrelated.rs")));
        assert!(!scope.matches("", &cfg, &json!({"tool_name":"Bash", "cwd":"/workspace/project", "tool_input":{"command":"cargo clean"}})));
        assert!(scope.matches("", &cfg, &json!({"tool_name":"Bash", "cwd":"/workspace/project", "tool_input":{"command":"cargo clean", "workdir":"packages/api"}})));
        assert!(scope.matches(
            "",
            &cfg,
            &json!({"hook_event_name":"UserPromptSubmit", "cwd":"/workspace/project/packages/api"})
        ));
    }

    #[test]
    fn claude_rule_directory_does_not_become_its_own_scope() {
        for path in [".claude/CLAUDE.md", ".claude/rules/backend/rules.md"] {
            let scope = SourceScope::from_source(path, "claude_md_project", "rule").unwrap();
            assert!(scope.matches("", &config(), &write("src/main.rs")));
        }
        let scope = SourceScope::from_source(
            "packages/api/.claude/rules/backend/rules.md",
            "claude_rules",
            "rule",
        )
        .unwrap();
        assert!(scope.matches("", &config(), &write("packages/api/new.py")));
        assert!(!scope.matches("", &config(), &write("new.py")));
    }

    #[test]
    fn claude_paths_support_yaml_lists_scalars_braces_and_root_only_globs() {
        for metadata in [
            "paths:\n  - \"src/**/*.{rs,toml}\"\n  - '*.md'",
            "paths: ['src/**/*.{rs,toml}', '*.md']",
            "paths: \"src/**/*.{rs,toml}, *.md\"",
        ] {
            let scope = SourceScope::from_source(
                ".claude/rules/dev.md",
                "claude_rules",
                &format!("---\n{metadata}\n---\nrule"),
            )
            .unwrap();
            assert!(
                scope.matches("", &config(), &write("src/main.rs")),
                "{metadata}"
            );
            assert!(
                scope.matches("", &config(), &write("src/deep/config.toml")),
                "{metadata}"
            );
            assert!(
                scope.matches("", &config(), &write("README.md")),
                "{metadata}"
            );
            assert!(
                !scope.matches("", &config(), &write("docs/README.md")),
                "{metadata}"
            );
            assert!(
                !scope.matches("", &config(), &write("src/main.py")),
                "{metadata}"
            );
            assert!(!scope.matches("", &config(), &json!({"tool_name":"Bash", "cwd":"/workspace/project/src", "tool_input":{"command":"cargo clean"}})), "{metadata}");
        }
    }

    #[test]
    fn cursor_activation_modes_do_not_become_global() {
        let cfg = config();
        let path = ".cursor/rules/api.mdc";
        for metadata in [
            "",
            "alwaysApply: false",
            "description: backend conventions\nalwaysApply: false",
            "globs: []",
            "globs:",
        ] {
            let scope = SourceScope::from_source(
                path,
                "cursor_rules",
                &format!("---\n{metadata}\n---\nrule"),
            )
            .unwrap();
            assert!(scope.inactive_reason().is_some(), "{metadata}");
            assert!(
                !scope.matches(path, &cfg, &write("api/file.py")),
                "{metadata}"
            );
        }
        let global = SourceScope::from_source(
            path,
            "cursor_rules",
            "---\nalwaysApply: true\nglobs: api/**\n---\nrule",
        )
        .unwrap();
        assert!(global.matches(path, &cfg, &write("unrelated.rs")));
        let scoped = SourceScope::from_source(
            path,
            "cursor_rules",
            "---\nalwaysApply: false\nglobs: api/**/*.py, tests/**/*.py\n---\nrule",
        )
        .unwrap();
        assert!(scoped.matches(path, &cfg, &write("api/file.py")));
        assert!(scoped.matches(path, &cfg, &write("tests/check.py")));
        assert!(!scoped.matches(path, &cfg, &write("web/file.py")));
        let legacy = SourceScope::from_source(
            ".cursor/rules/legacy.md",
            "cursor_rules",
            "- Never execute git",
        )
        .unwrap();
        assert!(legacy.matches("", &cfg, &write("anywhere.rs")));
        let explicit = SourceScope::from_source(
            ".cursor/rules/legacy.md",
            "cursor_rules",
            "---\nalwaysApply: false\n---\nrule",
        )
        .unwrap();
        assert!(!explicit.matches("", &cfg, &write("anywhere.rs")));
    }

    #[test]
    fn malformed_scope_is_an_error_not_a_global_rule() {
        for metadata in [
            "paths: [broken",
            "paths: ['unterminated]",
            "paths: [../outside/**]",
            "paths: '[broken'",
            "paths:  [src/**]\npaths: [tests/**]",
            "alwaysApply: maybe",
            "globs: src/**\nalwaysApply: false\nalwaysApply: true",
            "paths:\n  nested: not a list",
            " paths: src/**",
            "paths: [/absolute/**]",
            "globs: {unclosed/**",
            "paths: [!src/**]",
        ] {
            assert!(
                SourceScope::from_source(
                    ".claude/rules/test.md",
                    "claude_rules",
                    &format!("---\n{metadata}\n---\nrule")
                )
                .is_err(),
                "{metadata}"
            );
        }
        assert!(SourceScope::from_source(
            ".claude/rules/test.md",
            "claude_rules",
            "---\npaths: src/**\nrule"
        )
        .is_err());
        let empty = SourceScope::from_source(
            ".claude/rules/test.md",
            "claude_rules",
            "---\npaths: []\n---\nrule",
        )
        .unwrap();
        assert!(!empty.matches("", &config(), &write("src/main.rs")));
    }

    #[test]
    fn global_patterns_and_external_sources_preserve_embedding_contract() {
        let global = SourceScope::from_source(
            "/home/test/.claude/rules/python.md",
            "claude_rules_global",
            "---\npaths: '**/*.py'\n---\nrule",
        )
        .unwrap();
        assert!(global.matches("", &config(), &write("src/main.py")));
        assert!(!global.matches("", &config(), &write("src/main.rs")));
        for (path, tag) in [
            ("kete://tenant/rule", "claude_rules"),
            ("manual://x", "manual"),
            ("/external/policy.md", "organization"),
        ] {
            let scope =
                SourceScope::from_source(path, tag, "---\npaths: invalid[\n---\nrule").unwrap();
            assert!(scope.matches(path, &config(), &write("any/file.rs")));
        }
    }

    #[test]
    fn notebook_and_camel_case_host_fields_are_scoped() {
        let scope = SourceScope::from_source(
            ".claude/rules/notebook.md",
            "claude_rules",
            "---\npaths: 'notebooks/**/*.ipynb'\n---\nrule",
        )
        .unwrap();
        assert!(scope.matches(
            "",
            &config(),
            &json!({"toolName":"NotebookEdit", "toolInput":{"notebook_path":"notebooks/new.ipynb"}})
        ));
        assert!(!scope.matches(
            "",
            &config(),
            &json!({"toolName":"NotebookEdit", "toolInput":{"notebook_path":"outside/new.ipynb"}})
        ));
    }

    #[test]
    fn invalid_stored_scope_metadata_is_detected() {
        let bad: SourceScope = serde_json::from_value(
            json!({"directory":null,"patterns":["[invalid"],"inactive":null}),
        )
        .unwrap();
        assert!(bad.validate().is_err());
        assert!(serde_json::from_value::<SourceScope>(
            json!({"directory":null,"patterns":[],"inactive":null,"unknown":true})
        )
        .is_err());
    }

    #[test]
    fn instruction_recognition_accepts_relative_mdc_paths_only_in_rule_dirs() {
        for path in [
            ".cursor/rules/a.mdc",
            "C:\\repo\\.cursor\\rules\\a.mdc",
            ".claude/rules/a.md",
            "package/AGENTS.md",
            "CLAUDE.local.md",
            ".cursor/rules",
        ] {
            assert!(is_instruction_path(path), "{path}");
        }
        for path in [
            "some.mdc",
            "not.claude/rules/a.md",
            ".claude/rules/a.mdc",
            "src/readme.md",
            ".cursor/rules2/a.mdc",
        ] {
            assert!(!is_instruction_path(path), "{path}");
        }
    }
}

#[cfg(test)]
mod action_scope_regressions {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;
    fn config() -> Config {
        Config {
            project_root: PathBuf::from("/workspace/project"),
            home_dir: PathBuf::from("/home/test"),
            arai_base_dir: PathBuf::from("/state"),
            extra_sources: Vec::new(),
            guardrails_mode: "advise".into(),
            llm_command: None,
            api_url: None,
            api_key_env: None,
            api_model: None,
        }
    }
    #[test]
    fn irrelevant_tool_fields_cannot_redirect_scope() {
        let file_scope = SourceScope::from_source(
            ".claude/rules/restricted.md",
            "claude_rules",
            "---\npaths: protected/**\n---\nrule",
        )
        .unwrap();
        let directory_scope =
            SourceScope::from_source("protected/AGENTS.md", "agents_md", "rule").unwrap();
        let cfg = config();
        assert!(file_scope.matches("", &cfg, &json!({"tool_name":"NotebookEdit", "tool_input":{"notebook_path":"protected/alembic.ipynb","file_path":"unrelated/alembic.ipynb"}})));
        assert!(file_scope.matches("", &cfg, &json!({"tool_name":"Edit", "tool_input":{"file_path":"protected/alembic.py","workdir":"unrelated","cwd":"elsewhere"}})));
        assert!(directory_scope.matches("", &cfg, &json!({"tool_name":"Bash", "cwd":"/workspace/project/protected", "tool_input":{"command":"cargo clean","file_path":"/unrelated/file"}})));
        assert!(directory_scope.matches("", &cfg, &json!({"tool_name":"Bash", "cwd":"/workspace/project", "tool_input":{"command":"cargo clean","workdir":"protected"}})));
        let deeper =
            SourceScope::from_source("protected/nested/AGENTS.md", "agents_md", "rule").unwrap();
        assert!(deeper.matches("", &cfg, &json!({"tool_name":"Bash", "cwd":"/workspace/project/protected", "tool_input":{"command":"cargo clean","workdir":"nested"}})));
    }
    #[cfg(unix)]
    #[test]
    fn unix_literal_backslash_is_not_a_directory_separator() {
        let cfg = config();
        let scope = SourceScope::from_source(
            ".claude/rules/restricted.md",
            "claude_rules",
            "---\npaths: protected/**\n---\nrule",
        )
        .unwrap();
        assert!(!scope.matches(
            "",
            &cfg,
            &json!({"tool_name":"Edit", "tool_input":{"file_path":"protected\\alembic.py"}})
        ));
        let directory =
            SourceScope::from_source("protected\\literal/AGENTS.md", "agents_md", "rule").unwrap();
        assert!(directory.matches(
            "",
            &cfg,
            &json!({"tool_name":"Edit", "tool_input":{"file_path":"protected\\literal/file.py"}})
        ));
        assert!(!directory.matches(
            "",
            &cfg,
            &json!({"tool_name":"Edit", "tool_input":{"file_path":"protected/literal/file.py"}})
        ));
    }
}

#[cfg(test)]
mod resolved_scope_regressions {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;
    #[test]
    fn imported_frontmatter_does_not_replace_local_activation() {
        let cfg = Config {
            project_root: PathBuf::from("/workspace/project"),
            home_dir: PathBuf::from("/home/test"),
            arai_base_dir: PathBuf::from("/state"),
            extra_sources: Vec::new(),
            guardrails_mode: "advise".into(),
            llm_command: None,
            api_url: None,
            api_key_env: None,
            api_model: None,
        };
        let upstream = "<!-- arai:extends-block url=\"https://example.invalid/rules\" tier=\"strict\" -->\n---\nalwaysApply: true\npaths: '**/*'\n---\n- Never execute cargo\n\n<!-- end arai:extends -->\n\n";
        for (path, tag, local) in [
            (
                ".claude/rules/local.md",
                "claude_rules",
                "---\npaths: protected/**\n---\nlocal",
            ),
            (
                ".cursor/rules/local.mdc",
                "cursor_rules",
                "---\nalwaysApply: false\nglobs: protected/**\n---\nlocal",
            ),
        ] {
            let scope = SourceScope::from_source(path, tag, &format!("{upstream}{local}")).unwrap();
            assert!(scope.matches(
                path,
                &cfg,
                &json!({"tool_name":"Write","tool_input":{"file_path":"protected/file.rs"}})
            ));
            assert!(!scope.matches(
                path,
                &cfg,
                &json!({"tool_name":"Write","tool_input":{"file_path":"other/file.rs"}})
            ));
        }
        let manual = SourceScope::from_source(
            ".cursor/rules/local.mdc",
            "cursor_rules",
            &format!("{upstream}---\nalwaysApply: false\n---\nlocal"),
        )
        .unwrap();
        assert!(manual.inactive_reason().is_some());
    }
    #[test]
    fn malformed_resolver_framing_cannot_hide_local_scope() {
        for content in [
            "<!-- arai:extends-block malformed -->\n---\npaths: protected/**\n---\nrule",
            "<!-- arai:extends-block url=\"x\" tier=\"peer\" -->\n---\npaths: protected/**\n---\nrule",
            "<!-- arai:extends-block url=\"x\" tier=\"peer\" -->\n<!-- arai:extends-block url=\"y\" tier=\"peer\" -->\n<!-- end arai:extends -->\nrule",
            "<!-- arai:extends-block url=\"x\" tier=\"peer\" -->\n<!-- end arai:extends -->\nunexpected tail\n<!-- end arai:extends -->\n---\npaths: protected/**\n---\nrule",
        ] {
            assert!(SourceScope::from_source(".claude/rules/local.md", "claude_rules", content).is_err(), "{content}");
        }
    }
}
