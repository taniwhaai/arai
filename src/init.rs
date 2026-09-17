use crate::{code_scanner, config, discovery, store};
use base64::Engine;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn run(pre_commit: bool, force: bool) -> Result<(), String> {
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
    for spec in HOSTS {
        match inject_host(spec, &cfg) {
            Ok(()) => {
                println!("    \u{2713} {} updated", spec.label);
                for line in trust_guidance(spec.kind) {
                    println!("      {line}");
                }
            }
            Err(e) if spec.optional => {
                eprintln!("    \u{26a0} {} registration skipped: {e}", spec.label);
            }
            Err(e) => return Err(e),
        }
    }

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

    println!(
        "\n  Arai is registered for Claude Code, Codex, Grok Build, and Cursor (PreToolUse hooks)."
    );
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
    let cfg = config::Config::load()?;
    let mut errors = Vec::new();
    for spec in HOSTS {
        match remove_host(spec, &cfg) {
            Ok(removed) if removed > 0 => {
                println!(
                    "  Removed {removed} Arai hook(s) from {}",
                    (spec.path)(&cfg).display()
                );
            }
            Ok(_) => {}
            Err(e) => errors.push(e),
        }
    }

    // A worktree's .git is a file. Ask Git where hooks live instead of
    // appending hooks to it; core.hooksPath can also redirect this path.
    if cfg.project_root.join(".git").exists() {
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
    println!("  Arai's project hook registrations removed. User/global hooks are unchanged.");
    Ok(())
}

/// One hook registration: the host's event name, its matcher pattern, and
/// whether Cursor's `failClosed` flag is set (grouped-layout hosts ignore
/// the flag).  Tool-call events use an empty matcher — Arai's own skip-tool
/// list filters.  FileChanged also uses the shared instruction-path filter
/// in-process, so nested AGENTS files and rule-directory .md/.mdc files
/// cannot be excluded by a second, narrower registration list.
struct Registration {
    event: &'static str,
    matcher: &'static str,
    fail_closed: bool,
}

const fn reg(event: &'static str, matcher: &'static str) -> Registration {
    Registration {
        event,
        matcher,
        fail_closed: false,
    }
}

/// How a host's hooks file arranges handlers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// `{"hooks": {Event: [{matcher, hooks: [handler]}]}}` — Claude Code,
    /// Codex and Grok Build.
    Grouped,
    /// `{"version": 1, "hooks": {event: [handler]}}` with `matcher` and
    /// `failClosed` on the handler itself — Cursor.
    Flat,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HostKind {
    Claude,
    Codex,
    Grok,
    Cursor,
}

/// Everything `init`, `ensure_hooks` and `deinit` need to know about one
/// host.  Adding a host is one entry here plus a response shape in
/// `hooks.rs`; the inject/remove code is shared.
struct HostSpec {
    kind: HostKind,
    label: &'static str,
    path: fn(&config::Config) -> PathBuf,
    layout: Layout,
    registrations: &'static [Registration],
    /// Delete the file on deinit when nothing but Arai's handlers was in
    /// it.  Only for a file Arai owns outright; shared settings files are
    /// rewritten, never removed, even when only empty arrays remain.
    delete_empty: bool,
    /// Registration failure is reported but does not abort `init`.
    optional: bool,
}

const CLAUDE_REGISTRATIONS: &[Registration] = &[
    reg("PreToolUse", ""),
    reg("PostToolUse", ""),
    reg("UserPromptSubmit", ""),
    reg("FileChanged", ""),
    reg("InstructionsLoaded", ""),
    // CwdChanged: monorepo navigation.  No matcher — every cd matters
    // because we may be landing in a never-scanned subpackage.
    reg("CwdChanged", ""),
    // PostToolBatch: parallel-tool compliance correlation.  No matcher
    // — Arai's own skip-tool list filters per-tool inside the handler.
    reg("PostToolBatch", ""),
    // PermissionDenied: classifier-disagreement audit + Warn-level
    // retry override.  Empty matcher — the handler inspects the
    // denied tool_input itself.
    reg("PermissionDenied", ""),
];

// Codex and Grok Build support the three tool-call events with a verified
// contract, plus SessionStart for the change-gated rescan (neither emits
// FileChanged / InstructionsLoaded).  Claude-specific file/reload events
// must not be copied into their configs.  Codex documents SessionStart
// matcher values (startup|resume|clear|compact); Grok's matcher is only
// documented against tool names, so its registration is unfiltered.
const CODEX_REGISTRATIONS: &[Registration] = &[
    reg("PreToolUse", ""),
    reg("PostToolUse", ""),
    reg("UserPromptSubmit", ""),
    reg("SessionStart", "startup|resume"),
];

const GROK_REGISTRATIONS: &[Registration] = &[
    reg("PreToolUse", ""),
    reg("PostToolUse", ""),
    reg("UserPromptSubmit", ""),
    reg("SessionStart", ""),
];

/// Cursor Agent hooks (cursor.com/docs/agent/hooks).  Cursor is fail-open
/// unless `failClosed` is set, so the decision event carries it; under that
/// flag "no output" counts as a failure, which is why the handler always
/// answers preToolUse explicitly.  postToolUse is limited to `Shell`
/// because file edits arrive through afterFileEdit with documented fields.
const CURSOR_REGISTRATIONS: &[Registration] = &[
    Registration {
        event: "preToolUse",
        matcher: "",
        fail_closed: true,
    },
    reg("postToolUse", "Shell"),
    reg("afterFileEdit", ""),
    reg("sessionStart", ""),
];

const HOSTS: &[HostSpec] = &[
    HostSpec {
        kind: HostKind::Claude,
        label: ".claude/settings.json",
        path: config::Config::claude_settings_path,
        layout: Layout::Grouped,
        registrations: CLAUDE_REGISTRATIONS,
        delete_empty: false,
        optional: false,
    },
    HostSpec {
        kind: HostKind::Codex,
        label: ".codex/hooks.json (native Codex)",
        path: config::Config::codex_hooks_path,
        layout: Layout::Grouped,
        registrations: CODEX_REGISTRATIONS,
        delete_empty: false,
        optional: false,
    },
    HostSpec {
        kind: HostKind::Cursor,
        label: ".cursor/hooks.json (native Cursor Agent hooks)",
        path: config::Config::cursor_hooks_path,
        layout: Layout::Flat,
        registrations: CURSOR_REGISTRATIONS,
        delete_empty: false,
        optional: false,
    },
    HostSpec {
        kind: HostKind::Grok,
        label: ".grok/hooks/arai.json (native Grok Build)",
        path: config::Config::grok_hooks_path,
        layout: Layout::Grouped,
        registrations: GROK_REGISTRATIONS,
        // Arai's own file: nothing else lives in it.
        delete_empty: true,
        // Non-fatal — users still get value via the .claude/settings.json
        // compatibility layer that Grok loads.
        optional: true,
    },
];

/// What the host still requires from the user after registration.  Writing
/// the file never grants trust; say so where each host gates on it.
fn trust_guidance(kind: HostKind) -> &'static [&'static str] {
    match kind {
        HostKind::Claude => &[
            "Claude Code runs project hooks only in a trusted workspace; accept the",
            "trust prompt for this folder or the registration stays inactive.",
        ],
        HostKind::Codex => &[
            "Codex requires a trusted project plus review of each new or changed",
            "hook definition in /hooks. Registration does not grant trust.",
            "User-level hooks also require review; managed policy may restrict hooks.",
        ],
        HostKind::Grok => &[
            "Grok Build gates project hooks behind trust: run /hooks-trust in the",
            "Grok session (or launch with --trust) the first time, or Arai's hooks",
            "stay inactive on the native path.",
        ],
        HostKind::Cursor => &[
            "Cursor loads project hooks from .cursor/hooks.json; check the Hooks tab",
            "in Cursor settings if a managed or team policy restricts them.",
        ],
    }
}

