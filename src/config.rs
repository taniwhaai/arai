use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub project_root: PathBuf,
    pub home_dir: PathBuf,
    pub arai_base_dir: PathBuf,
    pub extra_sources: Vec<String>,
    pub guardrails_mode: String,
    pub llm_command: Option<String>,
}

impl Config {
    pub fn load() -> Result<Config, String> {
        let project_root = find_project_root()?;
        let home_dir = dirs::home_dir().ok_or("Could not determine home directory")?;

        let arai_base_dir = std::env::var("ARAI_DB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home_dir.join(".arai"));

        let llm_command = std::env::var("ARAI_LLM_CMD").ok();

        let mut cfg = Config {
            project_root,
            home_dir,
            arai_base_dir,
            extra_sources: Vec::new(),
            guardrails_mode: "advise".to_string(),
            llm_command,
        };

        // Load optional config file
        let config_path = cfg.arai_base_dir.join("config.toml");
        if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                cfg.load_toml(&content);
            }
        }

        Ok(cfg)
    }

    fn load_toml(&mut self, content: &str) {
        if let Ok(table) = content.parse::<toml::Table>() {
            if let Some(sources) = table.get("sources").and_then(|v| v.as_table()) {
                if let Some(extra) = sources.get("extra").and_then(|v| v.as_array()) {
                    self.extra_sources = extra
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                }
            }
            if let Some(guardrails) = table.get("guardrails").and_then(|v| v.as_table()) {
                if let Some(mode) = guardrails.get("mode").and_then(|v| v.as_str()) {
                    self.guardrails_mode = mode.to_string();
                }
            }
            if let Some(enrich) = table.get("enrich").and_then(|v| v.as_table()) {
                if let Some(cmd) = enrich.get("llm_command").and_then(|v| v.as_str()) {
                    // Config file is lower priority than env var
                    if self.llm_command.is_none() {
                        self.llm_command = Some(cmd.to_string());
                    }
                }
            }
        }
    }

    /// DB path: ~/.arai/projects/<dirname>-<8char-sha256>/arai.db
    pub fn db_path(&self) -> PathBuf {
        let canonical = self
            .project_root
            .to_string_lossy()
            .to_string();

        let dir_name = self
            .project_root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        let hash = hasher.finalize();
        let short_hash = hex_encode(&hash[..4]);

        let slug = format!("{dir_name}-{short_hash}");
        self.arai_base_dir.join("projects").join(slug).join("arai.db")
    }

    /// Claude Code memory slug: /home/matt/r/arai → -home-matt-r-arai
    pub fn claude_memory_slug(&self) -> String {
        let path_str = self.project_root.to_string_lossy();
        path_str.replace('/', "-")
    }

    /// Path to Claude Code memory files for this project
    pub fn claude_memory_dir(&self) -> PathBuf {
        self.home_dir
            .join(".claude")
            .join("projects")
            .join(self.claude_memory_slug())
            .join("memory")
    }

    /// Path to the project's .claude/settings.json
    pub fn claude_settings_path(&self) -> PathBuf {
        self.project_root.join(".claude").join("settings.json")
    }
}

fn find_project_root() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Could not get cwd: {e}"))?;
    let mut dir = cwd.as_path();
    loop {
        if dir.join(".git").exists() {
            return Ok(dir.to_path_buf());
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => {
                // No .git found, use cwd
                return Ok(cwd);
            }
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_memory_slug() {
        let cfg = Config {
            project_root: PathBuf::from("/usr/src/myproject"),
            home_dir: PathBuf::from("/usr/src"),
            arai_base_dir: PathBuf::from("/usr/src/.arai"),
            extra_sources: Vec::new(),
            guardrails_mode: "advise".to_string(),
            llm_command: None,
        };
        assert_eq!(cfg.claude_memory_slug(), "-usr-src-myproject");
    }

    #[test]
    fn test_db_path_format() {
        let cfg = Config {
            project_root: PathBuf::from("/usr/src/myproject"),
            home_dir: PathBuf::from("/usr/src"),
            arai_base_dir: PathBuf::from("/usr/src/.arai"),
            extra_sources: Vec::new(),
            guardrails_mode: "advise".to_string(),
            llm_command: None,
        };
        let db = cfg.db_path();
        let db_str = db.to_string_lossy();
        assert!(db_str.starts_with("/usr/src/.arai/projects/myproject-"));
        assert!(db_str.ends_with("/arai.db"));
    }
}
