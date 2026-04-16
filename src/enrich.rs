//! Tier 2 intent enrichment using a sentence transformer model.
//!
//! Downloads all-MiniLM-L6-v2 on first use, embeds rule text,
//! and classifies intent by cosine similarity to archetype sentences.

use crate::intent::{Action, RuleIntent};
use crate::store::Store;
use std::io::Read as _;
use std::path::Path;
#[cfg(feature = "enrich")]
use std::path::PathBuf;

#[cfg(feature = "enrich")]
const MODEL_DIR_NAME: &str = "models";
#[cfg(feature = "enrich")]
const MODEL_NAME: &str = "all-MiniLM-L6-v2";

#[cfg(feature = "enrich")]
const MODEL_URL: &str = "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx/model.onnx";
#[cfg(feature = "enrich")]
const TOKENIZER_URL: &str = "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json";

#[cfg(feature = "enrich")]
/// Archetype sentences for each intent category.
/// Multiple per category for robustness.
const CREATE_ARCHETYPES: &[&str] = &[
    "creating a new file from scratch",
    "writing a brand new file manually",
    "hand-writing a new source file",
    "generating a new file by hand",
    "manually authoring a new document",
    "scaffolding a new component from scratch",
    "adding a new file to the project",
];

#[cfg(feature = "enrich")]
const MODIFY_ARCHETYPES: &[&str] = &[
    "modifying an existing file",
    "editing the contents of a file",
    "changing code in an existing source file",
    "patching an existing document",
    "updating the implementation in a file",
    "refactoring existing code",
    "tweaking the configuration of an existing file",
];

#[cfg(feature = "enrich")]
const EXECUTE_ARCHETYPES: &[&str] = &[
    "running a command in the terminal",
    "executing a CLI tool with specific flags",
    "invoking a build or test command",
    "using a command-line tool to generate output",
    "launching a process from the shell",
    "calling a tool via the command line",
    "running a script with arguments",
];

#[cfg(feature = "enrich")]
const GENERAL_ARCHETYPES: &[&str] = &[
    "a general coding best practice",
    "a workflow guideline that applies broadly",
    "a rule about code quality or style",
    "a policy about version control",
    "a general development principle",
];

/// Run enrichment on all guardrails using the sentence transformer model.
#[cfg(feature = "enrich")]
pub fn enrich_guardrails(store: &Store, arai_base_dir: &Path) -> Result<usize, String> {
    let model_dir = ensure_model_downloaded(arai_base_dir)?;
    let model_path = model_dir.join("model.onnx");
    let tokenizer_path = model_dir.join("tokenizer.json");

    println!("    Loading model...");

    // Initialize ONNX Runtime
    let mut session = ort::session::Session::builder()
        .map_err(|e| format!("Failed to create ONNX session builder: {e}"))?
        .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
        .map_err(|e| format!("Failed to set optimization level: {e}"))?
        .commit_from_file(&model_path)
        .map_err(|e| format!("Failed to load model: {e}"))?;

    // Load tokenizer
    let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| format!("Failed to load tokenizer: {e}"))?;

    // Pre-compute archetype embeddings
    println!("    Computing archetype embeddings...");
    let create_embs = embed_sentences(&mut session, &tokenizer, CREATE_ARCHETYPES)?;
    let modify_embs = embed_sentences(&mut session, &tokenizer, MODIFY_ARCHETYPES)?;
    let execute_embs = embed_sentences(&mut session, &tokenizer, EXECUTE_ARCHETYPES)?;
    let general_embs = embed_sentences(&mut session, &tokenizer, GENERAL_ARCHETYPES)?;

    let category_embeddings = [
        (Action::Create, &create_embs),
        (Action::Modify, &modify_embs),
        (Action::Execute, &execute_embs),
        (Action::General, &general_embs),
    ];

    // Classify each guardrail
    let guardrails = store.load_guardrails().map_err(|e| e.to_string())?;
    let mut count = 0;

    for g in &guardrails {
        let rule_text = format!("{} {}: {}", g.subject, g.predicate, g.object);
        let rule_emb = embed_single(&mut session, &tokenizer, &rule_text)?;

        // Find best matching category
        let mut best_action = Action::General;
        let mut best_similarity = f32::NEG_INFINITY;

        for (action, embs) in &category_embeddings {
            let avg_sim = average_similarity(&rule_emb, embs);
            if avg_sim > best_similarity {
                best_similarity = avg_sim;
                best_action = action.clone();
            }
        }

        // Only override taxonomy if model is confident (similarity > 0.3)
        if best_similarity > 0.3 {
            let is_prohibition = matches!(g.predicate.as_str(), "never" | "forbids" | "must_not");
            let allow_inverse = is_prohibition && best_action == Action::Create;

            let tools = match best_action {
                Action::Create => vec!["Write".to_string(), "NotebookEdit".to_string()],
                Action::Modify => vec!["Edit".to_string()],
                Action::Execute => vec!["Bash".to_string()],
                Action::General => vec!["*".to_string()],
            };

            // Use taxonomy for timing classification (model doesn't handle this)
            let timing = crate::intent::classify_rule_with_subject(&g.predicate, &g.object, Some(&g.subject)).timing;

            let intent = RuleIntent {
                action: best_action,
                timing,
                tools,
                allow_inverse,
                enriched_by: "model".to_string(),
            };

            store.upsert_rule_intent(g.triple_id, &intent)
                .map_err(|e| e.to_string())?;
            count += 1;
        }
    }

    Ok(count)
}

