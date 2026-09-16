//! Discovery must produce a complete, scoped and unique source snapshot.
use arai::{config::Config, discovery};
use std::{fs, path::PathBuf};

struct Fixture {
    root: PathBuf,
    cfg: Config,
}
impl Fixture {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "arai_discovery_scope_{}_{}",
            std::process::id(),
            nanos
        ));
        let project_root = root.join("project");
        let home_dir = root.join("home");
        fs::create_dir_all(project_root.join(".git")).unwrap();
        fs::create_dir_all(&home_dir).unwrap();
        Self {
            cfg: Config {
                project_root,
                home_dir,
                arai_base_dir: root.join("state"),
                extra_sources: Vec::new(),
                guardrails_mode: "advise".into(),
                llm_command: None,
                api_url: None,
                api_key_env: None,
                api_model: None,
            },
            root,
        }
    }
    fn write(&self, path: &str, content: &str) {
        let path = self.cfg.project_root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn discovers_scoped_nested_and_ignored_host_files() {
    let fixture = Fixture::new();
    fixture.write(".gitignore", ".claude\npackage/AGENTS.md\n");
    for path in [
        ".claude/rules/security.md",
        ".claude/rules/backend/api.md",
        ".claude/CLAUDE.md",
        "package/AGENTS.md",
        "package/.claude/rules/api.md",
        "package/CLAUDE.local.md",
        ".cursor/rules/api.mdc",
    ] {
        fixture.write(path, "---\nalwaysApply: true\n---\n- Never execute cargo");
    }
    let files = discovery::discover(&fixture.cfg).unwrap();
    assert_eq!(
        files.len(),
        7,
        "{:?}",
        files.iter().map(|file| &file.path).collect::<Vec<_>>()
    );
    assert!(files.iter().any(|file| file.source_type == "claude_rules"));
    assert!(files
        .iter()
        .any(|file| file.path.replace('\\', "/").ends_with("package/AGENTS.md")));
}

#[test]
fn skips_generated_directories_and_nested_repositories() {
    let fixture = Fixture::new();
    for path in [
        "node_modules/pkg/AGENTS.md",
        "target/AGENTS.md",
        "dist/CLAUDE.md",
        ".git/CLAUDE.md",
        "nested-repo/AGENTS.md",
    ] {
        fixture.write(path, "- Never execute cargo");
    }
    fs::create_dir_all(fixture.cfg.project_root.join("nested-repo/.git")).unwrap();
    fixture.write("src/AGENTS.md", "- Never execute cargo");
    let files = discovery::discover(&fixture.cfg).unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].path.replace('\\', "/").ends_with("src/AGENTS.md"));
}

#[test]
fn global_claude_rules_are_discovered() {
    let fixture = Fixture::new();
    let path = fixture.cfg.home_dir.join(".claude/rules/nested/python.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "---\npaths:\n - '**/*.py'\n---\n- Never execute pip").unwrap();
    let files = discovery::discover(&fixture.cfg).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].source_type, "claude_rules_global");
}

#[test]
fn invalid_scope_or_unreadable_content_prevents_a_partial_snapshot() {
    let fixture = Fixture::new();
    fixture.write("AGENTS.md", "- Never execute cargo");
    fixture.write(
        ".claude/rules/broken.md",
        "---\npaths: [unterminated\n---\n- Never execute git",
    );
    assert!(discovery::discover(&fixture.cfg)
        .unwrap_err()
        .contains("Invalid instruction scope"));
    fs::write(
        fixture.cfg.project_root.join(".claude/rules/broken.md"),
        [0xff, 0xfe],
    )
    .unwrap();
    assert!(discovery::discover(&fixture.cfg)
        .unwrap_err()
        .contains("Could not read instruction file"));
}

#[test]
fn root_case_aliases_and_extra_references_are_deduplicated() {
    let mut fixture = Fixture::new();
    fixture.write("AGENTS.md", "- Never execute cargo");
    fixture.cfg.extra_sources.push("AGENTS.md".into());
    let files = discovery::discover(&fixture.cfg).unwrap();
    assert_eq!(
        files.len(),
        1,
        "{:?}",
        files.iter().map(|file| &file.path).collect::<Vec<_>>()
    );
    assert_eq!(files[0].source_type, "agents_md");
}
