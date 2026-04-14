use crate::{code_scanner, config, discovery, parser, store};
use serde_json::Value;

pub fn run() -> Result<(), String> {
    let cfg = config::Config::load()?;

    println!("  Scanning for instruction files...");
    let files = discovery::discover(&cfg)?;

    if files.is_empty() {
        println!("  No instruction files found.");
        return Ok(());
    }

    for f in &files {
        let line_count = f.content.lines().count();
        println!("    \u{2713} {} ({line_count} lines, {})", display_path(&f.path, &cfg), f.source_type);
    }

    println!("\n  Extracting guardrails...");
    let db = store::Store::open(&cfg.db_path())?;

    let mut total_rules = 0;
    for file in &files {
        let triples = parser::extract_rules(&file.content, &file.source_type, file.confidence);
        let count = triples.len();
        db.upsert_file(&file.path, &file.content, &triples, &file.source_type)
            .map_err(|e| e.to_string())?;
        total_rules += count;
    }
    println!("    \u{2713} {total_rules} rules extracted");

    let classified = db.classify_all_guardrails().map_err(|e| e.to_string())?;
    println!("    \u{2713} {classified} rules classified by intent");

    // Auto-enrich if the model is already downloaded
    let model_dir = cfg.arai_base_dir.join("models").join("all-MiniLM-L6-v2");
    if model_dir.join("model.onnx").exists() {
        match crate::enrich::enrich_guardrails(&db, &cfg.arai_base_dir) {
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
    println!("    \u{2713} {import_count} imports from {file_count} files, {tool_count} unique tools");

    println!("\n  Setting up hooks...");
    inject_hooks(&cfg)?;
    println!("    \u{2713} .claude/settings.json updated");

    println!("\n  Done. Arai is now watching your rules.");
    Ok(())
}

fn inject_hooks(cfg: &config::Config) -> Result<(), String> {
    let settings_path = cfg.claude_settings_path();

    // Ensure .claude directory exists
    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create .claude directory: {e}"))?;
    }

    // Read existing settings or start fresh
    let mut settings: Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)
            .map_err(|e| format!("Failed to read settings.json: {e}"))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse settings.json: {e}"))?
    } else {
        serde_json::json!({})
    };

    let hooks = settings
        .as_object_mut()
        .ok_or("settings.json is not an object")?
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));

    let hooks_obj = hooks
        .as_object_mut()
        .ok_or("hooks is not an object")?;

    // Inject arai hook into relevant hook events
    for event in &["PreToolUse", "PostToolUse", "UserPromptSubmit"] {
        let event_arr = hooks_obj
            .entry(*event)
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .ok_or(format!("{event} is not an array"))?;

        let arai_exists = event_arr.iter().any(|entry| {
            if let Some(hooks_arr) = entry.get("hooks").and_then(|v| v.as_array()) {
                hooks_arr.iter().any(|h| {
                    h.get("command")
                        .and_then(|v| v.as_str())
                        .map(|cmd| cmd.contains("arai"))
                        .unwrap_or(false)
                })
            } else {
                false
            }
        });

        if !arai_exists {
            let arai_hook = serde_json::json!({
                "matcher": "",
                "hooks": [
                    {
                        "type": "command",
                        "command": "arai guardrails --match-stdin",
                        "timeout": 3
                    }
                ]
            });
            event_arr.push(arai_hook);
        }
    }

    // Write back
    let output = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("Failed to serialize settings: {e}"))?;
    std::fs::write(&settings_path, output)
        .map_err(|e| format!("Failed to write settings.json: {e}"))?;

    Ok(())
}

/// Display a path relative to project root when possible.
fn display_path(path: &str, cfg: &config::Config) -> String {
    let root = cfg.project_root.to_string_lossy();
    if path.starts_with(root.as_ref()) {
        path[root.len()..].trim_start_matches('/').to_string()
    } else if path.starts_with(&cfg.home_dir.to_string_lossy().as_ref()) {
        format!("~{}", &path[cfg.home_dir.to_string_lossy().len()..])
    } else {
        path.to_string()
    }
}
