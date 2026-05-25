use sha2::{Digest, Sha256};
use std::io::IsTerminal;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub project_root: PathBuf,
    pub home_dir: PathBuf,
    pub arai_base_dir: PathBuf,
    pub extra_sources: Vec<String>,
    pub guardrails_mode: String,
    pub llm_command: Option<String>,
    pub api_url: Option<String>,
    pub api_key_env: Option<String>,
    pub api_model: Option<String>,
    /// The deprecation notice (if any) produced by [`resolve_base_dir`]
    /// during [`Config::load`].  Persisted here so the `arai init` entry
    /// point can pass it to the migration module without re-running the
    /// resolver.  The stderr-warning behaviour in `Config::load` is
    /// unchanged; this field is additive.
    pub deprecation_notice: Option<DeprecationNotice>,
}

/// A deprecation notice produced by [`resolve_base_dir`] when the chosen
/// path was selected via a deprecated mechanism.  Each variant carries a
/// human-readable message string the caller can emit verbatim to standard
/// error.  The two variants are mutually exclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeprecationNotice {
    /// Branch 2: the deprecated `ARAI_DB_DIR` environment variable
    /// supplied the path.  The message instructs the user to rename the
    /// environment variable to `ARAI_BASE_DIR`.
    DeprecatedEnvVar(String),
    /// Branch 4: the legacy default path `~/.arai` supplied the path
    /// because it exists and the new default `~/.taniwha/arai` does not.
    /// The message mentions the forthcoming `arai migrate` command.
    DeprecatedDefaultPath(String),
}

impl DeprecationNotice {
    /// Borrow the human-readable message string carried by this notice.
    pub fn message(&self) -> &str {
        match self {
            DeprecationNotice::DeprecatedEnvVar(msg) => msg,
            DeprecationNotice::DeprecatedDefaultPath(msg) => msg,
        }
    }
}

/// The structured value returned by [`resolve_base_dir`].
///
/// `path` is always present and never empty.  `notice` is `Some` only
/// when the chosen path was selected via a deprecated mechanism (branches
/// 2 and 4 in the resolver's five-branch precedence order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBaseDir {
    pub path: String,
    pub notice: Option<DeprecationNotice>,
}

/// Pure resolver for the Arai base directory.
///
/// Selects the base-directory path from a deterministic, five-branch
/// precedence order over injected environment lookups and existence
/// probes, and returns whether that selection warrants a deprecation
/// notice.
///
/// All world-touching capability is injected by the caller:
///
/// - `env_lookup` is invoked at most twice (with `"ARAI_BASE_DIR"` and
///   `"ARAI_DB_DIR"`).  It must return `Some(non_empty_string)` when the
///   variable is set to a non-empty value, and `None` when unset or
///   empty.
/// - `path_exists` is invoked at most twice (with the two well-known
///   default path strings constructed from `home_dir`).  It returns
///   `true` if the path currently exists on the filesystem.
/// - `home_dir` is the user's home directory as a string slice; the
///   caller is responsible for discovering it and for handling absent or
///   empty values before invoking the resolver.
///
/// The resolver evaluates branches in fixed order and returns on the
/// first match (short-circuit):
///
/// 1. `env_lookup("ARAI_BASE_DIR")` set → that value, no notice.
/// 2. else `env_lookup("ARAI_DB_DIR")` set → that value, `DeprecatedEnvVar` notice.
/// 3. else `path_exists(<home>/.taniwha/arai)` true → that path, no notice.
/// 4. else `path_exists(<home>/.arai)` true → that path, `DeprecatedDefaultPath` notice.
/// 5. else fresh-install fallback → `<home>/.taniwha/arai`, no notice.
pub fn resolve_base_dir<E, P>(
    env_lookup: E,
    path_exists: P,
    home_dir: &str,
) -> ResolvedBaseDir
where
    E: Fn(&str) -> Option<String>,
    P: Fn(&str) -> bool,
{
    // Branch 1: current canonical env var wins unconditionally.
    if let Some(value) = env_lookup("ARAI_BASE_DIR") {
        return ResolvedBaseDir {
            path: value,
            notice: None,
        };
    }

    // Branch 2: deprecated env var, with a notice.
    if let Some(value) = env_lookup("ARAI_DB_DIR") {
        let msg = "warning: ARAI_DB_DIR is deprecated; please rename it to \
                   ARAI_BASE_DIR. ARAI_DB_DIR will be removed in a future release."
            .to_string();
        return ResolvedBaseDir {
            path: value,
            notice: Some(DeprecationNotice::DeprecatedEnvVar(msg)),
        };
    }

    // Construct the two well-known default path strings from the home
    // directory.  Done with plain string concatenation so the resolver
    // never touches PathBuf or the filesystem.  A trailing slash on the
    // home value is tolerated.
    let home_trimmed = home_dir.trim_end_matches('/');
    let new_default = format!("{home_trimmed}/.taniwha/arai");
    let old_default = format!("{home_trimmed}/.arai");

    // Branch 3: new default exists — silent.
    if path_exists(&new_default) {
        return ResolvedBaseDir {
            path: new_default,
            notice: None,
        };
    }

    // Branch 4: only the old default exists — use it, with a notice that
    // mentions `arai migrate`.
    if path_exists(&old_default) {
        let msg = format!(
            "warning: ~/.arai is deprecated; the new default is ~/.taniwha/arai. \
             A forthcoming `arai migrate` command will help move your data to \
             the new location. (current path: {old_default})"
        );
        return ResolvedBaseDir {
            path: old_default,
            notice: Some(DeprecationNotice::DeprecatedDefaultPath(msg)),
        };
    }

    // Branch 5: fresh install — fall back to the new default, silently.
    ResolvedBaseDir {
        path: new_default,
        notice: None,
    }
}