#[cfg(not(feature = "enrich"))]
pub fn enrich_guardrails(_store: &Store, _arai_base_dir: &Path) -> Result<usize, String> {
    // Lean binary — offer to upgrade
    match crate::upgrade::offer_upgrade_to_full() {
        Ok(true) => {
            // Upgraded successfully, user needs to re-run
            std::process::exit(0);
        }
        Ok(false) => {
            // User declined
            Err("Enrichment skipped. Run `arai upgrade --full` when ready.".to_string())
        }
        Err(e) => Err(e),
    }
}

/// Ensure the model is downloaded to ~/.arai/models/all-MiniLM-L6-v2/
#[cfg(feature = "enrich")]
fn ensure_model_downloaded(arai_base_dir: &Path) -> Result<PathBuf, String> {
    let model_dir = arai_base_dir.join(MODEL_DIR_NAME).join(MODEL_NAME);
    let model_path = model_dir.join("model.onnx");
    let tokenizer_path = model_dir.join("tokenizer.json");

    if model_path.exists() && tokenizer_path.exists() {
        return Ok(model_dir);
    }

    std::fs::create_dir_all(&model_dir)
        .map_err(|e| format!("Failed to create model directory: {e}"))?;

    if !model_path.exists() {
        println!("    Downloading model ({MODEL_NAME})...");
        download_file(MODEL_URL, &model_path)?;
    }

    if !tokenizer_path.exists() {
        println!("    Downloading tokenizer...");
        download_file(TOKENIZER_URL, &tokenizer_path)?;
    }

    Ok(model_dir)
}

#[cfg(feature = "enrich")]
fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    let output = std::process::Command::new("curl")
        .args(["-sL", "-o", &dest.to_string_lossy(), url])
        .output()
        .map_err(|e| format!("Failed to run curl: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Download failed: {stderr}"));
    }

    // Verify file was created and has content
    let meta = std::fs::metadata(dest)
        .map_err(|e| format!("Downloaded file not found: {e}"))?;
    if meta.len() < 1000 {
        return Err(format!("Downloaded file is suspiciously small ({} bytes)", meta.len()));
    }

    Ok(())
}

/// Embed multiple sentences, returning a vec of embedding vectors.
#[cfg(feature = "enrich")]
fn embed_sentences(
    session: &mut ort::session::Session,
    tokenizer: &tokenizers::Tokenizer,
    sentences: &[&str],
) -> Result<Vec<Vec<f32>>, String> {
    sentences
        .iter()
        .map(|s| embed_single(session, tokenizer, s))
        .collect()
}

/// Embed a single sentence using the ONNX model.
#[cfg(feature = "enrich")]
fn embed_single(
    session: &mut ort::session::Session,
    tokenizer: &tokenizers::Tokenizer,
    text: &str,
) -> Result<Vec<f32>, String> {
    let encoding = tokenizer.encode(text, true)
        .map_err(|e| format!("Tokenization failed: {e}"))?;

    let ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
    let mask: Vec<i64> = encoding.get_attention_mask().iter().map(|&m| m as i64).collect();
    let type_ids: Vec<i64> = encoding.get_type_ids().iter().map(|&t| t as i64).collect();
    let len = ids.len();

    let ids_tensor = ort::value::Tensor::from_array(([1, len], ids))
        .map_err(|e| format!("Failed to create input_ids tensor: {e}"))?;
    let mask_tensor = ort::value::Tensor::from_array(([1, len], mask))
        .map_err(|e| format!("Failed to create attention_mask tensor: {e}"))?;
    let type_tensor = ort::value::Tensor::from_array(([1, len], type_ids))
        .map_err(|e| format!("Failed to create token_type_ids tensor: {e}"))?;

    let outputs = session
        .run(ort::inputs![ids_tensor, mask_tensor, type_tensor])
        .map_err(|e| format!("Inference failed: {e}"))?;

    // Get the first output (last_hidden_state) — shape [1, seq_len, 384]
    let output = outputs.values().next()
        .ok_or("No output from model")?;

    let (shape, raw_data) = output
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("Failed to extract tensor: {e}"))?;

    // Mean pooling: average across sequence length dimension
    let hidden_size = *shape.last().unwrap_or(&384) as usize;
    let seq_len = if shape.len() >= 2 { shape[shape.len() - 2] as usize } else { 1 };

    let mut embedding = vec![0.0f32; hidden_size];
    for i in 0..seq_len {
        for j in 0..hidden_size {
            embedding[j] += raw_data[i * hidden_size + j];
        }
    }
    for val in &mut embedding {
        *val /= seq_len as f32;
    }

    // L2 normalize
    let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for val in &mut embedding {
            *val /= norm;
        }
    }

    Ok(embedding)
}

