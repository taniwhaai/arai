mod audit;
mod code_scanner;
mod compliance;
mod config;
mod discovery;
mod enrich;
mod extends;
mod guardrails;
mod hooks;
mod init;
mod intent;
mod mcp;
mod parser;
mod scenarios;
mod session;
mod stats;
mod store;
mod telemetry;
mod upgrade;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "arai", version, about = "CLAUDE.md that actually works.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Discover instruction files, extract guardrails, set up hooks
    Init,
    /// Remove Arai hooks from .claude/settings.json
    Deinit,
    /// Show what's being enforced
    Status,
    /// List active guardrails or match against stdin
    Guardrails {
        /// Read Claude Code hook JSON from stdin and return matching guardrails
        #[arg(long)]
        match_stdin: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Re-scan instruction files and update guardrails
    Scan {
        /// Also scan source code for imports (builds code graph)
        #[arg(long)]
        code: bool,
        /// Enrich rule intent using a sentence transformer model (downloads ~80MB on first use)
        #[arg(long)]
        enrich: bool,
        /// Enrich rule intent using Claude Code (shells out to `claude -p`)
        #[arg(long)]
        enrich_llm: bool,
        /// Enrich rule intent via direct API call to an OpenAI-compatible endpoint
        #[arg(long)]
        enrich_api: bool,
        /// Import enrichment from a JSON file (for manual correction or sharing)
        #[arg(long, value_name = "FILE")]
        enrich_file: Option<String>,
    },
    /// Manually add a guardrail rule
    Add {
        /// The rule text (e.g. "Never force-push to main")
        rule: String,
    },
    /// Upgrade arai binary to latest version or switch variant
    Upgrade {
        /// Switch to full binary (with enrichment)
        #[arg(long)]
        full: bool,
        /// Switch to lean binary (without enrichment)
        #[arg(long)]
        lean: bool,
    },
    /// Show the local audit log of rule firings (no network egress)
    Audit {
        /// Only show firings newer than this (e.g. "7d", "24h", "30m")
        #[arg(long)]
        since: Option<String>,
        /// Filter by tool name (Bash, Edit, Write, ...)
        #[arg(long)]
        tool: Option<String>,
        /// Filter by hook event (PreToolUse, PostToolUse, UserPromptSubmit, Compliance)
        #[arg(long)]
        event: Option<String>,
        /// Filter Compliance entries by outcome (obeyed, ignored, unclear).
        /// Implies `--event=Compliance` unless one is set explicitly.
        #[arg(long)]
        outcome: Option<String>,
        /// Filter by rule subject/predicate/object (case-insensitive substring).
        /// Matches against any rule attached to the firing — handy for
        /// answering "every time the alembic rule fired this week".
        #[arg(long)]
        rule: Option<String>,
        /// Maximum entries to return
        #[arg(long, default_value = "50")]
        limit: usize,
        /// Output as JSON (one entry per line)
        #[arg(long)]
        json: bool,
    },
    /// Run the Ārai MCP server on stdio (for integration into Claude Code or
    /// any MCP-capable client).  Exposes `arai_add_guard` + `arai_list_guards`
    /// so the agent can program its own deterministic guardrails.
    Mcp,
    /// Parse an instruction file and show extracted rules with their
    /// classified intent.  No DB writes — use this to iterate on CLAUDE.md
    /// wording before committing.
    Lint {
        /// Path to a markdown instruction file
        file: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Manage the list of URLs trusted for `arai:extends` directives.
    /// Without arguments, shows the current list.
    Trust {
        /// Add this URL to the trust list
        #[arg(long, value_name = "URL")]
        add: Option<String>,
        /// Remove this URL from the trust list
        #[arg(long, value_name = "URL")]
        remove: Option<String>,
    },
    /// Replay synthetic hook scenarios against the current rule set to
    /// catch regressions in rule behaviour.  Scenarios are JSON describing
    /// a tool call + expected matches.
    Test {
        /// Path to a JSON scenario file
        file: String,
        /// Output as JSON instead of a pretty table
        #[arg(long)]
        json: bool,
    },
    /// Build a scenario file from recent audit-log entries.  Writes JSON
    /// to stdout — redirect into a scenarios file and tune by hand.
    Record {
        /// Only record firings newer than this (e.g. "1h", "7d")
        #[arg(long)]
        since: Option<String>,
        /// Filter by tool name (Bash, Edit, Write, ...)
        #[arg(long)]
        tool: Option<String>,
        /// Maximum audit entries to scan
        #[arg(long, default_value = "200")]
        limit: usize,
    },
    /// Aggregate summary of the local audit log — top rules, tools, days,
    /// plus per-rule compliance ratios when PostToolUse correlation has
    /// produced verdicts.
    Stats {
        /// Only count firings newer than this (e.g. "7d", "24h", "30m")
        #[arg(long)]
        since: Option<String>,
        /// Show top N entries per section
        #[arg(long, default_value = "10")]
        top: usize,
        /// Show only the per-rule compliance section (rules + their
        /// obeyed/ignored ratios).  Useful when you only care about
        /// "is the model honouring my rules?"
        #[arg(long)]
        by_rule: bool,
        /// Output as JSON instead of a table
        #[arg(long)]
        json: bool,
    },
    /// Pin a rule's severity so re-running `arai scan` won't reset it to the
    /// predicate-derived classification.  Use for incremental rollout — flip
    /// individual rules into deny mode while the rest of the set stays in
    /// advise mode.  No arguments → list current overrides.
    ///
    /// Examples:
    ///   arai severity                                # list overrides
    ///   arai severity alembic block                  # pin every rule whose
    ///                                                # subject/object contains
    ///                                                # "alembic" to block
    ///   arai severity --reset alembic                # back to classified
    Severity {
        /// Case-insensitive substring against subject/object.  Required when
        /// setting a severity; with `--reset` it picks which overrides to drop.
        pattern: Option<String>,
        /// New severity for matching rules: `block`, `warn`, or `inform`.
        /// Required unless `--reset` is set.
        level: Option<String>,
        /// Drop the override for matching rules; severity reverts to the
        /// predicate-derived classification.
        #[arg(long)]
        reset: bool,
        /// Output as JSON (machine-readable list of changes)
        #[arg(long)]
        json: bool,
    },
    /// Preview what changes a candidate edit to an instruction file would
    /// make to the live rule set — added, removed, severity changes — before
    /// you save and run `arai scan`.  Read-only against the store; pairs
    /// with `arai lint` (which previews the file in isolation) and `arai why`
    /// (which previews single-action firings).
    ///
    /// Examples:
    ///   arai diff CLAUDE.md
    ///   arai diff memory/feedback_testing.md --json   # for pre-commit hooks
    Diff {
        /// Path to the candidate instruction file.  The file must already be
        /// known to Arai (i.e. picked up by a previous `arai scan` or `arai
        /// init`); otherwise every rule in it would diff as "added", which
        /// `arai lint` covers more cleanly.
        file: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Explain which guardrails would fire on a hypothetical tool call —
    /// useful for debugging "why did this rule fire?" without running the
    /// hook live.  Pass either a Bash command or `--tool Edit <path>`.
    ///
    /// Examples:
    ///   arai why 'git push --force origin main'
    ///   arai why --tool Write /src/migrations/001_init.py
    Why {
        /// Bash command or tool input (depending on --tool).  Treated as a
        /// Bash command unless --tool is set.
        input: Vec<String>,
        /// Tool name to simulate (Bash, Edit, Write, Read, ...).  Defaults to Bash.
        #[arg(long, default_value = "Bash")]
        tool: String,
        /// Hook event to simulate (PreToolUse or PostToolUse)
        #[arg(long, default_value = "PreToolUse")]
        event: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init => init::run(),
        Commands::Deinit => init::deinit(),
        Commands::Status => cmd_status(),
        Commands::Guardrails { match_stdin, json } => {
            if match_stdin {
                hooks::handle_stdin()
            } else {
                cmd_guardrails(json)
            }
        }
        Commands::Scan { code, enrich, enrich_llm, enrich_api, enrich_file } => cmd_scan(code, enrich, enrich_llm, enrich_api, enrich_file),
        Commands::Add { rule } => cmd_add(&rule),
        Commands::Upgrade { full, lean } => upgrade::run(full, lean),
        Commands::Audit { since, tool, event, outcome, rule, limit, json } => cmd_audit(since, tool, event, outcome, rule, limit, json),
        Commands::Mcp => mcp::run(),
        Commands::Lint { file, json } => cmd_lint(&file, json),
        Commands::Trust { add, remove } => cmd_trust(add, remove),
        Commands::Test { file, json } => scenarios::run(std::path::Path::new(&file), json),
        Commands::Record { since, tool, limit } => cmd_record(since, tool, limit),
        Commands::Stats { since, top, by_rule, json } => cmd_stats(since, top, by_rule, json),
        Commands::Severity { pattern, level, reset, json } => cmd_severity(pattern, level, reset, json),
        Commands::Diff { file, json } => cmd_diff(&file, json),
        Commands::Why { input, tool, event, json } => cmd_why(input, tool, event, json),
    };

    if let Err(e) = result {
        eprintln!("arai: {e}");
        std::process::exit(1);
    }
}

fn cmd_status() -> Result<(), String> {
    let cfg = config::Config::load()?;
    let db = store::Store::open(&cfg.db_path())?;
    let files = db.list_files().map_err(|e| e.to_string())?;
    let count = db.guardrail_count().map_err(|e| e.to_string())?;
    let last_scan = db.get_meta("last_scan").map_err(|e| e.to_string())?;

    println!("Arai status");
    println!("  Rules:      {count}");
    println!("  Sources:    {} file(s)", files.len());
    for f in &files {
        println!("    - {f}");
    }
    if let Some(ts) = last_scan {
        println!("  Last scan:  {ts}");
    } else {
        println!("  Last scan:  never (run `arai init`)");
    }

    let graph_tools = db.code_graph_tool_count().map_err(|e| e.to_string())?;
    let graph_files = db.code_graph_file_count().map_err(|e| e.to_string())?;
    if graph_files > 0 {
        println!("  Code graph: {graph_tools} tools from {graph_files} files");
    } else {
        println!("  Code graph: not scanned (run `arai scan --code`)");
    }

    let issues = db.find_rule_issues().map_err(|e| e.to_string())?;
    if !issues.duplicates.is_empty() || !issues.opposing.is_empty() {
        println!();
        if !issues.duplicates.is_empty() {
            println!("  Duplicate rules ({}):", issues.duplicates.len());
            for d in issues.duplicates.iter().take(10) {
                println!(
                    "    - {} {}: {}",
                    d.subject, d.predicate, d.object,
                );
                for src in &d.sources {
                    println!("        from {src}");
                }
            }
            if issues.duplicates.len() > 10 {
                println!("    … {} more", issues.duplicates.len() - 10);
            }
        }
        if !issues.opposing.is_empty() {
            println!("  Opposing predicates ({}):", issues.opposing.len());
            for o in issues.opposing.iter().take(10) {
                println!(
                    "    - {} (predicates: {})",
                    o.subject,
                    o.predicates.join(", "),
                );
            }
            if issues.opposing.len() > 10 {
                println!("    … {} more", issues.opposing.len() - 10);
            }
        }
    }
    Ok(())
}

fn cmd_guardrails(json: bool) -> Result<(), String> {
    let cfg = config::Config::load()?;
    let db = store::Store::open(&cfg.db_path())?;
    let rules = db.load_guardrails().map_err(|e| e.to_string())?;

    if json {
        let out = serde_json::to_string_pretty(&rules).map_err(|e| e.to_string())?;
        println!("{out}");
    } else {
        if rules.is_empty() {
            println!("No guardrails found. Run `arai init` first.");
            return Ok(());
        }
        for r in &rules {
            println!("- {} {}: {}", r.subject, r.predicate, r.object);
        }
    }
    Ok(())
}

fn cmd_scan(code: bool, do_enrich: bool, enrich_llm: bool, enrich_api: bool, enrich_file: Option<String>) -> Result<(), String> {
    let cfg = config::Config::load()?;
    let files = discovery::discover(&cfg)?;
    let db = store::Store::open(&cfg.db_path())?;

    let mut total_rules = 0;
    for file in &files {
        let triples = parser::extract_rules(&file.content, &file.source_type, file.confidence);
        let count = triples.len();
        db.upsert_file(&file.path, &file.content, &triples, &file.source_type)
            .map_err(|e| e.to_string())?;
        if count > 0 {
            println!("  {} — {count} rule(s)", file.path);
        }
        total_rules += count;
    }

    db.classify_all_guardrails().map_err(|e| e.to_string())?;
    db.set_meta("last_scan", &chrono_now()).map_err(|e| e.to_string())?;
    println!("\n  {total_rules} rule(s) from {} file(s)", files.len());

    if code {
        scan_code_graph(&cfg, &db)?;
    }

    // Auto-enrich if model already downloaded, or if --enrich flag requests it
    let model_dir = cfg.arai_base_dir.join("models").join("all-MiniLM-L6-v2");
    if do_enrich || model_dir.join("model.onnx").exists() {
        println!("\n  Enriching rule intent with sentence transformer...");
        let enriched = enrich::enrich_guardrails(&db, &cfg.arai_base_dir)?;
        println!("    \u{2713} {enriched} rules enriched by model");
    }

    // LLM enrichment (explicit opt-in)
    if enrich_llm {
        println!("\n  Enriching rule intent via LLM...");
        let enriched = enrich::enrich_via_llm(&db, cfg.llm_command.as_deref(), &cfg.arai_base_dir)?;
        println!("    \u{2713} {enriched} rules enriched by LLM");
    }

    // API enrichment (direct HTTP call)
    if enrich_api {
        println!("\n  Enriching rule intent via API...");
        let enriched = enrich::enrich_via_api(
            &db,
            cfg.api_url.as_deref(),
            cfg.api_key_env.as_deref(),
            cfg.api_model.as_deref(),
            &cfg.arai_base_dir,
        )?;
        println!("    \u{2713} {enriched} rules enriched by API");
    }

    // File-based enrichment import
    if let Some(path) = enrich_file {
        println!("\n  Importing enrichment from {path}...");
        let enriched = enrich::enrich_from_file(&db, &path, &cfg.arai_base_dir)?;
        println!("    \u{2713} {enriched} rules imported");
    }

    Ok(())
}

fn scan_code_graph(cfg: &config::Config, db: &store::Store) -> Result<(), String> {
    println!("\n  Scanning source code for imports...");
    let imports = code_scanner::scan_project(&cfg.project_root);
    let import_count = imports.len();
    db.upsert_code_graph(&imports).map_err(|e| e.to_string())?;

    let tool_count = db.code_graph_tool_count().map_err(|e| e.to_string())?;
    let file_count = db.code_graph_file_count().map_err(|e| e.to_string())?;
    println!("    \u{2713} {import_count} imports from {file_count} files, {tool_count} unique tools");
    Ok(())
}

fn cmd_add(rule: &str) -> Result<(), String> {
    let cfg = config::Config::load()?;
    let db = store::Store::open(&cfg.db_path())?;

    let triples = parser::extract_rules(
        &format!("- {rule}"),
        "manual",
        0.95,
    );

    if triples.is_empty() {
        return Err(format!(
            "Could not extract a guardrail from: \"{rule}\"\nTry phrasing it as an imperative (e.g. \"Never force-push to main\")"
        ));
    }

    // Each manual rule gets a unique path based on content hash
    let manual_path = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(rule.as_bytes());
        let hash_bytes = h.finalize();
        let short: String = hash_bytes.iter().take(4).map(|b| format!("{b:02x}")).collect();
        format!("manual://arai-add/{short}")
    };
    db.upsert_file(&manual_path, rule, &triples, "manual")
        .map_err(|e| e.to_string())?;

    // Classify + enrich the new rule
    db.classify_all_guardrails().map_err(|e| e.to_string())?;
    let model_dir = cfg.arai_base_dir.join("models").join("all-MiniLM-L6-v2");
    if model_dir.join("model.onnx").exists() {
        enrich::enrich_guardrails(&db, &cfg.arai_base_dir).ok();
    }

    for t in &triples {
        println!("  Added: {} {}: {}", t.subject, t.predicate, t.object);
    }
    Ok(())
}

fn chrono_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", now.as_secs())
}

