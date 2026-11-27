use crate::{code_scanner, config, discovery, store};
use base64::Engine;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn run(pre_commit: bool, force: bool) -> Result<(), String> {
    let cfg = config::Config::load()?;

    // Captured before any file is read: the session-start change check
    // compares instruction-file mtimes against this, so a save that lands
    // while the scan is still running is not mistaken for "already seen".
    let scan_started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

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

    db.set_meta("last_scan", &scan_started.to_string())
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

    render_enforceability_summary(&enforceability_summary(&cfg, &db)?);

    println!("\n  Setting up hooks...");
    register_default_hosts(&cfg, |outcome| match outcome.state {
        HostRegistrationState::Registered => {
            println!(
                "    ✓ {} registered ({})",
                outcome.registration_label,
                outcome.registration_path.display()
            );
            println!("      {}", outcome.trust_guidance);
        }
        HostRegistrationState::Skipped | HostRegistrationState::Failed => {
            let state = match outcome.state {
                HostRegistrationState::Skipped => "skipped",
                _ => "failed",
            };
            eprintln!(
                "    ⚠ {} registration {state} ({}): {}",
                outcome.registration_label,
                outcome.registration_path.display(),
                outcome.diagnostic.as_deref().unwrap_or_default()
            );
        }
    })?;

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

    println!("\n  Host registration attempts completed; writes do not establish trust or hook execution.");
    println!("  Re-run init after moving the Arai binary to refresh registered paths.");
    Ok(())
}

/// Render the canonical stored-policy summary without reclassifying rules.
pub fn render_enforceability_summary(summary: &EnforceabilitySummary) {
    println!("\n  Effective policy summary");
    println!("    Stored sources: {}", summary.stored_source_count);
    println!(
        "    Stored actionable rules: {}",
        summary.stored_actionable_rule_count
    );
    println!("    Block eligible: {}", summary.block_eligible_count);
    println!("    Advise eligible: {}", summary.advise_eligible_count);
    println!("    Documentary/inert: {}", summary.documentary_inert_count);
    println!("    Inactive: {}", summary.inactive_count);
    let mode = if summary.hard_bypass {
        "hard bypass (no enforcement)"
    } else if summary.advisory_only {
        "advisory only (deny disabled)"
    } else {
        "deny enabled"
    };
    println!("    Runtime mode: {mode}");
    println!(
        "    Eligibility is conditional on scope and the actual tool call; it is not a match."
    );
    for rule in &summary.rules {
        let class = match rule.classification {
            EnforceabilityClass::BlockEligible => "block eligible",
            EnforceabilityClass::AdviseEligible => "advise eligible",
            EnforceabilityClass::DocumentaryInert => {
                "documentary/inert — never fires on tool calls"
            }
            EnforceabilityClass::Inactive => "inactive",
        };
        let mut source = rule
            .source_path
            .clone()
            .unwrap_or_else(|| "source unavailable".into());
        if let Some(line) = rule.line_start {
            source.push_str(&format!(":{line}"));
            if let Some(end) = rule.line_end.filter(|end| *end != line) {
                source.push_str(&format!("-{end}"));
            }
        }
        println!(
            "    - rule {}: {class}, severity {} ({source})",
            rule.triple_id, rule.effective_severity
        );
        for reason in &rule.inactive_reasons {
            let reason = match reason {
                InactivityReason::Disabled => "disabled",
                InactivityReason::Expired => "expired",
                InactivityReason::PermanentSourceInactive => "permanent source inactivity",
                InactivityReason::RuntimeBypass => "runtime bypass",
            };
            println!("      Inactive reason: {reason}");
        }
        if let Some(reason) = &rule.source_inactive_reason {
            println!("      Source diagnostic: {reason}");
        }
        if let Some(directory) = &rule.scope_directory {
            println!("      Conditional directory: {directory}");
        }
        if !rule.scope_patterns.is_empty() {
            println!(
                "      Conditional paths: {}",
                rule.scope_patterns.join(", ")
            );
        }
    }
}