#[cfg(feature = "enrich")]
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[cfg(feature = "enrich")]
fn average_similarity(query: &[f32], candidates: &[Vec<f32>]) -> f32 {
    if candidates.is_empty() {
        return 0.0;
    }
    let total: f32 = candidates.iter().map(|c| cosine_similarity(query, c)).sum();
    total / candidates.len() as f32
}

// ---------------------------------------------------------------------------
// Tier 3: LLM enrichment via `claude -p`
// ---------------------------------------------------------------------------

/// Build the enrichment prompt for a set of guardrails.
/// Shared by both LLM shell-out and API paths.
fn build_enrichment_prompt(guardrails: &[crate::store::Guardrail]) -> String {
    let mut rules_text = String::new();
    for (i, g) in guardrails.iter().enumerate() {
        rules_text.push_str(&format!(
            "{}. [id:{}] {} {}: {}\n",
            i + 1, g.triple_id, g.subject, g.predicate, g.object
        ));
    }

    format!(
r#"You are classifying guardrail rules for a CLI tool called Arai. For each rule, determine:

1. **action**: What type of action does this rule govern?
   - "create" — about creating NEW files (Write tool)
   - "modify" — about editing EXISTING files (Edit tool)
   - "execute" — about running commands/tools (Bash tool)
   - "general" — broadly applicable, no specific action

2. **timing**: When should this rule fire?
   - "tool_call" — on specific tool calls when the subject matches (domain-specific rules)
   - "principle" — general principle, not tied to a specific tool call

3. **prerequisite**: If the rule says "don't do X without doing Y first", what is Y?
   Extract the prerequisite action as a short phrase, or null if none.

4. **tools**: Which Claude Code tools should this rule match against?
   - ["Write", "NotebookEdit"] for create rules
   - ["Edit"] for modify rules
   - ["Bash"] for execute rules
   - ["*"] for general rules

5. **allow_inverse**: If this is a prohibition on creating files (predicate is never/forbids), should editing existing files be allowed? (true/false)

Rules to classify:
{rules_text}
Respond with ONLY a JSON array, no markdown, no explanation:
[
  {{"id": 1, "action": "create", "timing": "tool_call", "tools": ["Write"], "prerequisite": null, "allow_inverse": true}},
  ...
]"#
    )
}

/// Parse an LLM/API response string and apply classifications.
/// Shared by LLM shell-out, API, and file import paths.
fn parse_and_apply(
    store: &Store,
    guardrails: &[crate::store::Guardrail],
    response: &str,
    arai_base_dir: &Path,
) -> Result<usize, String> {
    let json_str = match extract_json_array(response) {
        Some(s) => s,
        None => {
            save_failed_response(arai_base_dir, response);
            let preview = if response.len() > 500 { &response[..500] } else { response };
            return Err(format!(
                "Could not parse LLM response as JSON array.\n\
                 Raw output (first 500 chars):\n{preview}\n\n\
                 Full response saved to: {}/last-enrich-response.json",
                arai_base_dir.display()
            ));
        }
    };

    let classifications: Vec<serde_json::Value> = match serde_json::from_str(&json_str) {
        Ok(c) => c,
        Err(e) => {
            save_failed_response(arai_base_dir, response);
            return Err(format!(
                "Failed to parse JSON: {e}\n\
                 Full response saved to: {}/last-enrich-response.json",
                arai_base_dir.display()
            ));
        }
    };

    apply_classifications(store, guardrails, &classifications, arai_base_dir)
}

/// Enrich guardrails by shelling out to an LLM CLI.
/// Uses the configured command (ARAI_LLM_CMD env var or config.toml).
/// Falls back to `claude -p` if nothing configured.
pub fn enrich_via_llm(store: &Store, llm_command: Option<&str>, arai_base_dir: &Path) -> Result<usize, String> {
    let cmd = llm_command
        .map(String::from)
        .or_else(detect_llm_command)
        .ok_or(
            "No LLM command configured. Set one of:\n\
             \n  ARAI_LLM_CMD=\"claude -p\"                # Claude Code\
             \n  ARAI_LLM_CMD=\"ollama run llama3\"         # Ollama\
             \n  ARAI_LLM_CMD=\"llm -m gpt-4o\"             # Simon Willison's llm\
             \n\nOr add to ~/.arai/config.toml:\n\
             \n  [enrich]\
             \n  llm_command = \"claude -p\""
        )?;

    let guardrails = store.load_guardrails().map_err(|e| e.to_string())?;
    if guardrails.is_empty() {
        return Ok(0);
    }

    let prompt = build_enrichment_prompt(&guardrails);
    println!("    Sending {} rules to LLM ({})...", guardrails.len(), cmd);

    let response = run_llm_command(&cmd, &prompt)?;
    parse_and_apply(store, &guardrails, &response, arai_base_dir)
}

