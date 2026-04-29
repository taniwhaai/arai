#[cfg(feature = "code-graph")]
use tree_sitter::{Language, Node, Parser};

use std::path::Path;

/// Represents an import discovered in a source file.
#[derive(Debug, Clone)]
pub struct ImportInfo {
    pub tool_name: String,
    pub source_file: String,
    pub directory: String,
}

/// Scan the entire project for imports, respecting .gitignore.
#[cfg(feature = "code-graph")]
pub fn scan_project(project_root: &Path) -> Vec<ImportInfo> {
    let mut all_imports = Vec::new();

    // Use the `ignore` crate which respects .gitignore, .git/info/exclude, etc.
    let walker = ignore::WalkBuilder::new(project_root)
        .hidden(true)      // skip hidden files/dirs
        .git_ignore(true)   // respect .gitignore
        .git_global(true)   // respect global gitignore
        .git_exclude(true)  // respect .git/info/exclude
        .build();

    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }

        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        if !is_supported_extension(ext) {
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(path) {
            let path_str = path.to_string_lossy().to_string();
            let dir = path
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();

            if let Some(imports) = extract_imports(ext, &content) {
                for tool_name in imports {
                    all_imports.push(ImportInfo {
                        tool_name,
                        source_file: path_str.clone(),
                        directory: dir.clone(),
                    });
                }
            }
        }
    }

    all_imports
}

#[cfg(not(feature = "code-graph"))]
pub fn scan_project(_project_root: &Path) -> Vec<ImportInfo> {
    Vec::new()
}

fn is_supported_extension(ext: &str) -> bool {
    matches!(
        ext,
        "py" | "js" | "jsx" | "ts" | "tsx" | "rs" | "go" | "rb" | "java"
    )
}

/// Extract import tool names from a source file using tree-sitter.
#[cfg(feature = "code-graph")]
fn extract_imports(ext: &str, content: &str) -> Option<Vec<String>> {
    let language: Language = match ext {
        #[cfg(feature = "lang-python")]
        "py" => tree_sitter_python::LANGUAGE.into(),
        #[cfg(feature = "lang-javascript")]
        "js" | "jsx" => tree_sitter_javascript::LANGUAGE.into(),
        #[cfg(feature = "lang-typescript")]
        "ts" | "tsx" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        #[cfg(feature = "lang-rust")]
        "rs" => tree_sitter_rust::LANGUAGE.into(),
        #[cfg(feature = "lang-go")]
        "go" => tree_sitter_go::LANGUAGE.into(),
        #[cfg(feature = "lang-ruby")]
        "rb" => tree_sitter_ruby::LANGUAGE.into(),
        #[cfg(feature = "lang-java")]
        "java" => tree_sitter_java::LANGUAGE.into(),
        _ => return None,
    };

    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(content, None)?;

    let mut tools = Vec::new();
    collect_imports(tree.root_node(), content.as_bytes(), ext, &mut tools);
    Some(tools)
}

#[cfg(not(feature = "code-graph"))]
fn extract_imports(_ext: &str, _content: &str) -> Option<Vec<String>> {
    None
}

/// Walk the AST and collect import nodes based on language-specific node kinds.
#[cfg(feature = "code-graph")]
fn collect_imports(node: Node, source: &[u8], ext: &str, tools: &mut Vec<String>) {
    collect_imports_bounded(node, source, ext, tools, 0);
}

/// Maximum AST recursion depth.  Tree-sitter has historically panicked on
/// adversarial inputs with extreme nesting; we bound the walk so a malicious
/// or generated source file can't blow the stack.  500 covers any realistic
/// program — the deepest legitimate Python/TS files are <100 levels deep.
#[cfg(feature = "code-graph")]
const MAX_AST_DEPTH: u32 = 500;

#[cfg(feature = "code-graph")]
fn collect_imports_bounded(
    node: Node,
    source: &[u8],
    ext: &str,
    tools: &mut Vec<String>,
    depth: u32,
) {
    if depth > MAX_AST_DEPTH {
        return;
    }
    let kind = node.kind();

    let import_text = match ext {
        "py" => extract_python_import(node, source, kind),
        "js" | "jsx" | "ts" | "tsx" => extract_js_import(node, source, kind),
        "rs" => extract_rust_import(node, source, kind),
        "go" => extract_go_import(node, source, kind),
        "rb" => extract_ruby_import(node, source, kind),
        "java" => extract_java_import(node, source, kind),
        _ => None,
    };

    if let Some(text) = import_text {
        if let Some(tool) = normalize_import(&text, ext) {
            if !tools.contains(&tool) {
                tools.push(tool);
            }
        }
        // Don't recurse into import nodes — we already extracted
        return;
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_imports_bounded(child, source, ext, tools, depth + 1);
    }
}

#[cfg(feature = "code-graph")]
fn node_text(node: Node, source: &[u8]) -> Option<String> {
    node.utf8_text(source).ok().map(|s| s.to_string())
}