fn inject_host(spec: &HostSpec, cfg: &config::Config) -> Result<(), String> {
    let path = (spec.path)(cfg);
    let mut settings = read_hooks_file(&path)?;
    let root = settings
        .as_object_mut()
        .ok_or("Hook config is not an object")?;
    if spec.layout == Layout::Flat {
        root.entry("version")
            .or_insert_with(|| serde_json::json!(1));
    }
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or("hooks is not an object")?;
    let handler = arai_hook_handler(spec.kind)?;
    for registration in spec.registrations {
        let entries = hooks
            .entry(registration.event.to_string())
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .ok_or(format!("{} is not an array", registration.event))?;
        // Refresh the binary path and matcher on every init. Remove only our
        // handlers, not entire groups: a group may also contain user hooks.
        remove_arai_entries(entries, spec.layout);
        match spec.layout {
            Layout::Grouped => entries.push(serde_json::json!({
                "matcher": registration.matcher,
                "hooks": [handler.clone()]
            })),
            Layout::Flat => {
                let mut entry = handler.clone();
                if !registration.matcher.is_empty() {
                    entry["matcher"] = Value::String(registration.matcher.into());
                }
                if registration.fail_closed {
                    entry["failClosed"] = Value::Bool(true);
                }
                entries.push(entry);
            }
        }
    }
    write_hooks_file(&path, &settings)
}

