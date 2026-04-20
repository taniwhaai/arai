mod audit;
mod code_scanner;
mod config;
mod discovery;
mod enrich;
mod guardrails;
mod hooks;
mod init;
mod intent;
mod mcp;
mod parser;
mod session;
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
        /// Filter by hook event (PreToolUse, PostToolUse, UserPromptSubmit)
        #[arg(long)]
        event: Option<String>,
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
        Commands::Audit { since, tool, event, limit, json } => cmd_audit(since, tool, event, limit, json),
        Commands::Mcp => mcp::run(),
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
    limit: usize,
    json: bool,
) -> Result<(), String> {
    let cfg = config::Config::load()?;
    let since_epoch = since.as_deref().map(parse_since).transpose()?;
    let entries = audit::query(
        &cfg.arai_base_dir,
        &cfg.project_slug(),
        since_epoch,
        tool.as_deref(),
        event.as_deref(),
        limit,
    )?;

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