#[cfg(feature = "code-graph")]
fn child_by_field(node: Node, field: &str, source: &[u8]) -> Option<String> {
    node.child_by_field_name(field)
        .and_then(|n| node_text(n, source))
}

// --- Language-specific import extractors ---

#[cfg(feature = "lang-python")]
fn extract_python_import(node: Node, source: &[u8], kind: &str) -> Option<String> {
    match kind {
        "import_statement" => {
            // import alembic / import alembic.op
            node.child_by_field_name("name")
                .and_then(|n| node_text(n, source))
        }
        "import_from_statement" => {
            // from alembic import op
            node.child_by_field_name("module_name")
                .and_then(|n| node_text(n, source))
        }
        _ => None,
    }
}

#[cfg(not(feature = "lang-python"))]
fn extract_python_import(_: Node, _: &[u8], _: &str) -> Option<String> { None }

#[cfg(feature = "lang-javascript")]
fn extract_js_import(node: Node, source: &[u8], kind: &str) -> Option<String> {
    match kind {
        "import_statement" => child_by_field(node, "source", source),
        "call_expression" => {
            // require("express")
            let func = node.child_by_field_name("function")?;
            let func_text = node_text(func, source)?;
            if func_text == "require" {
                let args = node.child_by_field_name("arguments")?;
                let mut cursor = args.walk();
                for child in args.children(&mut cursor) {
                    if child.kind() == "string" {
                        return node_text(child, source);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(not(feature = "lang-javascript"))]
fn extract_js_import(_: Node, _: &[u8], _: &str) -> Option<String> { None }

#[cfg(feature = "lang-rust")]
fn extract_rust_import(node: Node, source: &[u8], kind: &str) -> Option<String> {
    match kind {
        "use_declaration" => {
            // use serde::Serialize → extract "serde"
            let arg = node.child_by_field_name("argument")?;
            extract_rust_use_path(arg, source)
        }
        "extern_crate_declaration" => {
            child_by_field(node, "name", source)
        }
        _ => None,
    }
}

#[cfg(feature = "lang-rust")]
fn extract_rust_use_path(node: Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "scoped_identifier" | "scoped_use_list" => {
            // Get the leftmost path segment
            node.child_by_field_name("path")
                .and_then(|n| {
                    if n.kind() == "identifier" || n.kind() == "crate" {
                        node_text(n, source)
                    } else {
                        extract_rust_use_path(n, source)
                    }
                })
        }
        "identifier" => node_text(node, source),
        _ => node_text(node, source),
    }
}

#[cfg(not(feature = "lang-rust"))]
fn extract_rust_import(_: Node, _: &[u8], _: &str) -> Option<String> { None }

#[cfg(feature = "lang-go")]
fn extract_go_import(node: Node, source: &[u8], kind: &str) -> Option<String> {
    if kind == "import_spec" {
        child_by_field(node, "path", source)
    } else {
        None
    }
}

#[cfg(not(feature = "lang-go"))]
fn extract_go_import(_: Node, _: &[u8], _: &str) -> Option<String> { None }

#[cfg(feature = "lang-ruby")]
fn extract_ruby_import(node: Node, source: &[u8], kind: &str) -> Option<String> {
    if kind == "call" {
        let method = node.child_by_field_name("method")?;
        let method_text = node_text(method, source)?;
        if method_text == "require" || method_text == "require_relative" || method_text == "gem" {
            let args = node.child_by_field_name("arguments")?;
            let mut cursor = args.walk();
            for child in args.children(&mut cursor) {
                if child.kind() == "string" {
                    // Get the string content (without quotes)
                    let mut inner_cursor = child.walk();
                    for inner in child.children(&mut inner_cursor) {
                        if inner.kind() == "string_content" {
                            return node_text(inner, source);
                        }
                    }
                    return node_text(child, source);
                }
            }
        }
    }
    None
}

#[cfg(not(feature = "lang-ruby"))]
fn extract_ruby_import(_: Node, _: &[u8], _: &str) -> Option<String> { None }

#[cfg(feature = "lang-java")]
fn extract_java_import(node: Node, source: &[u8], kind: &str) -> Option<String> {
    if kind == "import_declaration" {
        // Get the full scoped identifier
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "scoped_identifier" {
                return node_text(child, source);
            }
        }
    }
    None
}

#[cfg(not(feature = "lang-java"))]
fn extract_java_import(_: Node, _: &[u8], _: &str) -> Option<String> { None }

/// Normalize an import path to a top-level tool/library name.
fn normalize_import(import_text: &str, ext: &str) -> Option<String> {
    let text = import_text
        .trim()
        .trim_matches('"')
        .trim_matches('\'');

    if text.is_empty() {
        return None;
    }

    let top_level = match ext {
        "py" => {
            // "alembic.op" → "alembic"
            text.split('.').next().unwrap_or(text)
        }
        "js" | "jsx" | "ts" | "tsx" => {
            // "@scope/package" → "package", "express" → "express"
            // Skip relative imports
            if text.starts_with('.') {
                return None;
            }
            if text.starts_with('@') {
                // Scoped package: @scope/name → name
                text.split('/').nth(1).unwrap_or(text)
            } else {
                text.split('/').next().unwrap_or(text)
            }
        }
        "rs" => {
            // Already the crate name from query
            text
        }
        "go" => {
            // "github.com/user/pkg" → "pkg", "fmt" → "fmt"
            text.rsplit('/').next().unwrap_or(text)
        }
        "rb" => {
            // "rails" → "rails", "active_record/base" → "active_record"
            text.split('/').next().unwrap_or(text)
        }
        "java" => {
            // "org.apache.kafka" → "kafka", "java.util" → skip stdlib
            let parts: Vec<&str> = text.split('.').collect();
            if parts.first() == Some(&"java") || parts.first() == Some(&"javax") {
                return None; // Skip Java stdlib
            }
            // Take the last meaningful segment (usually the library name)
            // For "org.apache.kafka.clients" → "kafka"
            if parts.len() >= 3 {
                parts.get(2).copied().unwrap_or(text)
            } else {
                parts.last().copied().unwrap_or(text)
            }
        }
        _ => text,
    };

    let name = top_level.to_lowercase();

    // Skip very common stdlib modules that aren't "tools"
    if is_stdlib(&name, ext) {
        return None;
    }

    if name.is_empty() || name.len() < 2 {
        return None;
    }

    Some(name)
}

fn is_stdlib(name: &str, ext: &str) -> bool {
    match ext {
        "py" => matches!(
            name,
            "os" | "sys" | "re" | "io" | "json" | "math" | "time" | "datetime"
            | "collections" | "functools" | "itertools" | "typing" | "pathlib"
            | "subprocess" | "logging" | "unittest" | "hashlib" | "abc"
            | "dataclasses" | "enum" | "copy" | "string" | "textwrap"
            | "struct" | "csv" | "argparse" | "shutil" | "tempfile"
            | "glob" | "fnmatch" | "stat" | "contextlib" | "warnings"
        ),
        "go" => matches!(
            name,
            "fmt" | "os" | "io" | "log" | "net" | "http" | "strings" | "strconv"
            | "sync" | "time" | "errors" | "context" | "testing" | "bytes"
            | "encoding" | "path" | "filepath" | "regexp" | "sort" | "math"
        ),
        "rs" => matches!(name, "std" | "core" | "alloc"),
        "rb" => matches!(
            name,
            "json" | "yaml" | "csv" | "net" | "uri" | "open-uri" | "fileutils"
            | "set" | "date" | "time" | "logger" | "pathname" | "stringio"
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_python_import() {
        assert_eq!(normalize_import("alembic", "py"), Some("alembic".to_string()));
        assert_eq!(normalize_import("alembic.op", "py"), Some("alembic".to_string()));
        assert_eq!(normalize_import("os", "py"), None); // stdlib
        assert_eq!(normalize_import("sqlalchemy", "py"), Some("sqlalchemy".to_string()));
    }

    #[test]
    fn test_normalize_js_import() {
        assert_eq!(normalize_import("express", "js"), Some("express".to_string()));
        assert_eq!(normalize_import("./utils", "js"), None); // relative
        assert_eq!(normalize_import("@scope/package", "js"), Some("package".to_string()));
    }

    #[test]
    fn test_normalize_rust_import() {
        assert_eq!(normalize_import("serde", "rs"), Some("serde".to_string()));
        assert_eq!(normalize_import("std", "rs"), None); // stdlib
    }

    #[test]
    fn test_normalize_go_import() {
        assert_eq!(normalize_import("github.com/user/pkg", "go"), Some("pkg".to_string()));
        assert_eq!(normalize_import("fmt", "go"), None); // stdlib
    }

    #[test]
    fn test_normalize_java_import() {
        assert_eq!(normalize_import("org.apache.kafka.clients", "java"), Some("kafka".to_string()));
        assert_eq!(normalize_import("java.util.List", "java"), None); // stdlib
    }

    #[cfg(feature = "lang-python")]
    #[test]
    fn test_python_extraction() {
        let code = r#"
import alembic
from sqlalchemy import Column
from os import path
import json
"#;
        let imports = extract_imports("py", code).unwrap();
        assert!(imports.contains(&"alembic".to_string()));
        assert!(imports.contains(&"sqlalchemy".to_string()));
        assert!(!imports.contains(&"os".to_string()));
        assert!(!imports.contains(&"json".to_string()));
    }

    #[cfg(feature = "lang-rust")]
    #[test]
    fn test_rust_extraction() {
        let code = r#"
use serde::Serialize;
use std::path::Path;
use clap::Parser;
"#;
        let imports = extract_imports("rs", code).unwrap();
        assert!(imports.contains(&"serde".to_string()));
        assert!(imports.contains(&"clap".to_string()));
        assert!(!imports.contains(&"std".to_string()));
    }
}
