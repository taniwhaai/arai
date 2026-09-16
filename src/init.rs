use crate::{code_scanner, config, discovery, platforms::Platform, store};
use base64::Engine;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn run(pre_commit: bool, force: bool) -> Result<(), String> {
    run_for_platforms(pre_commit, force, &[])
}

/// Register selected hosts. An empty request reuses the saved selection, or
/// the historical three hosts for projects that have never chosen platforms.
pub fn run_for_platforms(
    pre_commit: bool,
    force: bool,
    platforms: &[Platform],
) -> Result<(), String> {
    let cfg = config::Config::load()?;

    println!("  Scanning for instruction files...");
    let files = discovery::discover(&cfg)?;

    // Always open the store and register hooks — even when no instruction
    // files exist yet.  Users who start with `arai add` (manual rules only)
    // previously hit a silent dead path: init printed "No instruction files
    // found" and returned before writing `.claude/settings.json` or
    // `.grok/hooks/arai.json`, so the host never invoked Arai.  Rules that
    // present as present and aren't is the same shape of defect as #173.
    if files.is_empty() {
        println!("  No instruction files found.");
        println!("  (Hooks will still be registered so `arai add` rules can enforce.)");
    }

    for f in &files {
        let line_count = f.content.lines().count();
        println!(
            "    \u{2713} {} ({line_count} lines, {})",
            display_path(&f.path, &cfg),
            f.source_type
        );
    }

    println!("\n  Extracting rules...");
    let db = store::Store::open(&cfg.db_path())?;
    let platforms = if platforms.is_empty() {
        selected_platforms(&db)?
    } else {
        unique_platforms(platforms)
    };

    let total_rules = sync_discovered_rules(&db, &files)?;
    println!("    \u{2713} {total_rules} rules extracted");
    println!("    \u{2713} Discovered rules classified by intent");

    // Auto-enrich if the model is already downloaded
    let model_dir = cfg.arai_base_dir.join("models").join("all-MiniLM-L6-v2");
    if model_dir.join("model.onnx").exists() {
        match enrich_discovered_rules(&db, &cfg.arai_base_dir) {
            Ok(n) => println!("    \u{2713} {n} rules enriched by model"),
            Err(e) => eprintln!("    \u{26a0} Enrichment failed: {e}"),
        }
    }

    db.set_meta(
        "last_scan",
        &format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ),
    )
    .map_err(|e| e.to_string())?;

    println!("\n  Scanning source code for imports...");
    let imports = code_scanner::scan_project(&cfg.project_root);
    let import_count = imports.len();
    db.upsert_code_graph(&imports).map_err(|e| e.to_string())?;
    let tool_count = db.code_graph_tool_count().map_err(|e| e.to_string())?;
    let file_count = db.code_graph_file_count().map_err(|e| e.to_string())?;
    println!(
        "    \u{2713} {import_count} imports from {file_count} files, {tool_count} unique tools"
    );

    println!("\n  Setting up hooks...");
    register_platforms(&cfg, &platforms)?;
    save_platforms(&db, &platforms)?;

    // Track init event + flush queued telemetry
    let enrichment = if model_dir.join("model.onnx").exists() {
        "model"
    } else {
        "taxonomy"
    };
    crate::telemetry::track_init(
        &cfg.arai_base_dir,
        total_rules,
        files.len(),
        db.code_graph_tool_count().unwrap_or(0),
        enrichment,
    );
    crate::telemetry::flush(&cfg.arai_base_dir);

    if pre_commit {
        println!("\n  Installing git pre-commit hook...");
        install_pre_commit(&cfg, force)?;
        println!("    \u{2713} Git pre-commit hook → arai check-diff --cached");
    }

    if platforms.is_empty() {
        println!(
            "\n  No hook platforms selected. Use `arai init --platform <platform>` to enable one."
        );
    } else {
        println!(
            "\n  Arai hook registrations refreshed for: {}.",
            platforms
                .iter()
                .map(|platform| platform.id())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!("  Re-run init after moving the Arai binary to refresh registered paths.");
    Ok(())
}

/// Automatic enrichment is limited to sources owned by local discovery.
pub fn enrich_discovered_rules(db: &store::Store, base: &Path) -> Result<usize, String> {
    let sources = db.discovered_sources().map_err(|e| e.to_string())?;
    crate::enrich::enrich_guardrails_for_sources(db, base, &sources)
}

/// Classify one newly added source without resetting another owner's policy.
pub fn classify_source(db: &store::Store, path: &str) -> Result<(), String> {
    for guard in db.rules_for_file(path).map_err(|e| e.to_string())? {
        let intent = crate::intent::classify_rule_with_subject(
            &guard.predicate,
            &guard.object,
            Some(&guard.subject),
        );
        db.upsert_rule_intent(guard.triple_id, &intent)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn sync_discovered_rules(
    db: &store::Store,
    files: &[discovery::DiscoveredFile],
) -> Result<usize, String> {
    sync_discovered_rules_with_adoption(db, files, false)
}

/// Adopt only legacy rows that are present in this successful local snapshot,
/// and only when explicitly requested. Public API sources retain their owner.
pub fn sync_discovered_rules_with_adoption(
    db: &store::Store,
    files: &[discovery::DiscoveredFile],
    adopt_legacy: bool,
) -> Result<usize, String> {
    let count = db
        .sync_discovered_files(files, adopt_legacy)
        .map_err(|e| e.to_string())?;
    let legacy = db.legacy_source_count().map_err(|e| e.to_string())?;
    if legacy > 0 {
        eprintln!("  Retained {legacy} legacy source(s) with unknown ownership. To let local discovery manage matching on-disk instruction files, run `arai scan --adopt-legacy-sources`. Absent legacy sources remain retained; see docs/instruction-discovery.md for upgrade handling.");
    }
    for (path, scope) in db.source_scopes().map_err(|e| e.to_string())? {
        if let Some(reason) = scope.inactive_reason() {
            eprintln!("  Inactive source {path}: {reason}");
        }
    }
    Ok(count)
}

/// Remove Arai's project registrations, preserving other handlers and settings.
pub fn deinit() -> Result<(), String> {
    deinit_for_platforms(&[])
}

/// Remove selected hosts only. An empty request removes all project hook
/// registrations and Arai's pre-commit hook, and disables automatic re-adding.
pub fn deinit_for_platforms(platforms: &[Platform]) -> Result<(), String> {
    let cfg = config::Config::load()?;
    let db = store::Store::open(&cfg.db_path())?;
    let remove_all = platforms.is_empty();
    let platforms = if remove_all {
        Platform::ALL.to_vec()
    } else {
        unique_platforms(platforms)
    };
    let mut remaining = if remove_all {
        Vec::new()
    } else {
        selected_platforms(&db)?
    };
    let mut errors = Vec::new();
    for platform in &platforms {
        let path = hooks_path(&cfg, *platform);
        match remove_hooks_file(&path, *platform) {
            Ok(removed) => {
                remaining.retain(|selected| selected != platform);
                if removed > 0 {
                    println!("  Removed {removed} Arai hook(s) from {}", path.display());
                }
            }
            Err(e) => errors.push(e),
        }
    }
    // Persist successful removals even if an unrelated config needs repair.
    if let Err(error) = save_platforms(&db, &remaining) {
        errors.push(error);
    }

    // A worktree's .git is a file. Ask Git where hooks live instead of
    // appending hooks to it; core.hooksPath can also redirect this path.
    if remove_all && cfg.project_root.join(".git").exists() {
        match git_pre_commit_path(&cfg) {
            Ok(path) if path.exists() => match std::fs::read_to_string(&path) {
                Ok(body) if is_arai_pre_commit(&body) => {
                    if let Err(e) = std::fs::remove_file(&path) {
                        errors.push(format!("Could not remove {}: {e}", path.display()));
                    } else {
                        println!("  Removed Arai pre-commit hook from {}", path.display());
                    }
                }
                Ok(_) => {}
                Err(e) => errors.push(format!("Could not read {}: {e}", path.display())),
            },
            Ok(_) => {}
            Err(e) => errors.push(e),
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    if remove_all {
        println!("  Arai's project hook registrations removed. User/global hooks are unchanged.");
    } else {
        println!("  Selected Arai project hooks removed: {}. Other platforms and pre-commit are unchanged.",
            platforms.iter().map(|platform| platform.id()).collect::<Vec<_>>().join(", "));
    }
    Ok(())
}

fn remove_hooks_file(path: &Path, platform: Platform) -> Result<usize, String> {
    if !path.exists() {
        return Ok(0);
    }
    let mut settings = read_hooks_file(path)?;
    validate_hooks_shape(path, &settings, platform)?;
    let mut removed = 0;
    if let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) {
        for (event, entries) in hooks.iter_mut() {
            if let Some(groups) = entries.as_array_mut() {
                if platform == Platform::Cursor {
                    let before = groups.len();
                    groups.retain(|handler| !is_cursor_handler(handler, event));
                    removed += before - groups.len();
                } else {
                    removed += remove_arai_handlers(groups, platform);
                }
            }
        }
    }
    if removed > 0 {
        let hooks_only = settings.as_object().is_some_and(|object| object.len() == 1)
            && settings["hooks"].as_object().is_some_and(|hooks| {
                hooks
                    .values()
                    .all(|entries| entries.as_array().is_some_and(Vec::is_empty))
            });
        if platform == Platform::Grok && hooks_only {
            std::fs::remove_file(path)
                .map_err(|e| format!("Could not remove {}: {e}", path.display()))?;
        } else {
            write_hooks_file(path, &settings)?;
        }
    }
    Ok(removed)
}
/// Hook events Arai registers itself into and the `matcher` pattern for
/// each.  Tool-call events (`PreToolUse`/`PostToolUse`/`UserPromptSubmit`)
/// use an empty matcher — Arai's own skip-tool list filters. FileChanged
/// also uses the shared instruction-path filter in-process, so nested
/// AGENTS files and rule-directory .md/.mdc files cannot be excluded by
/// a second, narrower registration list.
const ARAI_HOOK_REGISTRATIONS: &[(&str, &str)] = &[
    ("PreToolUse", ""),
    ("PostToolUse", ""),
    ("UserPromptSubmit", ""),
    ("FileChanged", ""),
    ("InstructionsLoaded", ""),
    // CwdChanged: monorepo navigation.  No matcher — every cd matters
    // because we may be landing in a never-scanned subpackage.
    ("CwdChanged", ""),
    // PostToolBatch: parallel-tool compliance correlation.  No matcher
    // — Arai's own skip-tool list filters per-tool inside the handler.
    ("PostToolBatch", ""),
    // PermissionDenied: classifier-disagreement audit + Warn-level
    // retry override.  Empty matcher — the handler inspects the
    // denied tool_input itself.
    ("PermissionDenied", ""),
];

// These hosts support the three events Arai handles with a verified contract.
// Claude-specific file/reload events must not be copied into their configs.
const NATIVE_HOOK_REGISTRATIONS: &[(&str, &str)] = &[
    ("PreToolUse", ""),
    ("PostToolUse", ""),
    ("UserPromptSubmit", ""),
];

const PLATFORM_SELECTION_META: &str = "hook_platforms";

fn hooks_path(cfg: &config::Config, platform: Platform) -> PathBuf {
    match platform {
        Platform::Claude => cfg.claude_settings_path(),
        Platform::Grok => cfg.grok_hooks_dir().join("arai.json"),
        Platform::Codex => cfg.codex_hooks_path(),
        Platform::Cursor => cfg.project_root.join(platform.config_path()),
    }
}

fn unique_platforms(platforms: &[Platform]) -> Vec<Platform> {
    Platform::ALL
        .into_iter()
        .filter(|platform| platforms.contains(platform))
        .collect()
}

fn selected_platforms(db: &store::Store) -> Result<Vec<Platform>, String> {
    let Some(value) = db
        .get_meta(PLATFORM_SELECTION_META)
        .map_err(|e| e.to_string())?
    else {
        return Ok(Platform::LEGACY_DEFAULTS.to_vec());
    };
    let ids: Vec<String> = serde_json::from_str(&value)
        .map_err(|e| format!("Invalid saved hook platform selection: {e}"))?;
    let mut platforms = Vec::new();
    for id in ids {
        let platform = Platform::ALL
            .into_iter()
            .find(|platform| platform.id() == id)
            .ok_or_else(|| format!("Unknown saved hook platform: {id}"))?;
        platforms.push(platform);
    }
    Ok(unique_platforms(&platforms))
}

fn save_platforms(db: &store::Store, platforms: &[Platform]) -> Result<(), String> {
    let ids: Vec<_> = platforms.iter().map(|platform| platform.id()).collect();
    let value = serde_json::to_string(&ids).map_err(|e| e.to_string())?;
    db.set_meta(PLATFORM_SELECTION_META, &value)
        .map_err(|e| e.to_string())
}

fn register_platforms(cfg: &config::Config, platforms: &[Platform]) -> Result<(), String> {
    // Prepare every selected document before changing any of them. A malformed
    // file must not leave another selected host partially registered.
    let mut prepared = Vec::new();
    for platform in platforms {
        let path = hooks_path(cfg, *platform);
        let settings = if *platform == Platform::Cursor {
            prepare_cursor_hooks(&path)?
        } else {
            prepare_hooks_file(&path, *platform)?
        };
        prepared.push((platform, path, settings));
    }
    for (platform, path, settings) in prepared {
        write_hooks_file(&path, &settings)?;
        println!("    \u{2713} {} updated", platform.config_path());
        match platform {
            Platform::Codex => print_codex_trust_guidance(),
            Platform::Grok => {
                println!("      Grok Build requires project hook trust: run /hooks-trust in the");
                println!("      Grok session (or launch with --trust) before using native hooks.");
            }
            Platform::Cursor => {
                println!("      Cursor requires a trusted workspace. Verify both events in its Hooks output.");
                println!(
                    "      If third-party Claude hook imports are enabled, those hooks also run."
                );
                println!(
                    "      Choose one Arai integration in Cursor to avoid duplicate enforcement."
                );
                println!("      Arai has not changed third-party imports or user/global hooks.");
            }
            Platform::Claude => {}
        }
    }
    Ok(())
}

fn print_codex_trust_guidance() {
    println!("      Codex requires a trusted project plus review of each new or changed");
    println!("      hook definition in /hooks. Registration does not grant trust.");
    println!("      User-level hooks also require review; managed policy may restrict hooks.");
}

fn read_hooks_file(path: &Path) -> Result<Value, String> {
    let settings = if path.exists() {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Could not parse {}: {e}", path.display()))?
    } else {
        serde_json::json!({})
    };
    if !settings.is_object() {
        return Err(format!("{} is not an object", path.display()));
    }
    Ok(settings)
}

fn write_hooks_file(path: &Path, settings: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
    }
    let output = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Could not serialize {}: {e}", path.display()))?;
    std::fs::write(path, output).map_err(|e| format!("Could not write {}: {e}", path.display()))
}

fn prepare_hooks_file(path: &Path, platform: Platform) -> Result<Value, String> {
    let mut settings = read_hooks_file(path)?;
    validate_hooks_shape(path, &settings, platform)?;
    let registrations = if platform == Platform::Claude {
        ARAI_HOOK_REGISTRATIONS
    } else {
        NATIVE_HOOK_REGISTRATIONS
    };
    let hooks = settings
        .as_object_mut()
        .ok_or("Hook config is not an object")?
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let hooks_obj = hooks.as_object_mut().ok_or("hooks is not an object")?;
    let handler = arai_hook_handler(platform == Platform::Codex)?;
    for (event, matcher) in registrations {
        let groups = hooks_obj
            .entry(event.to_string())
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .ok_or(format!("{event} is not an array"))?;
        // Refresh the binary path and matcher on every init. Remove only our
        // handlers, not entire groups: a group may also contain user hooks.
        remove_arai_handlers(groups, platform);
        groups.push(serde_json::json!({
            "matcher": matcher,
            "hooks": [handler.clone()]
        }));
    }
    Ok(settings)
}

fn validate_hooks_shape(path: &Path, settings: &Value, platform: Platform) -> Result<(), String> {
    if platform == Platform::Cursor
        && path.exists()
        && settings.get("version").and_then(Value::as_u64) != Some(1)
    {
        return Err(format!(
            "{} must declare Cursor hooks version 1",
            path.display()
        ));
    }
    if let Some(hooks) = settings.get("hooks") {
        let hooks = hooks
            .as_object()
            .ok_or_else(|| format!("{}: hooks is not an object", path.display()))?;
        for (event, entries) in hooks {
            let entries = entries
                .as_array()
                .ok_or_else(|| format!("{}: {event} is not an array", path.display()))?;
            for entry in entries {
                if !entry.is_object() {
                    return Err(format!(
                        "{}: {event} contains a non-object hook",
                        path.display()
                    ));
                }
                if platform == Platform::Cursor {
                    if entry.get("hooks").is_some() {
                        return Err(format!(
                            "{}: {event} requires flat Cursor hook definitions",
                            path.display()
                        ));
                    }
                } else if entry.get("hooks").is_some_and(|hooks| !hooks.is_array()) {
                    return Err(format!(
                        "{}: {event} hook group is not an array",
                        path.display()
                    ));
                }
            }
        }
    }
    Ok(())
}

fn cursor_arguments(event: &str) -> String {
    format!("guardrails --match-stdin --platform cursor --hook-event {event}")
}

fn prepare_cursor_hooks(path: &Path) -> Result<Value, String> {
    let mut settings = read_hooks_file(path)?;
    validate_hooks_shape(path, &settings, Platform::Cursor)?;
    settings["version"] = Value::from(1);
    let hooks = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .unwrap();
    let exe = std::env::current_exe()
        .map_err(|e| format!("Could not resolve current executable: {e}"))?;
    for event in [Platform::Cursor.pre_event(), Platform::Cursor.post_event()] {
        let arguments = cursor_arguments(event);
        let command = if cfg!(windows) {
            windows_command(&exe.to_string_lossy(), &arguments)
        } else {
            format!(
                "'{}' {arguments}",
                exe.to_string_lossy().replace('\'', "'\\''")
            )
        };
        let handlers = hooks
            .entry(event)
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .unwrap();
        handlers.retain(|handler| !is_cursor_handler(handler, event));
        let mut handler = serde_json::json!({"type":"command", "command":command, "timeout":3});
        if event == Platform::Cursor.pre_event() {
            handler["failClosed"] = Value::Bool(true);
        }
        handlers.push(handler);
    }
    Ok(settings)
}

fn arai_hook_handler(codex: bool) -> Result<Value, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("Could not resolve current executable: {e}"))?;
    let path = exe.to_string_lossy();
    // Host command hooks use shell command strings. Single quotes also keep
    // dollar signs/backticks in an installation path from being evaluated.
    #[cfg(windows)]
    let path = path.replace('\\', "/");
    let command = format!("'{}' guardrails --match-stdin", path.replace('\'', "'\\''"));
    let mut handler = serde_json::json!({
        "type": "command",
        "command": command,
        "timeout": 3
    });
    if codex && cfg!(windows) {
        handler["commandWindows"] = Value::String(codex_windows_command(&exe.to_string_lossy()));
    }
    Ok(handler)
}

const POWERSHELL_PREFIX: &str =
    "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ";

fn codex_windows_command(path: &str) -> String {
    windows_command(path, "guardrails --match-stdin")
}

fn windows_command(path: &str, arguments: &str) -> String {
    // Explicit interpreter and encoded literal path avoid relying on the
    // host's Windows shell or letting metacharacters become shell code.
    let script = format!(
        "& '{}' {arguments}; exit $LASTEXITCODE",
        path.replace('\'', "''")
    );
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    format!(
        "{POWERSHELL_PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

#[cfg(test)]
fn is_arai_windows_command(command: &str) -> bool {
    is_windows_command(command, "guardrails --match-stdin")
}

fn is_windows_command(command: &str, arguments: &str) -> bool {
    let Some(encoded) = command.strip_prefix(POWERSHELL_PREFIX) else {
        return false;
    };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
        return false;
    };
    if bytes.len() % 2 != 0 {
        return false;
    }
    let utf16: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let Ok(script) = String::from_utf16(&utf16) else {
        return false;
    };
    let Some(path) = script
        .strip_prefix("& '")
        .and_then(|body| body.strip_suffix(&format!("' {arguments}; exit $LASTEXITCODE")))
    else {
        return false;
    };
    if path.replace("''", "").contains('\'') {
        return false;
    }
    matches!(path.rsplit(['/', '\\']).next(), Some("arai" | "arai.exe"))
}

/// Recognize our complete invocation, never an arbitrary mention of "arai".
/// Accept the bare and quoted absolute paths emitted by previous versions.
fn is_arai_command(command: &str, arguments: &str) -> bool {
    let Some(executable) = command.trim().strip_suffix(arguments) else {
        return false;
    };
    if !executable.ends_with(char::is_whitespace) {
        return false;
    }
    let executable = executable.trim_end();
    let path =
        if executable.len() >= 2 && executable.starts_with('\'') && executable.ends_with('\'') {
            let inner = &executable[1..executable.len() - 1];
            // The only interior quote form emitted by POSIX registration.
            if inner.replace("'\\''", "").contains('\'') {
                return false;
            }
            inner.replace("'\\''", "'")
        } else if executable.len() >= 2 && executable.starts_with('"') && executable.ends_with('"')
        {
            let inner = &executable[1..executable.len() - 1];
            if inner.contains('"') {
                return false;
            }
            inner.to_string()
        } else {
            if executable
                .chars()
                .any(|c| c.is_whitespace() || "'\";&|`$<>".contains(c))
            {
                return false;
            }
            executable.to_string()
        };
    matches!(path.rsplit(['/', '\\']).next(), Some("arai" | "arai.exe"))
}

fn is_arai_handler(handler: &Value, platform: Platform) -> bool {
    if handler.get("type").and_then(Value::as_str) != Some("command") {
        return false;
    }
    let explicit = format!("guardrails --match-stdin --platform {}", platform.id());
    let mut arguments = vec!["guardrails --match-stdin".to_string(), explicit.clone()];
    let registrations = if platform == Platform::Claude {
        ARAI_HOOK_REGISTRATIONS
    } else {
        NATIVE_HOOK_REGISTRATIONS
    };
    arguments.extend(
        registrations
            .iter()
            .map(|(event, _)| format!("{explicit} --hook-event {event}")),
    );
    arguments.iter().any(|arguments| {
        handler
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|command| is_arai_command(command, arguments))
            && handler.get("commandWindows").is_none_or(|command| {
                command
                    .as_str()
                    .is_some_and(|command| is_windows_command(command, arguments))
            })
    })
}

fn is_cursor_handler(handler: &Value, event: &str) -> bool {
    if ![Platform::Cursor.pre_event(), Platform::Cursor.post_event()].contains(&event)
        || handler
            .get("type")
            .is_some_and(|kind| kind.as_str() != Some("command"))
        || handler.get("commandWindows").is_some()
    {
        return false;
    }
    let arguments = cursor_arguments(event);
    handler
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| {
            is_arai_command(command, &arguments) || is_windows_command(command, &arguments)
        })
}