/// Stored policy counts and conditional eligibility, without a tool action.
#[derive(Debug, PartialEq, Eq)]
pub struct EnforceabilitySummary {
    pub stored_source_count: usize,
    pub stored_actionable_rule_count: usize,
    pub block_eligible_count: usize,
    pub advise_eligible_count: usize,
    pub documentary_inert_count: usize,
    pub inactive_count: usize,
    pub advisory_only: bool,
    pub hard_bypass: bool,
    pub rules: Vec<RuleEnforceabilityExplanation>,
}

/// Projection of one stored actionable row and its original scope/provenance.
#[derive(Debug, PartialEq, Eq)]
pub struct RuleEnforceabilityExplanation {
    pub triple_id: i64,
    pub source_path: Option<String>,
    pub line_start: Option<usize>,
    pub line_end: Option<usize>,
    pub effective_severity: String,
    pub classification: EnforceabilityClass,
    pub inactive_reasons: Vec<InactivityReason>,
    pub scope_directory: Option<String>,
    pub scope_patterns: Vec<String>,
    pub source_inactive_reason: Option<String>,
}

/// Disjoint potential eligibility categories; eligibility is not a match.
#[derive(Debug, PartialEq, Eq)]
pub enum EnforceabilityClass {
    BlockEligible,
    AdviseEligible,
    DocumentaryInert,
    Inactive,
}

/// Every applicable exclusion, without precedence between stored reasons.
#[derive(Debug, PartialEq, Eq)]
pub enum InactivityReason {
    Disabled,
    Expired,
    PermanentSourceInactive,
    RuntimeBypass,
}