fn cmd_audit(
    since: Option<String>,
    tool: Option<String>,
    event: Option<String>,
    outcome: Option<String>,
    rule: Option<String>,
    limit: usize,
    json: bool,
) -> Result<(), String> {
    let cfg = config::Config::load()?;
    let since_epoch = since.as_deref().map(parse_since).transpose()?;

    // `--outcome` is a shortcut that implies `--event=Compliance` unless the
    // caller already scoped to a specific event (some future event type may
    // also carry outcomes).
    let effective_event: Option<String> = match (event.as_deref(), outcome.as_deref()) {
        (Some(e), _) => Some(e.to_string()),
        (None, Some(_)) => Some("Compliance".to_string()),
        (None, None) => None,
    };

    // Over-fetch when post-filters are in play so we don't truncate before
    // the substring match has had a chance to match.
    let needs_post_filter = outcome.is_some() || rule.is_some();
    let entries = audit::query(
        &cfg.arai_base_dir,
        &cfg.project_slug(),
        since_epoch,
        tool.as_deref(),
        effective_event.as_deref(),
        if needs_post_filter { limit.saturating_mul(4) } else { limit },
    )?;

    // Apply outcome filter in-process: an entry passes if any item in
    // `payload.rules[]` has `outcome == <filter>`.
    let entries: Vec<serde_json::Value> = if let Some(target) = outcome.as_deref() {
        entries
            .into_iter()
            .filter(|e| {
                e.get("payload")
                    .and_then(|p| p.get("rules"))
                    .and_then(|r| r.as_array())
                    .map(|rs| rs.iter().any(|r| r.get("outcome").and_then(|o| o.as_str()) == Some(target)))
                    .unwrap_or(false)
            })
            .collect()
    } else {
        entries
    };

    // Apply rule-pattern filter: an entry passes if any rule attached to it
    // (top-level `rules[]` for firings, or `payload.rules[]` for Compliance)
    // matches the pattern as a case-insensitive substring of subject /
    // predicate / object.
    let entries: Vec<serde_json::Value> = if let Some(pat) = rule.as_deref() {
        let needle = pat.to_lowercase();
        entries
            .into_iter()
            .filter(|e| entry_rules(e).any(|r| rule_matches_pattern(r, &needle)))
            .collect()
    } else {
        entries
    };

    let entries: Vec<serde_json::Value> = entries.into_iter().take(limit).collect();

    if entries.is_empty() {
        if json {
            // Still valid — just an empty stream.
            return Ok(());
        }
        println!("No audit entries.  Rules haven't fired yet, or filters excluded everything.");
        return Ok(());
    }

    if json {
        for e in &entries {
            println!("{}", serde_json::to_string(e).map_err(|e| e.to_string())?);
        }
        return Ok(());
    }

    // Table view: one line per firing, rule names condensed.
    println!("{:<20} {:<13} {:<8} {:<7} summary", "time", "event", "tool", "rules");
    println!("{}", "─".repeat(80));
    for e in &entries {
        let ts = e.get("ts").and_then(|v| v.as_str()).unwrap_or("");
        let ev = e.get("event").and_then(|v| v.as_str()).unwrap_or("");
        let tool = e.get("tool").and_then(|v| v.as_str()).unwrap_or("");
        let rule_count = e.get("rules").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        let preview = e.get("prompt_preview").and_then(|v| v.as_str()).unwrap_or("");
        let preview_short: String = preview.chars().take(50).collect();
        println!("{:<20} {:<13} {:<8} {:<7} {}", ts, ev, tool, rule_count, preview_short);
    }
    println!("\n  {} firing(s) shown.  Log at {}/audit/{}/", entries.len(), cfg.arai_base_dir.display(), cfg.project_slug());
    Ok(())
}