fn remove_arai_handlers(groups: &mut Vec<Value>, platform: Platform) -> usize {
    let mut removed = 0;
    groups.retain_mut(|group| {
        let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            return true;
        };
        let before = handlers.len();
        handlers.retain(|handler| !is_arai_handler(handler, platform));
        let count = before - handlers.len();
        removed += count;
        // Preserve existing empty groups, but discard groups emptied by us.
        count == 0 || !handlers.is_empty()
    });
    removed
}

const PRE_COMMIT_MARKER: &str = "# arai-managed-pre-commit: v1";
const PRE_COMMIT_DESCRIPTION: &str =
    "# Installed by `arai init --pre-commit`. Evaluates the staged diff";
const PRE_COMMIT_BYPASS: &str = "# against Arai guardrails. Bypass with `git commit --no-verify`.";

fn is_arai_pre_commit(body: &str) -> bool {
    let mut lines = body.lines();
    if lines.next() != Some("#!/bin/sh") {
        return false;
    }
    let mut description = lines.next();
    if description == Some(PRE_COMMIT_MARKER) {
        description = lines.next();
    }
    if description != Some(PRE_COMMIT_DESCRIPTION) || lines.next() != Some(PRE_COMMIT_BYPASS) {
        return false;
    }
    let Some(command) = lines.next().and_then(|line| line.strip_prefix("exec ")) else {
        return false;
    };
    is_arai_command(command, "check-diff --cached") && lines.next().is_none()
}