/// Summarize the complete stored actionable population without changing it.
/// Runtime modes use the same environment interpretation as native hooks;
/// configuration's historical guardrails mode does not control denial.
pub fn enforceability_summary(
    _cfg: &config::Config,
    db: &store::Store,
) -> Result<EnforceabilitySummary, String> {
    let deny_enabled = crate::hooks::deny_mode_enabled();
    let hard_bypass = crate::hooks::is_disabled_via_env();
    let source_count = db.list_files().map_err(|e| e.to_string())?.len();
    let inventory = db.enforceability_inventory().map_err(|e| e.to_string())?;
    let scopes = db.source_scopes().map_err(|e| e.to_string())?;
    let mut summary = EnforceabilitySummary {
        stored_source_count: source_count,
        stored_actionable_rule_count: inventory.len(),
        block_eligible_count: 0,
        advise_eligible_count: 0,
        documentary_inert_count: 0,
        inactive_count: 0,
        advisory_only: !deny_enabled && !hard_bypass,
        hard_bypass,
        rules: Vec::with_capacity(inventory.len()),
    };
    for row in inventory {
        let guard = row.guardrail;
        let scope = scopes.get(&guard.file_path);
        let source_inactive_reason = scope
            .and_then(|scope| scope.inactive_reason())
            .map(str::to_owned);
        let mut inactive_reasons = Vec::new();
        if row.disabled {
            inactive_reasons.push(InactivityReason::Disabled);
        }
        if row.expired {
            inactive_reasons.push(InactivityReason::Expired);
        }
        if source_inactive_reason.is_some() {
            inactive_reasons.push(InactivityReason::PermanentSourceInactive);
        }
        if hard_bypass && inactive_reasons.is_empty() {
            inactive_reasons.push(InactivityReason::RuntimeBypass);
        }
        let severity = guard
            .intent
            .as_ref()
            .map(|intent| intent.severity)
            .unwrap_or_else(|| crate::intent::Severity::from_predicate(&guard.predicate));
        let classification = if !inactive_reasons.is_empty() {
            summary.inactive_count += 1;
            EnforceabilityClass::Inactive
        } else if guard.intent.as_ref().is_some_and(crate::intent::is_inert) {
            summary.documentary_inert_count += 1;
            EnforceabilityClass::DocumentaryInert
        } else if severity == crate::intent::Severity::Block && deny_enabled {
            summary.block_eligible_count += 1;
            EnforceabilityClass::BlockEligible
        } else {
            summary.advise_eligible_count += 1;
            EnforceabilityClass::AdviseEligible
        };
        let source_path = if !guard.file_path.is_empty() {
            Some(guard.file_path)
        } else if !guard.source_file.is_empty() {
            Some(guard.source_file)
        } else {
            None
        };
        let positive_line = |line: i64| usize::try_from(line).ok().filter(|line| *line > 0);
        summary.rules.push(RuleEnforceabilityExplanation {
            triple_id: guard.triple_id,
            source_path,
            line_start: guard.line_start.and_then(positive_line),
            line_end: row.line_end.and_then(positive_line),
            effective_severity: severity.as_str().to_owned(),
            classification,
            inactive_reasons,
            scope_directory: scope.and_then(|scope| scope.directory()).map(str::to_owned),
            scope_patterns: scope
                .map(|scope| scope.patterns().to_vec())
                .unwrap_or_default(),
            source_inactive_reason,
        });
    }
    Ok(summary)
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
    /// `{"version": 1, "hooks": {event: [handler]}}` with `bash`,
    /// `powershell`, and `timeoutSec` on the handler — GitHub Copilot.
    /// Handlers sit in a flat array, like Cursor, but the command fields
    /// differ and there is no `failClosed` flag.
    Copilot,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HostKind {
    Claude,
    Codex,
    Grok,
    Cursor,
    Copilot,
}

/// Everything `init`, `ensure_hooks` and `deinit` need to know about one
/// host.  Adding a host is one entry here plus a response shape in
/// `hooks.rs`; the inject/remove code is shared.
struct HostSpec {
    kind: HostKind,
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

/// GitHub Copilot command hooks (docs.github.com/copilot/reference/hooks-reference).
/// `userPromptSubmitted` command output is dropped, so the active-rule
/// summary is delivered on `sessionStart` instead. `postToolUse` carries
/// advisory text after the tool has already run.
const COPILOT_REGISTRATIONS: &[Registration] = &[
    reg("preToolUse", ""),
    reg("postToolUse", ""),
    reg("sessionStart", ""),
];

const HOSTS: &[HostSpec] = &[
    HostSpec {
        kind: HostKind::Claude,
        path: config::Config::claude_settings_path,
        layout: Layout::Grouped,
        registrations: CLAUDE_REGISTRATIONS,
        delete_empty: false,
        optional: false,
    },
    HostSpec {
        kind: HostKind::Codex,
        path: config::Config::codex_hooks_path,
        layout: Layout::Grouped,
        registrations: CODEX_REGISTRATIONS,
        delete_empty: false,
        optional: false,
    },
    HostSpec {
        kind: HostKind::Cursor,
        path: config::Config::cursor_hooks_path,
        layout: Layout::Flat,
        registrations: CURSOR_REGISTRATIONS,
        delete_empty: false,
        optional: false,
    },
    HostSpec {
        kind: HostKind::Grok,
        path: config::Config::grok_hooks_path,
        layout: Layout::Grouped,
        registrations: GROK_REGISTRATIONS,
        // Arai's own file: nothing else lives in it.
        delete_empty: true,
        // Non-fatal — users still get value via the .claude/settings.json
        // compatibility layer that Grok loads.
        optional: true,
    },
    HostSpec {
        kind: HostKind::Copilot,
        path: config::Config::copilot_hooks_path,
        layout: Layout::Copilot,
        registrations: COPILOT_REGISTRATIONS,
        // Arai's own file. A neighbour handler or any other key keeps it.
        delete_empty: true,
        // A failed write must surface: this file is the only hook source
        // the Copilot cloud agent loads.
        optional: false,
    },
];

/// Actual result of attempting one project host registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostRegistrationOutcome {
    /// Stable host identity.
    pub host: String,
    /// Project file targeted by the registration.
    pub registration_path: PathBuf,
    /// Human-readable host name.
    pub registration_label: String,
    /// Whether the write completed, was optionally skipped, or failed setup.
    pub state: HostRegistrationState,
    /// Original injection diagnostic, absent after a completed write.
    pub diagnostic: Option<String>,
    /// Existing host-specific steps needed beyond writing the project file.
    pub trust_guidance: String,
}

/// Registration status only; no state establishes trust or native execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostRegistrationState {
    /// The project configuration write completed.
    Registered,
    /// Optional Grok registration failed; later hosts are still attempted.
    Skipped,
    /// Mandatory registration failed; no later host is attempted.
    Failed,
}