fn cmd_record(
    since: Option<String>,
    tool: Option<String>,
    limit: usize,
) -> Result<(), String> {
    let cfg = config::Config::load()?;
    let since_epoch = since.as_deref().map(parse_since).transpose()?;
    scenarios::record(&cfg, since_epoch, tool, limit)
}

fn cmd_lint(path: &str, json: bool) -> Result<(), String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {path}: {e}"))?;
    let triples = parser::extract_rules(&content, "lint", 0.90);

    if json {
        let out: Vec<serde_json::Value> = triples
            .iter()
            .map(|t| {
                let intent = intent::classify_rule_with_subject(&t.predicate, &t.object, Some(&t.subject));
                serde_json::json!({
                    "subject": t.subject,
                    "predicate": t.predicate,
                    "object": t.object,
                    "line_start": t.line_start,
                    "line_end": t.line_end,
                    "action": intent.action.as_str(),
                    "timing": format!("{:?}", intent.timing),
                    "tools": intent.tools,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
        return Ok(());
    }

    if triples.is_empty() {
        println!("No rules extracted from {path}.");
        println!("  Try phrasing list items as imperatives: \"- Never force-push to main\"");
        return Ok(());
    }

    println!("Lint: {path}");
    println!("  {} rule(s) extracted\n", triples.len());
    for t in &triples {
        let intent = intent::classify_rule_with_subject(&t.predicate, &t.object, Some(&t.subject));
        let timing = format!("{:?}", intent.timing);
        let line_info = t
            .line_start
            .map(|l| format!(" L{l}"))
            .unwrap_or_default();
        println!(
            "  [{:<7}] {}{}\n    subject:   {}\n    predicate: {}\n    object:    {}\n    action:    {}  timing: {}  tools: {}\n",
            intent.action.as_str(),
            t.source_file,
            line_info,
            t.subject,
            t.predicate,
            t.object,
            intent.action.as_str(),
            timing,
            if intent.tools.is_empty() {
                "<any>".to_string()
            } else {
                intent.tools.join(", ")
            },
        );
    }
    Ok(())
}

fn cmd_trust(add: Option<String>, remove: Option<String>) -> Result<(), String> {
    let cfg = config::Config::load()?;
    match (add, remove) {
        (Some(url), None) => {
            let added = extends::trust_add(&url, &cfg.arai_base_dir)?;
            if added {
                println!("  Trusted: {url}");
            } else {
                println!("  Already trusted: {url}");
            }
        }
        (None, Some(url)) => {
            let removed = extends::trust_remove(&url, &cfg.arai_base_dir)?;
            if removed {
                println!("  Untrusted: {url}");
            } else {
                println!("  Not in trust list: {url}");
            }
        }
        (Some(_), Some(_)) => {
            return Err("Pass --add or --remove, not both".to_string());
        }
        (None, None) => {
            let urls = extends::trust_list(&cfg.arai_base_dir);
            if urls.is_empty() {
                println!("No trusted URLs.  Add one with `arai trust --add https://...`");
            } else {
                println!("Trusted URLs ({}):", urls.len());
                for u in urls {
                    println!("  - {u}");
                }
            }
        }
    }
    Ok(())
}

fn cmd_stats(
    since: Option<String>,
    top: usize,
    by_rule: bool,
    json: bool,
) -> Result<(), String> {
    let cfg = config::Config::load()?;
    let since_epoch = since.as_deref().map(parse_since).transpose()?;
    stats::run(&cfg, since_epoch, top, by_rule, json)
}

/// Return an iterator over every rule object attached to an audit entry,
/// regardless of whether it lives at the top level (Pre/PostToolUse firings)
/// or inside `payload.rules[]` (Compliance events).
fn entry_rules(entry: &serde_json::Value) -> Box<dyn Iterator<Item = &serde_json::Value> + '_> {
    let direct = entry.get("rules").and_then(|v| v.as_array());
    let payload = entry
        .get("payload")
        .and_then(|p| p.get("rules"))
        .and_then(|v| v.as_array());
    match (direct, payload) {
        (Some(d), Some(p)) => Box::new(d.iter().chain(p.iter())),
        (Some(d), None) => Box::new(d.iter()),
        (None, Some(p)) => Box::new(p.iter()),
        (None, None) => Box::new(std::iter::empty()),
    }
}

/// Test a rule object (subject/predicate/object — any may be absent) against
/// a lowercase substring needle.
fn rule_matches_pattern(rule: &serde_json::Value, needle: &str) -> bool {
    let pick = |k: &str| {
        rule.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase()
    };
    pick("subject").contains(needle)
        || pick("predicate").contains(needle)
        || pick("object").contains(needle)
}

/// `arai severity` — list overrides, pin a severity, or reset.
fn cmd_severity(
    pattern: Option<String>,
    level: Option<String>,
    reset: bool,
    json: bool,
) -> Result<(), String> {
    let cfg = config::Config::load()?;
    let db_path = cfg.db_path();
    if !db_path.exists() {
        return Err("No guardrail database found.  Run `arai init` first.".to_string());
    }
    let db = store::Store::open(&db_path)?;

    // No args → list current overrides.
    if pattern.is_none() && level.is_none() && !reset {
        let overrides = db.list_severity_overrides().map_err(|e| e.to_string())?;
        if json {
            println!("{}", serde_json::to_string_pretty(&overrides).map_err(|e| e.to_string())?);
            return Ok(());
        }
        if overrides.is_empty() {
            println!("No severity overrides.  All rules use predicate-derived severity.");
            println!("  Pin one with `arai severity <pattern> block|warn|inform`.");
            return Ok(());
        }
        println!("Active severity overrides ({}):", overrides.len());
        for c in &overrides {
            println!(
                "  [{:>6} \u{2192} {:<6}] {} {}: {}",
                c.from.as_str(),
                c.to.as_str(),
                c.subject,
                c.predicate,
                c.object,
            );
            println!("    from {}", c.source);
        }
        return Ok(());
    }

    let pattern = pattern.ok_or_else(|| {
        "missing pattern.  Usage: `arai severity <pattern> block|warn|inform` or `arai severity --reset <pattern>`".to_string()
    })?;
    if pattern.trim().is_empty() {
        return Err("pattern is empty (would match every rule)".to_string());
    }

    if reset {
        let cleared = db.clear_severity_override(&pattern).map_err(|e| e.to_string())?;
        if json {
            println!("{}", serde_json::to_string_pretty(&cleared).map_err(|e| e.to_string())?);
            return Ok(());
        }
        if cleared.is_empty() {
            println!("No overrides matched `{pattern}` — nothing to clear.");
            return Ok(());
        }
        println!("Cleared {} override(s):", cleared.len());
        for c in &cleared {
            println!(
                "  [{:>6} \u{2192} {:<6}] {} {}: {}",
                c.from.as_str(),
                c.to.as_str(),
                c.subject,
                c.predicate,
                c.object,
            );
        }
        return Ok(());
    }

    let level = level.ok_or_else(|| {
        "missing severity.  Pass `block`, `warn`, or `inform` (or use `--reset` to drop the override)".to_string()
    })?;
    let severity = match level.to_lowercase().as_str() {
        "block" => intent::Severity::Block,
        "warn" => intent::Severity::Warn,
        "inform" => intent::Severity::Inform,
        other => return Err(format!("invalid severity `{other}` (expected block/warn/inform)")),
    };

    let changed = db
        .set_severity_override(&pattern, severity)
        .map_err(|e| e.to_string())?;

    if json {
        println!("{}", serde_json::to_string_pretty(&changed).map_err(|e| e.to_string())?);
        return Ok(());
    }
    if changed.is_empty() {
        println!("No rules matched `{pattern}`.  Run `arai guardrails` to see active rules.");
        return Ok(());
    }
    println!("Pinned severity \u{2192} {} on {} rule(s):", severity.as_str(), changed.len());
    for c in &changed {
        println!(
            "  [{:>6} \u{2192} {:<6}] {} {}: {}",
            c.from.as_str(),
            c.to.as_str(),
            c.subject,
            c.predicate,
            c.object,
        );
        println!("    from {}", c.source);
    }
    Ok(())
}

/// `arai diff <file>` — preview rule-set delta between the candidate file
/// content and the rules currently in the store for that source path.
fn cmd_diff(path: &str, json: bool) -> Result<(), String> {
    let cfg = config::Config::load()?;
    let db_path = cfg.db_path();
    if !db_path.exists() {
        return Err("No guardrail database found.  Run `arai init` first.".to_string());
    }
    let db = store::Store::open(&db_path)?;

    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {path}: {e}"))?;

    // Extract from the candidate content.  Use the same "lint" source-type
    // tag the lint command does so domain bookkeeping stays consistent.
    let candidate = parser::extract_rules(&content, "lint", 0.90);

    // Look up live rules for this exact source path.  If the file has never
    // been scanned before, every rule looks "added", which `arai lint` is
    // already the right tool for — surface that to the user explicitly.
    let live = db.rules_for_file(path).map_err(|e| e.to_string())?;
    let cold_file = live.is_empty() && !candidate.is_empty();
    if cold_file && !json {
        println!("Diff: {path}");
        println!("  (this file has not been scanned yet — every rule reads as added)");
        println!("  Run `arai scan` after saving, or use `arai lint {path}` for a preview.");
        println!();
    }

    // Key by (subject, predicate, object) — the natural identity for a rule
    // independent of which line it lives on.
    use std::collections::BTreeMap;
    let key = |s: &str, p: &str, o: &str| (s.to_lowercase(), p.to_lowercase(), o.to_lowercase());

    let mut live_map: BTreeMap<(String, String, String), &store::Guardrail> = BTreeMap::new();
    for g in &live {
        live_map.insert(key(&g.subject, &g.predicate, &g.object), g);
    }
    let mut cand_map: BTreeMap<(String, String, String), &parser::Triple> = BTreeMap::new();
    for t in &candidate {
        cand_map.insert(key(&t.subject, &t.predicate, &t.object), t);
    }

    let mut added: Vec<&parser::Triple> = Vec::new();
    let mut removed: Vec<&store::Guardrail> = Vec::new();
    let mut moved: Vec<(&store::Guardrail, &parser::Triple)> = Vec::new(); // same SPO, line moved

    for (k, t) in &cand_map {
        match live_map.get(k) {
            Some(g) => {
                let live_line = g.line_start;
                let cand_line = t.line_start;
                if live_line != cand_line {
                    moved.push((g, t));
                }
            }
            None => added.push(t),
        }
    }
    for (k, g) in &live_map {
        if !cand_map.contains_key(k) {
            removed.push(g);
        }
    }

    if json {
        let added_json: Vec<serde_json::Value> = added
            .iter()
            .map(|t| serde_json::json!({
                "subject": t.subject,
                "predicate": t.predicate,
                "object": t.object,
                "line": t.line_start,
                "layer": t.layer,
            }))
            .collect();
        let removed_json: Vec<serde_json::Value> = removed
            .iter()
            .map(|g| serde_json::json!({
                "subject": g.subject,
                "predicate": g.predicate,
                "object": g.object,
                "line": g.line_start,
            }))
            .collect();
        let moved_json: Vec<serde_json::Value> = moved
            .iter()
            .map(|(g, t)| serde_json::json!({
                "subject": g.subject,
                "predicate": g.predicate,
                "object": g.object,
                "from_line": g.line_start,
                "to_line": t.line_start,
            }))
            .collect();
        let out = serde_json::json!({
            "file": path,
            "added": added_json,
            "removed": removed_json,
            "moved": moved_json,
            "summary": {
                "added": added.len(),
                "removed": removed.len(),
                "moved": moved.len(),
            },
        });
        println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
        return Ok(());
    }

    if added.is_empty() && removed.is_empty() && moved.is_empty() {
        println!("Diff: {path}");
        println!("  No rule changes — file content differs only outside rule lines (or not at all).");
        return Ok(());
    }

    println!("Diff: {path}");
    println!(
        "  +{} added   -{} removed   ~{} moved",
        added.len(),
        removed.len(),
        moved.len(),
    );
    println!();
    if !added.is_empty() {
        println!("  Added:");
        for t in &added {
            let line = t.line_start.map(|l| format!(" L{l}")).unwrap_or_default();
            println!("    + {} {}: {}{}", t.subject, t.predicate, t.object, line);
        }
        println!();
    }
    if !removed.is_empty() {
        println!("  Removed:");
        for g in &removed {
            let line = g.line_start.map(|l| format!(" L{l}")).unwrap_or_default();
            println!("    - {} {}: {}{}", g.subject, g.predicate, g.object, line);
        }
        println!();
    }
    if !moved.is_empty() {
        println!("  Moved:");
        for (g, t) in &moved {
            let from = g.line_start.map(|l| l.to_string()).unwrap_or_default();
            let to = t.line_start.map(|l| l.to_string()).unwrap_or_default();
            println!(
                "    ~ {} {}: {}  (L{from} \u{2192} L{to})",
                g.subject, g.predicate, g.object,
            );
        }
        println!();
    }
    Ok(())
}


/// Explain which guardrails would fire on a hypothetical tool call — same
/// matching pipeline the live hook uses, but read-only and no audit write.
fn cmd_why(
    input: Vec<String>,
    tool: String,
    event: String,
    json: bool,
) -> Result<(), String> {
    let cfg = config::Config::load()?;
    let db_path = cfg.db_path();
    if !db_path.exists() {
        return Err(
            "No guardrail database found.  Run `arai init` first.".to_string(),
        );
    }
    let db = store::Store::open(&db_path)?;

    // Build a hook payload mirroring what Claude Code sends.
    let joined = input.join(" ");
    let tool_input = match tool.as_str() {
        "Bash" => serde_json::json!({ "command": joined }),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            serde_json::json!({ "file_path": joined })
        }
        _ => serde_json::json!({ "preview": joined }),
    };
    let hook = serde_json::json!({
        "hook_event_name": event,
        "tool_name": tool,
        "tool_input": tool_input,
        "session_id": "",
    });

    let result = hooks::match_hook(&hook, &cfg, &db)?;

    if json {
        let entries: Vec<serde_json::Value> = result
            .matched
            .iter()
            .map(|(g, pct)| {
                let intent = db.get_rule_intent(g.triple_id).ok().flatten();
                let severity = intent
                    .as_ref()
                    .map(|i| i.severity.as_str().to_string())
                    .unwrap_or_else(|| intent::Severity::from_predicate(&g.predicate).as_str().to_string());
                serde_json::json!({
                    "triple_id": g.triple_id,
                    "subject": g.subject,
                    "predicate": g.predicate,
                    "object": g.object,
                    "source": g.file_path,
                    "line": g.line_start,
                    "layer": g.layer,
                    "layer_label": g.layer.map(audit::layer_label),
                    "match_pct": pct,
                    "severity": severity,
                })
            })
            .collect();
        let out = serde_json::json!({
            "tool": result.tool_name,
            "event": result.event,
            "terms": result.terms,
            "matched": entries,
        });
        println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
        return Ok(());
    }

    println!("  tool:   {}", result.tool_name);
    println!("  event:  {}", result.event);
    println!("  terms:  {}", result.terms.join(", "));
    if result.skipped {
        println!("  status: skipped (tool is on the bypass list)");
        return Ok(());
    }
    if result.matched.is_empty() {
        println!("  matched: 0 rules");
        return Ok(());
    }
    println!("  matched: {} rule(s)", result.matched.len());
    println!();
    for (g, pct) in &result.matched {
        let intent = db.get_rule_intent(g.triple_id).ok().flatten();
        let severity = intent
            .as_ref()
            .map(|i| i.severity.as_str())
            .unwrap_or_else(|| {
                // Fallback string lifetime: compute once before the match.
                match intent::Severity::from_predicate(&g.predicate) {
                    intent::Severity::Block => "block",
                    intent::Severity::Warn => "warn",
                    intent::Severity::Inform => "inform",
                }
            });
        let src = if g.file_path.is_empty() {
            &g.source_file
        } else {
            &g.file_path
        };
        let line_suffix = g
            .line_start
            .map(|l| format!(":{l}"))
            .unwrap_or_default();
        let layer_suffix = g
            .layer
            .map(|l| format!("  [{}]", audit::layer_label(l)))
            .unwrap_or_default();
        println!(
            "  • [{sev:6}] {subj} {pred}: {obj}  ({pct}% match){layer_suffix}",
            sev = severity,
            subj = g.subject,
            pred = g.predicate,
            obj = g.object,
        );
        println!("       from {src}{line_suffix}");
    }
    Ok(())
}

/// Parse a duration like "7d", "24h", "30m", "3600s" into an epoch-seconds
/// cutoff (now - duration).  Plain digits are treated as seconds.
fn parse_since(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("--since cannot be empty".to_string());
    }
    let (num_part, unit_secs): (&str, u64) = match s.chars().last().unwrap() {
        'd' => (&s[..s.len() - 1], 86_400),
        'h' => (&s[..s.len() - 1], 3_600),
        'm' => (&s[..s.len() - 1], 60),
        's' => (&s[..s.len() - 1], 1),
        c if c.is_ascii_digit() => (s, 1),
        other => return Err(format!("--since: unknown unit '{other}' (use d/h/m/s)")),
    };
    let n: u64 = num_part
        .parse()
        .map_err(|_| format!("--since: invalid number '{num_part}'"))?;
    let delta = n.checked_mul(unit_secs).ok_or("--since: overflow")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(now.saturating_sub(delta))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn entry_rules_walks_top_level() {
        let e = json!({"rules": [{"subject": "git", "predicate": "never", "object": "force"}]});
        let collected: Vec<_> = entry_rules(&e).collect();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0]["subject"], "git");
    }

    #[test]
    fn entry_rules_walks_compliance_payload() {
        let e = json!({
            "event": "Compliance",
            "payload": {"rules": [{"triple_id": 7, "predicate": "never", "object": "force"}]}
        });
        let collected: Vec<_> = entry_rules(&e).collect();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0]["triple_id"], 7);
    }

    #[test]
    fn entry_rules_walks_both_when_present() {
        // Defensive: nothing in the audit format combines both today, but the
        // helper should still surface every rule it can find.
        let e = json!({
            "rules": [{"subject": "a", "predicate": "p", "object": "o"}],
            "payload": {"rules": [{"triple_id": 1, "predicate": "p", "object": "o"}]},
        });
        let collected: Vec<_> = entry_rules(&e).collect();
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn entry_rules_empty_when_neither_present() {
        let e = json!({"event": "PreToolUse", "tool": "Bash"});
        let collected: Vec<_> = entry_rules(&e).collect();
        assert!(collected.is_empty());
    }

    #[test]
    fn rule_matches_pattern_substring_subject() {
        let r = json!({"subject": "alembic", "predicate": "never", "object": "hand-write"});
        assert!(rule_matches_pattern(&r, "alem"));
        assert!(rule_matches_pattern(&r, "lemb")); // mid-substring
        assert!(!rule_matches_pattern(&r, "django"));
    }

    #[test]
    fn rule_matches_pattern_substring_object_and_predicate() {
        let r = json!({"subject": "git", "predicate": "must_not", "object": "force-push"});
        assert!(rule_matches_pattern(&r, "force"));
        assert!(rule_matches_pattern(&r, "must_not"));
    }

    #[test]
    fn rule_matches_pattern_handles_missing_fields() {
        // A Compliance rule object only has predicate/object/triple_id — no
        // subject.  The matcher must still handle it without panicking.
        let r = json!({"triple_id": 1, "predicate": "never", "object": "x"});
        assert!(rule_matches_pattern(&r, "never"));
        assert!(!rule_matches_pattern(&r, "git"));
    }
}

