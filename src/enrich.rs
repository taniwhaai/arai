//! Tier 2 intent enrichment using a sentence transformer model.
//!
//! Downloads all-MiniLM-L6-v2 on first use, embeds rule text,
//! and classifies intent by cosine similarity to archetype sentences.

use crate::intent::{Action, RuleIntent};
use crate::store::Store;
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

/// Enrich guardrails by shelling out to an LLM CLI.
/// Uses the configured command (ARAI_LLM_CMD env var or config.toml).
/// Falls back to `claude -p` if nothing configured.
pub fn enrich_via_llm(store: &Store, llm_command: Option<&str>) -> Result<usize, String> {
    let cmd = llm_command
        .map(String::from)
        .or_else(|| detect_llm_command())
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

    // Build the prompt with all rules
    let mut rules_text = String::new();
    for (i, g) in guardrails.iter().enumerate() {
        rules_text.push_str(&format!(
            "{}. [id:{}] {} {}: {}\n",
            i + 1, g.triple_id, g.subject, g.predicate, g.object
        ));
    }

    let prompt = format!(
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
    );

    println!("    Sending {} rules to LLM ({})...", guardrails.len(), cmd);

    let output = run_llm_command(&cmd, &prompt)?;

    let response = output;

    // Extract JSON array from response (claude might wrap it in markdown)
    let json_str = extract_json_array(&response)
        .ok_or("Could not find JSON array in Claude's response")?;

    let classifications: Vec<serde_json::Value> = serde_json::from_str(&json_str)
        .map_err(|e| format!("Failed to parse Claude's response as JSON: {e}"))?;

    let mut count = 0;
    for entry in &classifications {
        let id = entry.get("id").and_then(|v| v.as_i64()).unwrap_or(-1);
        if id < 0 {
            continue;
        }

        // Find the guardrail with this ID
        let guardrail = guardrails.iter().find(|g| g.triple_id == id);
        if guardrail.is_none() {
            continue;
        }

        let action_str = entry.get("action").and_then(|v| v.as_str()).unwrap_or("general");
        let timing_str = entry.get("timing").and_then(|v| v.as_str()).unwrap_or("principle");
        let allow_inverse = entry.get("allow_inverse").and_then(|v| v.as_bool()).unwrap_or(false);

        let tools: Vec<String> = entry
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| vec!["*".to_string()]);

        // Validate: tool_call timing requires the rule to reference a known tool.
        // LLMs sometimes over-classify workflow rules as tool_call.
        let mut timing = crate::intent::Timing::from_str(timing_str);
        if timing == crate::intent::Timing::ToolCall {
            let g = guardrail.unwrap();
            let full_text = format!("{} {}", g.subject.to_lowercase(), g.object.to_lowercase());
            let has_tool = crate::parser::KNOWN_TOOLS.iter().any(|tool| {
                full_text.split(|c: char| !c.is_alphanumeric()).any(|w| w == *tool)
            });
            if !has_tool {
                timing = crate::intent::Timing::Principle;
            }
        }

        let intent = RuleIntent {
            action: Action::from_str(action_str),
            timing,
            tools,
            allow_inverse,
            enriched_by: "llm".to_string(),
        };

        store.upsert_rule_intent(id, &intent).map_err(|e| e.to_string())?;
        count += 1;
    }

    Ok(count)
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