/// Attempt default registrations in table order, reporting each actual result
/// synchronously before continuing or returning its unchanged failure.
pub fn register_default_hosts(
    cfg: &config::Config,
    mut report: impl FnMut(&HostRegistrationOutcome),
) -> Result<(), String> {
    for spec in HOSTS {
        let result = inject_host(spec, cfg);
        let (host, registration_label) = match spec.kind {
            HostKind::Claude => ("claude", "Claude"),
            HostKind::Codex => ("codex", "Codex"),
            HostKind::Cursor => ("cursor", "Cursor"),
            HostKind::Grok => ("grok", "Grok"),
            HostKind::Copilot => ("copilot", "Copilot"),
        };
        report(&HostRegistrationOutcome {
            host: host.into(),
            registration_path: (spec.path)(cfg),
            registration_label: registration_label.into(),
            state: match &result {
                Ok(()) => HostRegistrationState::Registered,
                Err(_) if spec.optional => HostRegistrationState::Skipped,
                Err(_) => HostRegistrationState::Failed,
            },
            diagnostic: result.as_ref().err().cloned(),
            trust_guidance: trust_guidance(spec.kind).join(" "),
        });
        if !spec.optional {
            result?;
        }
    }
    Ok(())
}

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
        HostKind::Copilot => &[
            "Registration does not grant workspace trust. The cloud agent loads only",
            ".github/hooks from the default branch; a missing arai binary exits 0 and",
            "does not enforce. The Copilot CLI may also run the Claude registration,",
            "so one tool call can be evaluated twice.",
        ],
    }
}