/// Save failed LLM response for debugging.
fn save_failed_response(arai_base_dir: &Path, response: &str) {
    let path = arai_base_dir.join("last-enrich-response.json");
    std::fs::write(&path, response).ok();
}

// ---------------------------------------------------------------------------
// Tier 3b: Direct API enrichment (OpenAI-compatible endpoints)
// ---------------------------------------------------------------------------

/// Resolved API configuration.
#[derive(Debug)]
struct ApiConfig {
    url: String,
    api_key: Option<String>,
    model: String,
}

/// Enrich guardrails via direct HTTP API call to an OpenAI-compatible endpoint.
pub fn enrich_via_api(
    store: &Store,
    api_url: Option<&str>,
    api_key_env: Option<&str>,
    api_model: Option<&str>,
    arai_base_dir: &Path,
) -> Result<usize, String> {
    let config = resolve_api_config(api_url, api_key_env, api_model)?;

    let guardrails = store.load_guardrails().map_err(|e| e.to_string())?;
    if guardrails.is_empty() {
        return Ok(0);
    }

    let prompt = build_enrichment_prompt(&guardrails);
    println!("    Sending {} rules to {} (model: {})...", guardrails.len(), config.url, config.model);

    let response = call_chat_completions(&config, &prompt)?;
    parse_and_apply(store, &guardrails, &response, arai_base_dir)
}

/// Resolve API configuration from env vars, config, and auto-detection.
fn resolve_api_config(
    api_url: Option<&str>,
    api_key_env: Option<&str>,
    api_model: Option<&str>,
) -> Result<ApiConfig, String> {
    // Resolve API key from the named env var
    let api_key = api_key_env.and_then(|env_name| std::env::var(env_name).ok());

    // 1. Explicit URL provided
    if let Some(url) = api_url {
        let model = api_model.unwrap_or("gpt-4o-mini").to_string();
        return Ok(ApiConfig {
            url: ensure_completions_path(url),
            api_key,
            model,
        });
    }

    // 2. Auto-detect Ollama at localhost
    if probe_ollama() {
        return Ok(ApiConfig {
            url: "http://localhost:11434/v1/chat/completions".to_string(),
            api_key: None,
            model: api_model.unwrap_or("llama3.1").to_string(),
        });
    }

    // 3. API key set but no URL → default to OpenAI
    if api_key.is_some() {
        return Ok(ApiConfig {
            url: "https://api.openai.com/v1/chat/completions".to_string(),
            api_key,
            model: api_model.unwrap_or("gpt-4o-mini").to_string(),
        });
    }

    Err("No API endpoint configured. Set one of:\n\
         \n  ARAI_API_KEY=sk-...                        # Uses OpenAI by default\
         \n  ARAI_API_URL=http://localhost:11434/v1      # Ollama (no key needed)\
         \n\nOr add to ~/.arai/config.toml:\n\
         \n  [enrich]\
         \n  api_url = \"https://api.openai.com/v1\"\
         \n  api_key_env = \"OPENAI_API_KEY\"\
         \n  model = \"gpt-4o-mini\"".to_string())
}

/// Ensure the URL ends with /chat/completions.
fn ensure_completions_path(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/v1/chat/completions")
    }
}

/// Probe for a local Ollama instance at localhost:11434.
fn probe_ollama() -> bool {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(1)))
        .build()
        .new_agent();
    match agent.get("http://localhost:11434/").call() {
        Ok(response) => {
            let mut body = String::new();
            response.into_body().as_reader().read_to_string(&mut body).ok();
            body.contains("Ollama")
        }
        Err(_) => false,
    }
}