impl Config {
    pub fn load() -> Result<Config, String> {
        let project_root = find_project_root()?;
        let home_dir = dirs::home_dir().ok_or("Could not determine home directory")?;

        // Resolve the base directory through the pure resolver, injecting
        // the real process environment and filesystem as the dependencies.
        let home_str = home_dir.to_string_lossy().to_string();
        let resolved = resolve_base_dir(
            |name| std::env::var(name).ok().filter(|v| !v.is_empty()),
            |p| std::path::Path::new(p).exists(),
            &home_str,
        );

        // Emit any deprecation notice to stderr only when stderr is
        // attached to an interactive terminal.  The TTY gate is the
        // caller's concern — the resolver does not inspect TTY state.
        if let Some(notice) = &resolved.notice {
            if std::io::stderr().is_terminal() {
                eprintln!("{}", notice.message());
            }
        }

        // Persist the notice before `resolved.path` is moved into
        // `arai_base_dir`.  The stderr-warning behaviour above is
        // unchanged; this clone is additive.
        let deprecation_notice = resolved.notice.clone();

        let arai_base_dir = PathBuf::from(resolved.path);

        let llm_command = std::env::var("ARAI_LLM_CMD").ok();
        let api_url = std::env::var("ARAI_API_URL").ok();
        let api_key_env = if std::env::var("ARAI_API_KEY").is_ok() {
            Some("ARAI_API_KEY".to_string())
        } else {
            None
        };
        let api_model = std::env::var("ARAI_API_MODEL").ok();

        let mut cfg = Config {
            project_root,
            home_dir,
            arai_base_dir,
            extra_sources: Vec::new(),
            guardrails_mode: "advise".to_string(),
            llm_command,
            api_url,
            api_key_env,
            api_model,
            deprecation_notice,
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
                if let Some(url) = enrich.get("api_url").and_then(|v| v.as_str()) {
                    if self.api_url.is_none() {
                        self.api_url = Some(url.to_string());
                    }
                }
                if let Some(key_env) = enrich.get("api_key_env").and_then(|v| v.as_str()) {
                    if self.api_key_env.is_none() {
                        self.api_key_env = Some(key_env.to_string());
                    }
                }
                if let Some(model) = enrich.get("model").and_then(|v| v.as_str()) {
                    if self.api_model.is_none() {
                        self.api_model = Some(model.to_string());
                    }
                }
            }
        }
    }

    /// Stable per-project slug: `<dirname>-<8char-sha256>`.  Used as the
    /// subdirectory name under `{arai_base}/projects/` and
    /// `{arai_base}/audit/`.
    pub fn project_slug(&self) -> String {
        let canonical = self.project_root.to_string_lossy().to_string();
        let dir_name = self
            .project_root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        let hash = hasher.finalize();
        let short_hash = hex_encode(&hash[..4]);

        format!("{dir_name}-{short_hash}")
    }

    /// DB path: ~/.taniwha/arai/projects/<dirname>-<8char-sha256>/arai.db
    pub fn db_path(&self) -> PathBuf {
        self.arai_base_dir
            .join("projects")
            .join(self.project_slug())
            .join("arai.db")
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
    use std::cell::RefCell;

    #[test]
    fn test_claude_memory_slug() {
        let cfg = Config {
            project_root: PathBuf::from("/usr/src/myproject"),
            home_dir: PathBuf::from("/usr/src"),
            arai_base_dir: PathBuf::from("/usr/src/.taniwha/arai"),
            extra_sources: Vec::new(),
            guardrails_mode: "advise".to_string(),
            llm_command: None,
            api_url: None,
            api_key_env: None,
            api_model: None,
            deprecation_notice: None,
        };
        assert_eq!(cfg.claude_memory_slug(), "-usr-src-myproject");
    }

    #[test]
    fn test_db_path_format() {
        let cfg = Config {
            project_root: PathBuf::from("/usr/src/myproject"),
            home_dir: PathBuf::from("/usr/src"),
            arai_base_dir: PathBuf::from("/usr/src/.taniwha/arai"),
            extra_sources: Vec::new(),
            guardrails_mode: "advise".to_string(),
            llm_command: None,
            api_url: None,
            api_key_env: None,
            api_model: None,
            deprecation_notice: None,
        };
        let db = cfg.db_path();
        let db_str = db.to_string_lossy();
        assert!(db_str.starts_with("/usr/src/.taniwha/arai/projects/myproject-"));
        assert!(db_str.ends_with("/arai.db"));
    }

    // -----------------------------------------------------------------
    // resolve_base_dir tests — AC1 through AC6.
    //
    // All tests use injected closures.  None set process environment
    // variables, none touch the filesystem, none alter $HOME.  Per AC7,
    // this is the only way to verify the resolver does not access
    // ambient state (the test bodies do not access ambient state, and
    // the resolver still produces correct results).
    // -----------------------------------------------------------------

    const HOME: &str = "/home/test";
    const NEW_DEFAULT: &str = "/home/test/.taniwha/arai";
    const OLD_DEFAULT: &str = "/home/test/.arai";

    /// Build an env-lookup closure from a fixed mapping.  Every call is
    /// recorded in `calls` so tests can assert short-circuit behaviour.
    fn env_with<'a>(
        map: Vec<(&'static str, Option<&'static str>)>,
        calls: &'a RefCell<Vec<String>>,
    ) -> impl Fn(&str) -> Option<String> + 'a {
        move |name: &str| {
            calls.borrow_mut().push(name.to_string());
            for (k, v) in &map {
                if *k == name {
                    return v.map(|s| s.to_string());
                }
            }
            None
        }
    }

    /// Build a path-exists closure from a fixed mapping.  Every call is
    /// recorded in `calls` so tests can assert short-circuit behaviour.
    fn exists_with<'a>(
        map: Vec<(&'static str, bool)>,
        calls: &'a RefCell<Vec<String>>,
    ) -> impl Fn(&str) -> bool + 'a {
        move |path: &str| {
            calls.borrow_mut().push(path.to_string());
            for (p, exists) in &map {
                if *p == path {
                    return *exists;
                }
            }
            false
        }
    }

    /// AC1 — Current env var wins unconditionally.
    ///
    /// `ARAI_BASE_DIR` set → that value, no notice, no further calls.
    /// We test with the deprecated env var BOTH set and unset, and with
    /// each path-exists response combination — the answer must be
    /// identical because branch 1 short-circuits all later branches.
    #[test]
    fn ac1_current_env_var_wins_unconditionally() {
        for db_dir in [None, Some("/some/other/path")] {
            for new_exists in [false, true] {
                for old_exists in [false, true] {
                    let env_calls = RefCell::new(Vec::new());
                    let path_calls = RefCell::new(Vec::new());

                    let result = resolve_base_dir(
                        env_with(
                            vec![
                                ("ARAI_BASE_DIR", Some("/explicit/path")),
                                ("ARAI_DB_DIR", db_dir),
                            ],
                            &env_calls,
                        ),
                        exists_with(
                            vec![(NEW_DEFAULT, new_exists), (OLD_DEFAULT, old_exists)],
                            &path_calls,
                        ),
                        HOME,
                    );

                    assert_eq!(result.path, "/explicit/path");
                    assert!(result.notice.is_none());

                    // Short-circuit: ARAI_DB_DIR must not be consulted,
                    // and path-exists must not be called at all.
                    let env = env_calls.borrow();
                    assert_eq!(env.len(), 1, "expected exactly one env-lookup call");
                    assert_eq!(env[0], "ARAI_BASE_DIR");
                    assert!(
                        path_calls.borrow().is_empty(),
                        "path_exists must not be called when ARAI_BASE_DIR is set"
                    );
                }
            }
        }
    }

    /// AC2 — Deprecated env var used when current env var absent.
    ///
    /// `ARAI_DB_DIR` set, `ARAI_BASE_DIR` unset → that value as path,
    /// notice variant `DeprecatedEnvVar` with a non-empty message.
    /// path-exists must not be consulted (branch 2 short-circuits 3-5).
    #[test]
    fn ac2_deprecated_env_var_used_when_current_absent() {
        for new_exists in [false, true] {
            for old_exists in [false, true] {
                let env_calls = RefCell::new(Vec::new());
                let path_calls = RefCell::new(Vec::new());

                let result = resolve_base_dir(
                    env_with(
                        vec![("ARAI_BASE_DIR", None), ("ARAI_DB_DIR", Some("/legacy/db"))],
                        &env_calls,
                    ),
                    exists_with(
                        vec![(NEW_DEFAULT, new_exists), (OLD_DEFAULT, old_exists)],
                        &path_calls,
                    ),
                    HOME,
                );

                assert_eq!(result.path, "/legacy/db");
                match result.notice {
                    Some(DeprecationNotice::DeprecatedEnvVar(msg)) => {
                        assert!(!msg.is_empty(), "notice message must be non-empty");
                    }
                    other => panic!("expected DeprecatedEnvVar, got {other:?}"),
                }
                assert!(
                    path_calls.borrow().is_empty(),
                    "path_exists must not be called when ARAI_DB_DIR is set"
                );
            }
        }
    }

    /// AC3 — New default used silently when it exists.
    #[test]
    fn ac3_new_default_used_silently_when_it_exists() {
        let env_calls = RefCell::new(Vec::new());
        let path_calls = RefCell::new(Vec::new());

        let result = resolve_base_dir(
            env_with(
                vec![("ARAI_BASE_DIR", None), ("ARAI_DB_DIR", None)],
                &env_calls,
            ),
            exists_with(vec![(NEW_DEFAULT, true), (OLD_DEFAULT, false)], &path_calls),
            HOME,
        );

        assert_eq!(result.path, NEW_DEFAULT);
        assert!(result.notice.is_none());
        // old-default must not have been probed (branch 3 short-circuits 4).
        assert!(
            !path_calls.borrow().iter().any(|p| p == OLD_DEFAULT),
            "old default must not be probed when new default exists"
        );
    }

    /// AC4 — Old default used with notice when only it exists.
    /// The notice message MUST mention `arai migrate`.
    #[test]
    fn ac4_old_default_used_with_notice_when_only_it_exists() {
        let env_calls = RefCell::new(Vec::new());
        let path_calls = RefCell::new(Vec::new());

        let result = resolve_base_dir(
            env_with(
                vec![("ARAI_BASE_DIR", None), ("ARAI_DB_DIR", None)],
                &env_calls,
            ),
            exists_with(vec![(NEW_DEFAULT, false), (OLD_DEFAULT, true)], &path_calls),
            HOME,
        );

        assert_eq!(result.path, OLD_DEFAULT);
        match result.notice {
            Some(DeprecationNotice::DeprecatedDefaultPath(msg)) => {
                assert!(!msg.is_empty(), "notice message must be non-empty");
                assert!(
                    msg.contains("arai migrate"),
                    "deprecated-default-path message must mention `arai migrate`, got: {msg}"
                );
            }
            other => panic!("expected DeprecatedDefaultPath, got {other:?}"),
        }
    }

    /// AC5 — Fresh-install fallback to new default with no notice.
    #[test]
    fn ac5_fresh_install_fallback_to_new_default_with_no_notice() {
        let env_calls = RefCell::new(Vec::new());
        let path_calls = RefCell::new(Vec::new());

        let result = resolve_base_dir(
            env_with(
                vec![("ARAI_BASE_DIR", None), ("ARAI_DB_DIR", None)],
                &env_calls,
            ),
            exists_with(vec![(NEW_DEFAULT, false), (OLD_DEFAULT, false)], &path_calls),
            HOME,
        );

        assert_eq!(result.path, NEW_DEFAULT);
        assert!(result.notice.is_none());
    }

    /// AC6 — New default takes precedence over old default when both exist.
    /// The result must be identical to AC3, not AC4.
    #[test]
    fn ac6_new_default_takes_precedence_over_old_when_both_exist() {
        let env_calls = RefCell::new(Vec::new());
        let path_calls = RefCell::new(Vec::new());

        let result = resolve_base_dir(
            env_with(
                vec![("ARAI_BASE_DIR", None), ("ARAI_DB_DIR", None)],
                &env_calls,
            ),
            exists_with(vec![(NEW_DEFAULT, true), (OLD_DEFAULT, true)], &path_calls),
            HOME,
        );

        // New default wins; no notice (silent — same as AC3).
        assert_eq!(result.path, NEW_DEFAULT);
        assert!(
            result.notice.is_none(),
            "AC6: when both defaults exist, result must match AC3 (no notice), not AC4"
        );
    }

    /// Determinism: two successive calls with the same inputs return
    /// structurally identical results.
    #[test]
    fn additional_determinism() {
        let make_env = || {
            |name: &str| -> Option<String> {
                if name == "ARAI_DB_DIR" {
                    Some("/legacy".to_string())
                } else {
                    None
                }
            }
        };
        let make_exists = || |_: &str| -> bool { false };

        let r1 = resolve_base_dir(make_env(), make_exists(), HOME);
        let r2 = resolve_base_dir(make_env(), make_exists(), HOME);
        assert_eq!(r1, r2);
    }

    /// Notice mutual exclusivity: branches 2 and 4 cannot both fire on a
    /// single call.  Branch 2 short-circuits before path-exists is
    /// consulted, so reaching branch 4 requires both env-lookups to
    /// return None.  Confirm by inspecting all returned notices across
    /// the branch grid.
    #[test]
    fn additional_notice_mutual_exclusivity() {
        // Branch 2: notice is DeprecatedEnvVar.
        let r2 = resolve_base_dir(
            |n: &str| {
                if n == "ARAI_DB_DIR" {
                    Some("/x".to_string())
                } else {
                    None
                }
            },
            |_: &str| true, // both defaults exist; should not matter
            HOME,
        );
        assert!(matches!(
            r2.notice,
            Some(DeprecationNotice::DeprecatedEnvVar(_))
        ));

        // Branch 4: notice is DeprecatedDefaultPath.
        let r4 = resolve_base_dir(|_: &str| None, |p: &str| p == OLD_DEFAULT, HOME);
        assert!(matches!(
            r4.notice,
            Some(DeprecationNotice::DeprecatedDefaultPath(_))
        ));
    }
}