fn git_pre_commit_path(cfg: &config::Config) -> Result<PathBuf, String> {
    let output = std::process::Command::new("git")
        .args([
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "hooks/pre-commit",
        ])
        .current_dir(&cfg.project_root)
        .output()
        .map_err(|e| format!("Could not locate Git hooks: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not locate Git pre-commit hook: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value =
        String::from_utf8(output.stdout).map_err(|e| format!("Git hook path is not UTF-8: {e}"))?;
    let path = PathBuf::from(value.trim_end_matches(['\r', '\n']));
    if !path.is_absolute() {
        return Err("Git did not return an absolute pre-commit hook path".to_string());
    }
    Ok(path)
}

/// Install a managed hook at Git's configured pre-commit path. Refresh our
/// own registration, and require --force before replacing any other hook.
pub fn install_pre_commit(cfg: &config::Config, force: bool) -> Result<(), String> {
    let path = git_pre_commit_path(cfg)?;
    if path.exists() && !force {
        let body = std::fs::read_to_string(&path)
            .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
        if !is_arai_pre_commit(&body) {
            return Err(format!(
                "{} already exists; pass --force to overwrite",
                path.display()
            ));
        }
    }
    let exe = std::env::current_exe()
        .map_err(|e| format!("Could not resolve current executable: {e}"))?;
    let exe_str = exe.to_string_lossy();
    #[cfg(windows)]
    let exe_str = exe_str.replace('\\', "/");
    let exe_str = exe_str.replace('\'', "'\\''");
    let script = format!(
        "#!/bin/sh\n{PRE_COMMIT_MARKER}\n{PRE_COMMIT_DESCRIPTION}\n{PRE_COMMIT_BYPASS}\nexec '{exe_str}' check-diff --cached\n"
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, script).map_err(|e| format!("Could not write pre-commit hook: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Could not chmod pre-commit hook: {e}"))?;
    }
    Ok(())
}

/// Refresh only selected hosts for manual-only projects. Without a saved
/// selection, preserve the historical Claude/Grok/Codex registration behavior.
pub fn ensure_hooks() -> Result<(), String> {
    let cfg = config::Config::load()?;
    let db = store::Store::open(&cfg.db_path())?;
    let platforms = selected_platforms(&db)?;
    register_platforms(&cfg, &platforms)?;
    save_platforms(&db, &platforms)?;
    Ok(())
}
/// Display a path relative to project root when possible.
fn display_path(path: &str, cfg: &config::Config) -> String {
    let root = cfg.project_root.to_string_lossy();
    if path.starts_with(root.as_ref()) {
        path[root.len()..].trim_start_matches('/').to_string()
    } else if path.starts_with(cfg.home_dir.to_string_lossy().as_ref()) {
        format!("~{}", &path[cfg.home_dir.to_string_lossy().len()..])
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod registration_tests {
    use super::*;

    #[test]
    fn ownership_requires_an_exact_arai_invocation() {
        for command in [
            "arai guardrails --match-stdin",
            "\"/old install/arai\" guardrails --match-stdin",
            "'/old install/arai' guardrails --match-stdin",
            "'C:/Arai'\\''s install/arai.exe' guardrails --match-stdin",
            "C:\\tools\\arai.exe guardrails --match-stdin",
        ] {
            assert!(
                is_arai_command(command, "guardrails --match-stdin"),
                "{command}"
            );
        }
        for command in [
            "echo arai guardrails --match-stdin",
            "echo harmless && arai guardrails --match-stdin",
            "/tools/arai-audit guardrails --match-stdin",
            "arai status",
            "arai guardrails --match-stdin && echo done",
            "' guardrails --match-stdin",
            "\" guardrails --match-stdin",
            "'bad' '/tools/arai' guardrails --match-stdin",
            "echo /tools/arai guardrails --match-stdin",
            " guardrails --match-stdin",
        ] {
            assert!(
                !is_arai_command(command, "guardrails --match-stdin"),
                "{command}"
            );
        }
    }

    #[test]
    fn windows_wrapper_is_literal_and_keeps_native_exit_status() {
        let path = "C:\\Arai's $install\\arai.exe";
        let command = codex_windows_command(path);
        assert!(is_arai_windows_command(&command));
        let encoded = command.strip_prefix(POWERSHELL_PREFIX).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let utf16: Vec<_> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(
            String::from_utf16(&utf16).unwrap(),
            "& 'C:\\Arai''s $install\\arai.exe' guardrails --match-stdin; exit $LASTEXITCODE"
        );
        assert!(!is_arai_windows_command("echo arai"));
        assert!(!is_arai_windows_command(&format!(
            "{POWERSHELL_PREFIX}AA=="
        )));
        assert!(!is_arai_windows_command(&format!(
            "{POWERSHELL_PREFIX}not-base64!"
        )));
        assert!(!is_arai_windows_command(&codex_windows_command(
            "C:\\tools\\unrelated.exe"
        )));
    }
}