/// Call an OpenAI-compatible chat completions endpoint.
fn call_chat_completions(config: &ApiConfig, prompt: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": config.model,
        "messages": [
            {
                "role": "system",
                "content": "You are a classification engine. Respond only with valid JSON arrays."
            },
            {
                "role": "user",
                "content": prompt
            }
        ],
        "temperature": 0.0
    });

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(120)))
        .build()
        .new_agent();

    let mut request = agent.post(&config.url)
        .header("Content-Type", "application/json");

    if let Some(ref key) = config.api_key {
        request = request.header("Authorization", &format!("Bearer {key}"));
    }

    let response = request
        .send_json(&body)
        .map_err(|e| {
            match &e {
                ureq::Error::StatusCode(401) =>
                    "Authentication failed. Check your ARAI_API_KEY.".to_string(),
                ureq::Error::StatusCode(429) =>
                    "Rate limited by API. Try again in a moment.".to_string(),
                ureq::Error::StatusCode(404) =>
                    format!("Endpoint not found: {}. Check your ARAI_API_URL.", config.url),
                ureq::Error::StatusCode(code) =>
                    format!("API returned HTTP {code}"),
                _ => format!("API request failed: {e}"),
            }
        })?;

    let mut response_str = String::new();
    response.into_body().as_reader().read_to_string(&mut response_str)
        .map_err(|e| format!("Failed to read API response: {e}"))?;

    let response_body: serde_json::Value = serde_json::from_str(&response_str)
        .map_err(|e| format!("Failed to parse API response as JSON: {e}"))?;

    // Extract content from OpenAI chat completions response
    response_body
        .get("choices")
        .and_then(|c: &serde_json::Value| c.get(0))
        .and_then(|c: &serde_json::Value| c.get("message"))
        .and_then(|m: &serde_json::Value| m.get("content"))
        .and_then(|c: &serde_json::Value| c.as_str())
        .map(String::from)
        .ok_or_else(|| {
            format!("Unexpected API response structure: {}",
                serde_json::to_string_pretty(&response_body).unwrap_or_default())
        })
}

/// Import enrichment from a JSON file (same format as LLM output).
pub fn enrich_from_file(store: &Store, path: &str, arai_base_dir: &Path) -> Result<usize, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {path}: {e}"))?;

    let guardrails = store.load_guardrails().map_err(|e| e.to_string())?;
    parse_and_apply(store, &guardrails, &content, arai_base_dir)
}

/// Apply parsed classifications to guardrails with validation + fuzzy matching.
fn apply_classifications(
    store: &Store,
    guardrails: &[crate::store::Guardrail],
    classifications: &[serde_json::Value],
    _arai_base_dir: &Path,
) -> Result<usize, String> {
    let mut count = 0;
    let mut partial_count = 0;

    for entry in classifications {
        let id = entry.get("id").and_then(|v| v.as_i64()).unwrap_or(-1);
        if id < 0 {
            continue;
        }

        let guardrail = guardrails.iter().find(|g| g.triple_id == id);
        if guardrail.is_none() {
            continue;
        }
        let g = guardrail.unwrap();

        let raw_action = entry.get("action").and_then(|v| v.as_str()).unwrap_or("general");
        let raw_timing = entry.get("timing").and_then(|v| v.as_str()).unwrap_or("principle");
        let allow_inverse = entry.get("allow_inverse").and_then(|v| v.as_bool()).unwrap_or(false);

        let tools: Vec<String> = entry
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_else(|| vec!["*".to_string()]);

        // Validate + fuzzy match each enum field
        let mut is_partial = false;

        let action = if Action::is_valid(raw_action) {
            Action::from_str(raw_action)
        } else {
            eprintln!("    \u{26a0} Rule '{}': unrecognized action '{}', using fuzzy match", g.subject, raw_action);
            is_partial = true;
            fuzzy_match_action(raw_action, g)
        };

        let timing_raw = if crate::intent::Timing::is_valid(raw_timing) {
            crate::intent::Timing::from_str(raw_timing)
        } else {
            eprintln!("    \u{26a0} Rule '{}': unrecognized timing '{}', using fuzzy match", g.subject, raw_timing);
            is_partial = true;
            fuzzy_match_timing(raw_timing, g)
        };

        // Validate tool_call timing requires a known tool reference
        let mut timing = timing_raw;
        if timing == crate::intent::Timing::ToolCall {
            let full_text = format!("{} {}", g.subject.to_lowercase(), g.object.to_lowercase());
            let has_tool = crate::parser::KNOWN_TOOLS.iter().any(|tool| {
                full_text.split(|c: char| !c.is_alphanumeric()).any(|w| w == *tool)
            });
            if !has_tool {
                timing = crate::intent::Timing::Principle;
            }
        }

        if is_partial {
            partial_count += 1;
        }

        let intent = RuleIntent {
            action,
            timing,
            tools,
            allow_inverse,
            enriched_by: if is_partial { "llm-partial".to_string() } else { "llm".to_string() },
        };

        store.upsert_rule_intent(id, &intent).map_err(|e| e.to_string())?;
        count += 1;
    }

    if partial_count > 0 {
        eprintln!("    {partial_count} rule(s) had unrecognized values — used fuzzy match/taxonomy fallback");
    }

    Ok(count)
}

/// Fuzzy match an unrecognized action value.
/// Tries simple prefix/substring matching, then falls back to taxonomy.
fn fuzzy_match_action(raw: &str, g: &crate::store::Guardrail) -> Action {
    let lower = raw.to_lowercase();

    // Common alternatives models might produce
    if lower.contains("creat") || lower.contains("writ") || lower.contains("generat") {
        return Action::Create;
    }
    if lower.contains("modif") || lower.contains("edit") || lower.contains("updat") || lower.contains("chang") {
        return Action::Modify;
    }
    if lower.contains("exec") || lower.contains("run") || lower.contains("command") || lower.contains("invoke") {
        return Action::Execute;
    }
    if lower.contains("forbid") || lower.contains("prevent") || lower.contains("block") || lower.contains("deny") {
        // "forbid" is about the predicate, not the action — fall back to taxonomy
        return crate::intent::classify_rule_with_subject(&g.predicate, &g.object, Some(&g.subject)).action;
    }

    // Fall back to taxonomy
    crate::intent::classify_rule_with_subject(&g.predicate, &g.object, Some(&g.subject)).action
}