fn remove_host(spec: &HostSpec, cfg: &config::Config) -> Result<usize, String> {
    let path = (spec.path)(cfg);
    if !path.exists() {
        return Ok(0);
    }
    let mut settings = read_hooks_file(&path)?;
    let mut removed = 0;
    if let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) {
        for entries in hooks.values_mut() {
            if let Some(entries) = entries.as_array_mut() {
                removed += remove_arai_entries(entries, spec.layout);
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
        if spec.delete_empty && hooks_only {
            std::fs::remove_file(&path)
                .map_err(|e| format!("Could not remove {}: {e}", path.display()))?;
        } else {
            write_hooks_file(&path, &settings)?;
        }
    }
    Ok(removed)
}

/// Strip Arai's own handlers from one event's entries, whichever layout the
/// host uses.  Returns how many were removed.
fn remove_arai_entries(entries: &mut Vec<Value>, layout: Layout) -> usize {
    match layout {
        Layout::Grouped => remove_arai_handlers(entries),
        Layout::Flat => {
            let before = entries.len();
            entries.retain(|handler| !is_arai_handler(handler));
            before - entries.len()
        }
    }
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

fn arai_hook_handler(kind: HostKind) -> Result<Value, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("Could not resolve current executable: {e}"))?;
    let native = exe.to_string_lossy();
    // Host command hooks use shell command strings. Single quotes also keep
    // dollar signs/backticks in an installation path from being evaluated.
    #[cfg(windows)]
    let path = native.replace('\\', "/");
    #[cfg(not(windows))]
    let path = native.to_string();
    let command = format!("'{}' guardrails --match-stdin", path.replace('\'', "'\\''"));
    let mut handler = serde_json::json!({
        "type": "command",
        "command": command,
        "timeout": 3
    });
    if cfg!(windows) {
        match kind {
            HostKind::Codex => {
                handler["commandWindows"] = Value::String(windows_command(&native));
            }
            // Cursor has no per-platform field and does not document which
            // shell runs `command` on Windows.  An explicit PowerShell
            // invocation with an encoded literal path parses the same under
            // cmd.exe and PowerShell, unlike the POSIX quoting above.
            HostKind::Cursor => {
                handler["command"] = Value::String(windows_command(&native));
            }
            HostKind::Claude | HostKind::Grok => {}
        }
    }
    Ok(handler)
}

const POWERSHELL_PREFIX: &str =
    "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ";

fn windows_command(path: &str) -> String {
    // Explicit interpreter and encoded literal path avoid relying on the
    // host's Windows shell or letting metacharacters become shell code.
    let script = format!(
        "& '{}' guardrails --match-stdin; exit $LASTEXITCODE",
        path.replace('\'', "''")
    );
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    format!(
        "{POWERSHELL_PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn is_arai_windows_command(command: &str) -> bool {
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
        .and_then(|body| body.strip_suffix("' guardrails --match-stdin; exit $LASTEXITCODE"))
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

fn is_arai_handler(handler: &Value) -> bool {
    // Every handler Arai writes carries `type: "command"`, including
    // Cursor's (where it is the documented default).  On Windows the Cursor
    // `command` is the encoded PowerShell form, so accept either spelling.
    handler.get("type").and_then(Value::as_str) == Some("command")
        && handler
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|command| {
                is_arai_command(command, "guardrails --match-stdin")
                    || is_arai_windows_command(command)
            })
        && handler
            .get("commandWindows")
            .is_none_or(|command| command.as_str().is_some_and(is_arai_windows_command))
}

fn remove_arai_handlers(groups: &mut Vec<Value>) -> usize {
    let mut removed = 0;
    groups.retain_mut(|group| {
        let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            return true;
        };
        let before = handlers.len();
        handlers.retain(|handler| !is_arai_handler(handler));
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

/// Ensure all supported host registrations exist for manual-only projects.
pub fn ensure_hooks() -> Result<(), String> {
    let cfg = config::Config::load()?;
    for spec in HOSTS {
        inject_host(spec, &cfg)?;
    }
    for line in trust_guidance(HostKind::Codex) {
        println!("      {line}");
    }
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
        let command = windows_command(path);
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
        assert!(!is_arai_windows_command(&windows_command(
            "C:\\tools\\unrelated.exe"
        )));
    }
}