fn inject_host(spec: &HostSpec, cfg: &config::Config) -> Result<(), String> {
    let path = (spec.path)(cfg);
    let mut settings = read_hooks_file(&path)?;
    let root = settings
        .as_object_mut()
        .ok_or("Hook config is not an object")?;
    if matches!(spec.layout, Layout::Flat | Layout::Copilot) {
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
        let mut handler = handler.clone();
        // Session start walks the project for the change check; give it
        // more than the tool-call budget so a large tree is not killed
        // mid-check, which would silently leave the rule set stale.
        if registration.event.eq_ignore_ascii_case("sessionstart") {
            // Copilot reads `timeoutSec` (it wins over `timeout`). Other
            // hosts keep the `timeout` field they already document.
            if spec.kind == HostKind::Copilot {
                handler["timeoutSec"] = serde_json::json!(10);
            } else {
                handler["timeout"] = serde_json::json!(10);
            }
        }
        match spec.layout {
            Layout::Grouped => entries.push(serde_json::json!({
                "matcher": registration.matcher,
                "hooks": [handler]
            })),
            Layout::Flat | Layout::Copilot => {
                let mut entry = handler;
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
        if spec.delete_empty && owned_file_is_empty(&settings) {
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
        Layout::Flat | Layout::Copilot => {
            let before = entries.len();
            entries.retain(|handler| !is_arai_handler(handler));
            before - entries.len()
        }
    }
}

/// True when the file holds only `hooks` and/or `version`, and every hook
/// array is empty. A description, a neighbour handler, or any other key
/// keeps the file. Cursor's shared file is never passed here
/// (`delete_empty` is false).
fn owned_file_is_empty(settings: &Value) -> bool {
    let Some(object) = settings.as_object() else {
        return false;
    };
    if object.keys().any(|key| key != "hooks" && key != "version") {
        return false;
    }
    settings
        .get("hooks")
        .and_then(Value::as_object)
        .is_some_and(|hooks| {
            hooks
                .values()
                .all(|entries| entries.as_array().is_some_and(Vec::is_empty))
        })
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
    // Copilot's cloud agent runs `bash` and ignores `powershell`. The local
    // CLI uses the field for its OS. Omit `command` so neither host falls
    // through to a single shell string. A missing binary exits 0: an
    // absolute path that does not exist in the cloud sandbox would otherwise
    // fail closed on every tool call.
    if kind == HostKind::Copilot {
        return Ok(serde_json::json!({
            "type": "command",
            "bash": copilot_bash(&path),
            "powershell": copilot_powershell(&native),
            "timeoutSec": 3
        }));
    }
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
            // Claude can run the command under Git Bash or PowerShell.
            // Cursor does not specify its shell on Windows. An explicit
            // PowerShell invocation works across these shells and cmd.exe.
            HostKind::Claude | HostKind::Cursor => {
                handler["command"] = Value::String(windows_command(&native));
            }
            HostKind::Grok | HostKind::Copilot => {}
        }
    }
    Ok(handler)
}

const POWERSHELL_PREFIX: &str =
    "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ";
const POWERSHELL_STOP_ON_ERROR: &str = "$ErrorActionPreference = 'Stop'; ";

fn copilot_bash(path: &str) -> String {
    let quoted = path.replace('\'', "'\\''");
    format!("if [ -x '{quoted}' ]; then '{quoted}' guardrails --match-stdin; else exit 0; fi")
}

fn copilot_powershell(native: &str) -> String {
    let script = format!(
        "{POWERSHELL_STOP_ON_ERROR}$bin = '{}'; if (Test-Path -LiteralPath $bin) {{ & $bin guardrails --match-stdin; exit $LASTEXITCODE }} else {{ exit 0 }}",
        native.replace('\'', "''")
    );
    encode_powershell(&script)
}

fn encode_powershell(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    format!(
        "{POWERSHELL_PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn windows_command(path: &str) -> String {
    // Explicit interpreter and encoded literal path avoid relying on the
    // host's Windows shell or letting metacharacters become shell code.
    // A launch error must exit nonzero even when no native exit code exists.
    let script = format!(
        "{POWERSHELL_STOP_ON_ERROR}& '{}' guardrails --match-stdin; exit $LASTEXITCODE",
        path.replace('\'', "''")
    );
    encode_powershell(&script)
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
    let script = script
        .strip_prefix(POWERSHELL_STOP_ON_ERROR)
        .unwrap_or(&script);
    let Some(path) = script
        .strip_prefix("& '")
        .and_then(|body| body.strip_suffix("' guardrails --match-stdin; exit $LASTEXITCODE"))
    else {
        return false;
    };
    if path.replace("''", "").contains('\'') {
        return false;
    }
    is_arai_basename(path)
}

fn unescape_posix_single_quotes(inner: &str) -> Option<String> {
    if inner.replace("'\\''", "").contains('\'') {
        return None;
    }
    Some(inner.replace("'\\''", "'"))
}

fn is_arai_basename(path: &str) -> bool {
    matches!(path.rsplit(['/', '\\']).next(), Some("arai" | "arai.exe"))
}

/// Both copies of the path must be the same `arai` binary, and the suffix
/// must be the hook invocation with nothing after it.
fn is_copilot_bash(script: &str) -> bool {
    let Some(rest) = script.strip_prefix("if [ -x '") else {
        return false;
    };
    let Some((first, rest)) = rest.split_once("' ]; then '") else {
        return false;
    };
    let Some((second, tail)) = rest.split_once("' guardrails --match-stdin; else exit 0; fi")
    else {
        return false;
    };
    if !tail.is_empty() {
        return false;
    }
    let (Some(first), Some(second)) = (
        unescape_posix_single_quotes(first),
        unescape_posix_single_quotes(second),
    ) else {
        return false;
    };
    first == second && is_arai_basename(&first)
}

fn decode_powershell_script(command: &str) -> Option<String> {
    let encoded = command.strip_prefix(POWERSHELL_PREFIX)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    if bytes.len() % 2 != 0 {
        return None;
    }
    let utf16: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16(&utf16).ok()
}

fn is_copilot_powershell(command: &str) -> bool {
    let Some(script) = decode_powershell_script(command) else {
        return false;
    };
    let Some(script) = script.strip_prefix(POWERSHELL_STOP_ON_ERROR) else {
        return false;
    };
    let Some(rest) = script.strip_prefix("$bin = '") else {
        return false;
    };
    let Some((path, tail)) = rest.split_once(
        "'; if (Test-Path -LiteralPath $bin) { & $bin guardrails --match-stdin; exit $LASTEXITCODE } else { exit 0 }",
    ) else {
        return false;
    };
    if !tail.is_empty() || path.replace("''", "").contains('\'') {
        return false;
    }
    is_arai_basename(&path.replace("''", "'"))
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
    is_arai_basename(&path)
}

fn command_is_arai(command: &str) -> bool {
    is_arai_command(command, "guardrails --match-stdin") || is_arai_windows_command(command)
}

fn is_arai_handler(handler: &Value) -> bool {
    // Every handler Arai writes carries `type: "command"`, including
    // Cursor's (where it is the documented default). Accept the shell
    // command, the encoded PowerShell form used on Windows, and Copilot's
    // bash/powershell pair (which has no `command` field).
    if handler.get("type").and_then(Value::as_str) != Some("command") {
        return false;
    }
    if !handler
        .get("commandWindows")
        .is_none_or(|command| command.as_str().is_some_and(is_arai_windows_command))
    {
        return false;
    }
    if handler.get("bash").is_some() || handler.get("powershell").is_some() {
        let bash_ok = handler
            .get("bash")
            .and_then(Value::as_str)
            .is_some_and(is_copilot_bash);
        let powershell_ok = handler
            .get("powershell")
            .and_then(Value::as_str)
            .is_some_and(is_copilot_powershell);
        let command_ok = handler
            .get("command")
            .is_none_or(|command| command.as_str().is_some_and(command_is_arai));
        return bash_ok && powershell_ok && command_ok;
    }
    handler
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(command_is_arai)
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
            "$ErrorActionPreference = 'Stop'; & 'C:\\Arai''s $install\\arai.exe' guardrails --match-stdin; exit $LASTEXITCODE"
        );
        let legacy_bytes = &bytes[POWERSHELL_STOP_ON_ERROR.len() * 2..];
        let legacy = base64::engine::general_purpose::STANDARD.encode(legacy_bytes);
        assert!(is_arai_windows_command(&format!(
            "{POWERSHELL_PREFIX}{legacy}"
        )));
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

    #[test]
    fn copilot_wrapper_requires_both_paths_and_both_shells() {
        let bash = copilot_bash("/usr/local/bin/arai");
        let powershell = copilot_powershell(r"C:\tools\arai.exe");
        let handler = serde_json::json!({
            "type": "command",
            "bash": bash,
            "powershell": powershell,
            "timeoutSec": 3
        });
        assert!(is_copilot_bash(&bash));
        assert!(is_copilot_powershell(&powershell));
        assert!(is_arai_handler(&handler));

        let quoted = copilot_bash("/tmp/Arai's install/arai");
        assert!(is_copilot_bash(&quoted), "{quoted}");
        let quoted_ps = copilot_powershell(r"C:\Arai's install\arai.exe");
        assert!(is_copilot_powershell(&quoted_ps));

        let mismatched = "if [ -x '/usr/local/bin/arai' ]; then '/tmp/other/arai' guardrails --match-stdin; else exit 0; fi";
        assert!(!is_copilot_bash(mismatched));
        let mut wrong = handler.clone();
        wrong["bash"] = Value::String(mismatched.into());
        assert!(!is_arai_handler(&wrong));

        let mention = serde_json::json!({
            "type": "command",
            "bash": "echo arai guardrails --match-stdin",
            "powershell": powershell
        });
        assert!(!is_arai_handler(&mention));
        assert!(is_arai_handler(&serde_json::json!({
            "type": "command",
            "command": "'/usr/bin/arai' guardrails --match-stdin"
        })));
        assert!(owned_file_is_empty(
            &serde_json::json!({"version": 1, "hooks": {"preToolUse": []}})
        ));
        assert!(!owned_file_is_empty(
            &serde_json::json!({"version": 1, "hooks": {"preToolUse": []}, "description": "keep"})
        ));
        assert!(owned_file_is_empty(
            &serde_json::json!({"hooks": {"PreToolUse": []}})
        ));
    }
}