/// Fuzzy match an unrecognized timing value.
/// Tries simple prefix/substring matching, then falls back to taxonomy.
fn fuzzy_match_timing(raw: &str, g: &crate::store::Guardrail) -> crate::intent::Timing {
    let lower = raw.to_lowercase();

    if lower.contains("tool") || lower.contains("pre_tool") || lower.contains("pretool") {
        return crate::intent::Timing::ToolCall;
    }
    if lower.contains("commit") || lower.contains("pre-commit") || lower.contains("precommit") {
        return crate::intent::Timing::ToolCall;
    }
    if lower.contains("start") || lower.contains("begin") || lower.contains("prompt") {
        return crate::intent::Timing::Start;
    }
    if lower.contains("stop") || lower.contains("end") || lower.contains("finish") || lower.contains("complet") {
        return crate::intent::Timing::Stop;
    }
    if lower.contains("princip") || lower.contains("general") || lower.contains("always") {
        return crate::intent::Timing::Principle;
    }

    // Fall back to taxonomy
    crate::intent::classify_rule_with_subject(&g.predicate, &g.object, Some(&g.subject)).timing
}

/// Auto-detect which LLM CLI is available.
fn detect_llm_command() -> Option<String> {
    let candidates = [
        ("claude", "claude -p"),
        ("ollama", "ollama run llama3.1"),
        ("llm", "llm -m gpt-4o-mini"),
    ];

    for (binary, full_cmd) in &candidates {
        if std::process::Command::new("which")
            .arg(binary)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some(full_cmd.to_string());
        }
    }

    None
}

/// Run an LLM command with the given prompt and return the output text.
fn run_llm_command(cmd: &str, prompt: &str) -> Result<String, String> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return Err("Empty LLM command".to_string());
    }

    let binary = parts[0];
    let args = &parts[1..];

    // Try passing prompt as the last argument first
    let output = std::process::Command::new(binary)
        .args(args)
        .arg(prompt)
        .output()
        .map_err(|e| format!("Failed to run '{binary}': {e}"))?;

    if !output.status.success() {
        // Some CLIs want prompt on stdin instead
        let output = std::process::Command::new(binary)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(ref mut stdin) = child.stdin {
                    stdin.write_all(prompt.as_bytes()).ok();
                }
                child.wait_with_output()
            })
            .map_err(|e| format!("Failed to run '{binary}' with stdin: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("LLM command failed: {stderr}"));
        }

        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Extract a JSON array from text that might contain markdown wrapping.
fn extract_json_array(text: &str) -> Option<String> {
    // Try direct parse first
    if text.trim().starts_with('[') {
        return Some(text.trim().to_string());
    }

    // Look for ```json ... ``` blocks
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            return Some(after[..end].trim().to_string());
        }
    }

    // Look for ``` ... ``` blocks
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        if let Some(end) = after.find("```") {
            let content = after[..end].trim();
            if content.starts_with('[') {
                return Some(content.to_string());
            }
        }
    }

    // Look for first [ to last ]
    if let Some(start) = text.find('[') {
        if let Some(end) = text.rfind(']') {
            return Some(text[start..=end].to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Fuzzy action matching ---

    #[test]
    fn test_fuzzy_action_forbid() {
        // Qwen 2.5 reported this exact value
        let g = crate::store::Guardrail {
            triple_id: 1,
            subject: "Git".to_string(),
            predicate: "never".to_string(),
            object: "force-push to main".to_string(),
            confidence: 0.92,
            source_file: "test".to_string(),
            file_path: "test".to_string(),
        };
        let action = fuzzy_match_action("forbid", &g);
        // "forbid" is about predicate, not action — should fall back to taxonomy
        assert_ne!(action, Action::Create, "forbid should not map to create");
    }

    #[test]
    fn test_fuzzy_action_prevent() {
        let g = dummy_guardrail();
        let action = fuzzy_match_action("prevent", &g);
        assert_ne!(action, Action::Create, "prevent should fall back to taxonomy");
    }

    #[test]
    fn test_fuzzy_action_write_variant() {
        let g = dummy_guardrail();
        assert_eq!(fuzzy_match_action("writing", &g), Action::Create);
        assert_eq!(fuzzy_match_action("create_file", &g), Action::Create);
        assert_eq!(fuzzy_match_action("generate", &g), Action::Create);
    }

    #[test]
    fn test_fuzzy_action_modify_variant() {
        let g = dummy_guardrail();
        assert_eq!(fuzzy_match_action("modify_file", &g), Action::Modify);
        assert_eq!(fuzzy_match_action("editing", &g), Action::Modify);
        assert_eq!(fuzzy_match_action("update_config", &g), Action::Modify);
        assert_eq!(fuzzy_match_action("change", &g), Action::Modify);
    }

    #[test]
    fn test_fuzzy_action_execute_variant() {
        let g = dummy_guardrail();
        assert_eq!(fuzzy_match_action("execute_command", &g), Action::Execute);
        assert_eq!(fuzzy_match_action("run_cli", &g), Action::Execute);
        assert_eq!(fuzzy_match_action("invoke", &g), Action::Execute);
    }

    #[test]
    fn test_fuzzy_action_total_nonsense() {
        // Completely unrecognizable — falls back to taxonomy
        let g = dummy_guardrail();
        let _action = fuzzy_match_action("xyzzy_plugh", &g);
        // Should not panic, should return something valid
    }

    // --- Fuzzy timing matching ---

    #[test]
    fn test_fuzzy_timing_pre_commit() {
        // Qwen 2.5 reported this exact value
        let g = dummy_guardrail();
        let timing = fuzzy_match_timing("pre-commit", &g);
        assert_eq!(timing, crate::intent::Timing::ToolCall);
    }

    #[test]
    fn test_fuzzy_timing_variants() {
        let g = dummy_guardrail();
        assert_eq!(fuzzy_match_timing("pre_tool_use", &g), crate::intent::Timing::ToolCall);
        assert_eq!(fuzzy_match_timing("pretool", &g), crate::intent::Timing::ToolCall);
        assert_eq!(fuzzy_match_timing("on_start", &g), crate::intent::Timing::Start);
        assert_eq!(fuzzy_match_timing("before_finish", &g), crate::intent::Timing::Stop);
        assert_eq!(fuzzy_match_timing("completion", &g), crate::intent::Timing::Stop);
        assert_eq!(fuzzy_match_timing("general_principle", &g), crate::intent::Timing::Principle);
        assert_eq!(fuzzy_match_timing("always_apply", &g), crate::intent::Timing::Principle);
    }

    #[test]
    fn test_fuzzy_timing_nonsense() {
        let g = dummy_guardrail();
        let _timing = fuzzy_match_timing("banana", &g);
        // Should not panic, falls back to taxonomy
    }

    // --- Valid enum detection ---

    #[test]
    fn test_action_is_valid() {
        assert!(Action::is_valid("create"));
        assert!(Action::is_valid("modify"));
        assert!(Action::is_valid("execute"));
        assert!(Action::is_valid("general"));
        assert!(!Action::is_valid("forbid"));
        assert!(!Action::is_valid("prevent"));
        assert!(!Action::is_valid(""));
    }

    #[test]
    fn test_timing_is_valid() {
        assert!(crate::intent::Timing::is_valid("tool_call"));
        assert!(crate::intent::Timing::is_valid("principle"));
        assert!(crate::intent::Timing::is_valid("stop"));
        assert!(crate::intent::Timing::is_valid("start"));
        assert!(!crate::intent::Timing::is_valid("pre-commit"));
        assert!(!crate::intent::Timing::is_valid("pre_tool_use"));
        assert!(!crate::intent::Timing::is_valid(""));
    }

    // --- JSON extraction ---

    #[test]
    fn test_extract_json_from_markdown() {
        let response = "Here is the result:\n```json\n[{\"id\": 1}]\n```\nDone.";
        let result = extract_json_array(response);
        assert!(result.is_some());
        assert!(result.unwrap().contains("\"id\""));
    }

    #[test]
    fn test_extract_json_plain() {
        let response = "[{\"id\": 1, \"action\": \"create\"}]";
        let result = extract_json_array(response);
        assert!(result.is_some());
    }

    #[test]
    fn test_extract_json_with_preamble() {
        // Some models add explanation before the JSON
        let response = "Sure, here are the classifications:\n\n[{\"id\": 1}]";
        let result = extract_json_array(response);
        assert!(result.is_some());
    }

    #[test]
    fn test_extract_json_no_array() {
        let response = "I don't understand what you want me to do.";
        let result = extract_json_array(response);
        assert!(result.is_none());
    }

    // --- Integration: apply_classifications with mixed valid/invalid ---

    #[test]
    fn test_apply_mixed_classifications() {
        use crate::store::Store;
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(500);

        let id = CTR.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("arai_enrich_test_{}", id));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let store = Store::open(&db_path).unwrap();

        let triples = vec![
            crate::parser::Triple {
                subject: "Git".to_string(),
                predicate: "never".to_string(),
                object: "git push to main without a PR".to_string(),
                confidence: 0.92,
                domain: "test".to_string(),
                source_file: "test".to_string(),
                line_start: Some(1),
                line_end: Some(1),
            },
            crate::parser::Triple {
                subject: "Alembic".to_string(),
                predicate: "forbids".to_string(),
                object: "hand-write migration files".to_string(),
                confidence: 0.92,
                domain: "test".to_string(),
                source_file: "test".to_string(),
                line_start: Some(2),
                line_end: Some(2),
            },
        ];

        store.upsert_file("test", "test", &triples, "test").unwrap();
        let guardrails = store.load_guardrails().unwrap();

        // Mixed: first entry has bad values, second has good values
        let classifications: Vec<serde_json::Value> = serde_json::from_str(r#"[
            {"id": 1, "action": "forbid", "timing": "pre-commit", "tools": ["Bash"], "allow_inverse": false},
            {"id": 2, "action": "create", "timing": "tool_call", "tools": ["Write"], "allow_inverse": true}
        ]"#).unwrap();

        let count = apply_classifications(&store, &guardrails, &classifications, &dir).unwrap();
        assert_eq!(count, 2, "both rules should be processed");

        // First rule should be enriched_by "llm-partial" (bad values fuzzy matched)
        let intent1 = store.get_rule_intent(1).unwrap().unwrap();
        assert_eq!(intent1.enriched_by, "llm-partial");

        // Second rule should be enriched_by "llm" (valid values)
        let intent2 = store.get_rule_intent(2).unwrap().unwrap();
        assert_eq!(intent2.enriched_by, "llm");
        assert_eq!(intent2.action, Action::Create);

        std::fs::remove_dir_all(&dir).ok();
    }

    // --- API support ---

    #[test]
    fn test_ensure_completions_path_full() {
        assert_eq!(
            ensure_completions_path("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_ensure_completions_path_v1() {
        assert_eq!(
            ensure_completions_path("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_ensure_completions_path_v1_trailing_slash() {
        assert_eq!(
            ensure_completions_path("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_ensure_completions_path_base_url() {
        assert_eq!(
            ensure_completions_path("http://localhost:11434"),
            "http://localhost:11434/v1/chat/completions"
        );
    }

    #[test]
    fn test_build_enrichment_prompt_format() {
        let guardrails = vec![
            crate::store::Guardrail {
                triple_id: 1,
                subject: "Git".to_string(),
                predicate: "never".to_string(),
                object: "force-push to main".to_string(),
                confidence: 0.92,
                source_file: "test".to_string(),
                file_path: "test".to_string(),
            },
        ];
        let prompt = build_enrichment_prompt(&guardrails);
        assert!(prompt.contains("[id:1] Git never: force-push to main"));
        assert!(prompt.contains("\"action\""));
        assert!(prompt.contains("\"timing\""));
        assert!(prompt.contains("JSON array"));
    }

    #[test]
    fn test_resolve_api_config_explicit_url() {
        let config = resolve_api_config(
            Some("https://api.example.com/v1"),
            None,
            Some("test-model"),
        ).unwrap();
        assert_eq!(config.url, "https://api.example.com/v1/chat/completions");
        assert_eq!(config.model, "test-model");
        assert!(config.api_key.is_none());
    }

    #[test]
    fn test_resolve_api_config_no_config() {
        // No URL, no key, no Ollama → should error
        let result = resolve_api_config(None, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No API endpoint configured"));
    }

    #[test]
    fn test_parse_chat_response_extraction() {
        // Simulate what parse_and_apply does with a mock OpenAI-style response
        // (testing the shared pipeline, not the HTTP call)
        use crate::store::Store;
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(600);

        let id = CTR.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("arai_api_test_{}", id));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let store = Store::open(&db_path).unwrap();

        let triples = vec![crate::parser::Triple {
            subject: "Docker".to_string(),
            predicate: "requires".to_string(),
            object: "use docker compose for local development".to_string(),
            confidence: 0.92,
            domain: "test".to_string(),
            source_file: "test".to_string(),
            line_start: Some(1),
            line_end: Some(1),
        }];
        store.upsert_file("test", "test", &triples, "test").unwrap();
        let guardrails = store.load_guardrails().unwrap();

        // Simulate API response content (the string that would be in choices[0].message.content)
        let response = r#"[{"id": 1, "action": "execute", "timing": "tool_call", "tools": ["Bash"], "allow_inverse": false}]"#;
        let count = parse_and_apply(&store, &guardrails, response, &dir).unwrap();
        assert_eq!(count, 1);

        let intent = store.get_rule_intent(1).unwrap().unwrap();
        assert_eq!(intent.action, Action::Execute);

        std::fs::remove_dir_all(&dir).ok();
    }

    fn dummy_guardrail() -> crate::store::Guardrail {
        crate::store::Guardrail {
            triple_id: 1,
            subject: "General".to_string(),
            predicate: "requires".to_string(),
            object: "follow best practices".to_string(),
            confidence: 0.9,
            source_file: "test".to_string(),
            file_path: "test".to_string(),
        }
    }
}
