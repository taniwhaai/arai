//! Minimal MCP server over stdio — two tools an LLM can call to
//! program its own deterministic guardrails mid-session:
//!
//!   `arai_add_guard(rule, reason?)`  → parse + store a rule
//!   `arai_list_guards(pattern?)`     → introspect active rules
//!
//! Matches the Claude Code MCP transport (newline-delimited JSON-RPC 2.0
//! on stdin/stdout).  No async runtime, no external MCP SDK — the wire
//! format is small and we dispatch by method name.
//!
//! Rationale: instruction files (CLAUDE.md) are useful for stable rules,
//! but an agent that discovers a rule mid-session ("from now on, never
//! write to /etc") currently has nowhere to register it such that Ārai
//! will enforce it on the next tool call.  This closes that loop:
//! the agent adds a guard via MCP, Ārai stores it through the same
//! pipeline as `arai add`, and the next PreToolUse hook sees it.

use crate::{audit, config, enrich, parser, store, telemetry};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "arai";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Block on stdin, dispatch JSON-RPC messages until EOF.  Called from
/// `arai mcp`.
pub fn run() -> Result<(), String> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut stdout_lock = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) if l.trim().is_empty() => continue,
            Ok(l) => l,
            Err(e) => return Err(format!("stdin read: {e}")),
        };

        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                // Invalid JSON — per JSON-RPC 2.0 we can't know the id,
                // so emit a parse-error with null id.
                let err = error_response(Value::Null, -32700, &format!("Parse error: {e}"));
                writeln!(stdout_lock, "{}", err).ok();
                continue;
            }
        };

        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let is_notification = req.get("id").is_none();

        let resp = match method {
            "initialize" => Some(success_response(id, handle_initialize())),
            m if m.starts_with("notifications/") => None, // notifications never get responses
            "tools/list" => Some(success_response(id, handle_tools_list())),
            "tools/call" => {
                let params = req.get("params").cloned().unwrap_or(Value::Null);
                match handle_tools_call(&params) {
                    Ok(v) => Some(success_response(id, v)),
                    Err(msg) => Some(error_response(id, -32000, &msg)),
                }
            }
            "ping" => Some(success_response(id, json!({}))),
            other if is_notification => {
                // Unknown notification — silently drop per JSON-RPC 2.0.
                let _ = other;
                None
            }
            other => Some(error_response(id, -32601, &format!("Method not found: {other}"))),
        };

        if let Some(r) = resp {
            writeln!(stdout_lock, "{}", r).map_err(|e| format!("stdout write: {e}"))?;
            stdout_lock.flush().ok();
        }
    }
    Ok(())
}

fn handle_initialize() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
    })
}

fn handle_tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "arai_add_guard",
                "description":
                    "Register a new guardrail that Ārai will enforce on subsequent tool calls. \
                     Use when you discover a rule mid-session that should persist for the rest \
                     of this project (e.g. 'never write to /etc', 'always run tests before push'). \
                     The rule is parsed the same way CLAUDE.md instructions are and stored locally \
                     — it takes effect on the very next PreToolUse hook.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "rule": {
                            "type": "string",
                            "description":
                                "The rule, phrased as an imperative. Examples: 'Never force-push to main', \
                                 'Always run pytest before committing', 'Never edit files in vendor/'."
                        },
                        "reason": {
                            "type": "string",
                            "description":
                                "Optional rationale — why this rule is being added. Recorded in the \
                                 audit log so a human reviewer can see the agent's justification."
                        }
                    },
                    "required": ["rule"]
                }
            },
            {
                "name": "arai_list_guards",
                "description":
                    "List currently active guardrails, optionally filtered by a substring. \
                     Returns subject/predicate/object triples plus their source files so the \
                     agent can see what constraints are live before making a tool call.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Optional case-insensitive substring match against subject/object."
                        }
                    }
                }
            },
            {
                "name": "arai_recent_decisions",
                "description":
                    "Look up the most recent guardrail decisions Ārai has emitted in this \
                     session (or any session if `session_id` is omitted).  Use this when \
                     you've just been denied or warned and want to check whether you've \
                     hit the same rule before — closes the feedback loop so you don't \
                     repeat a refused action.  Returns each firing's tool, decision \
                     (deny/inject/review), the matched rule(s), and the source file the \
                     rule came from.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description":
                                "Optional Claude Code session id.  When set, only decisions \
                                 from that session are returned.  Match the value Claude Code \
                                 passes in hook payloads."
                        },
                        "limit": {
                            "type": "integer",
                            "description":
                                "Maximum decisions to return (default 10, max 50).",
                            "minimum": 1,
                            "maximum": 50
                        },
                        "since": {
                            "type": "string",
                            "description":
                                "Optional time window like '1h', '24h', '7d'.  Defaults to \
                                 the last 24 hours so stale entries don't crowd out today's."
                        }
                    }
                }
            }
        ]
    })
}

fn handle_tools_call(params: &Value) -> Result<Value, String> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing tool name".to_string())?;
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);

    match name {
        "arai_add_guard" => tool_add_guard(&args),
        "arai_list_guards" => tool_list_guards(&args),
        "arai_recent_decisions" => tool_recent_decisions(&args),
        other => Err(format!("unknown tool: {other}")),
    }
}

fn tool_add_guard(args: &Value) -> Result<Value, String> {
    let rule = args
        .get("rule")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'rule'".to_string())?;
    let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("");

    if rule.trim().is_empty() {
        return Err("rule is empty".to_string());
    }

    let cfg = config::Config::load()?;
    let db = store::Store::open(&cfg.db_path())?;

    // Parse via the same path as `arai add`: extract triples from the
    // imperative, store under a content-hashed manual:// path so repeat
    // calls don't collide.
    let triples = parser::extract_rules(&format!("- {rule}"), "mcp", 0.9);
    if triples.is_empty() {
        return Err(format!(
            "could not extract a guardrail from: {rule:?} \
             (try an imperative like 'Never force-push to main')"
        ));
    }

    let manual_path = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(rule.as_bytes());
        let hash = h.finalize();
        let short: String = hash.iter().take(4).map(|b| format!("{b:02x}")).collect();
        format!("manual://arai-mcp/{short}")
    };
    db.upsert_file(&manual_path, rule, &triples, "mcp")
        .map_err(|e| e.to_string())?;
    db.classify_all_guardrails().map_err(|e| e.to_string())?;

    // Best-effort enrichment if the ST model is already present (no download).
    let model_dir = cfg.arai_base_dir.join("models").join("all-MiniLM-L6-v2");
    if model_dir.join("model.onnx").exists() {
        enrich::enrich_guardrails(&db, &cfg.arai_base_dir).ok();
    }

    // Track via anonymous telemetry the same way a CLI add would.
    for t in &triples {
        telemetry::track(
            &cfg.arai_base_dir,
            "rule_added",
            json!({
                "subject": t.subject,
                "predicate": t.predicate,
                "source": "mcp",
            }),
        );
    }

    let summary: Vec<String> = triples
        .iter()
        .map(|t| format!("- {} {}: {}", t.subject, t.predicate, t.object))
        .collect();
    let mut text = format!("Added {} guard(s):\n{}", triples.len(), summary.join("\n"));
    if !reason.is_empty() {
        text.push_str(&format!("\n\nReason: {reason}"));
    }

    Ok(content_text(&text))
}

fn tool_list_guards(args: &Value) -> Result<Value, String> {
    let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
    let cfg = config::Config::load()?;
    let db = store::Store::open(&cfg.db_path())?;
    let rails = db.load_guardrails().map_err(|e| e.to_string())?;

    let filtered: Vec<_> = if pattern.is_empty() {
        rails.iter().collect()
    } else {
        let needle = pattern.to_lowercase();
        rails
            .iter()
            .filter(|g| {
                g.subject.to_lowercase().contains(&needle)
                    || g.object.to_lowercase().contains(&needle)
            })
            .collect()
    };

    if filtered.is_empty() {
        return Ok(content_text(
            "No guardrails match. Try `arai_add_guard` or adjust your pattern.",
        ));
    }

    let lines: Vec<String> = filtered
        .iter()
        .map(|g| {
            format!(
                "- {} {}: {}  (from {}, confidence {:.2})",
                g.subject, g.predicate, g.object, g.file_path, g.confidence
            )
        })
        .collect();
    let text = format!("{} active guard(s):\n{}", filtered.len(), lines.join("\n"));
    Ok(content_text(&text))
}

/// Look up recent hook decisions from the audit log so the calling agent can
/// see what Arai has just denied / injected / reviewed.  Pure read — no
/// audit write, no telemetry.
fn tool_recent_decisions(args: &Value) -> Result<Value, String> {
    let session_id = args.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n.clamp(1, 50) as usize)
        .unwrap_or(10);
    let since = args.get("since").and_then(|v| v.as_str()).unwrap_or("24h");
    let since_epoch = parse_since_window(since)?;

    let cfg = config::Config::load()?;
    // Over-fetch when filtering by session so the per-session window has
    // material — same trick `cmd_audit` uses for outcome/rule filters.
    let raw_limit = if session_id.is_empty() {
        limit
    } else {
        limit.saturating_mul(8)
    };
    let entries = audit::query(
        &cfg.arai_base_dir,
        &cfg.project_slug(),
        Some(since_epoch),
        None,
        None,
        raw_limit,
    )
    .map_err(|e| e.to_string())?;

    // Skip Compliance events — they're verdicts, not decisions, and the
    // model doesn't need to self-check against them via this surface.
    let filtered: Vec<&Value> = entries
        .iter()
        .filter(|e| {
            let ev = e.get("event").and_then(|v| v.as_str()).unwrap_or("");
            if ev == "Compliance" {
                return false;
            }
            if !session_id.is_empty() {
                e.get("session").and_then(|v| v.as_str()) == Some(session_id)
            } else {
                true
            }
        })
        .take(limit)
        .collect();

    if filtered.is_empty() {
        let scope = if session_id.is_empty() {
            "any session".to_string()
        } else {
            format!("session {session_id}")
        };
        return Ok(content_text(&format!(
            "No Ārai decisions in the last {since} for {scope}."
        )));
    }

    let mut lines = Vec::new();
    for entry in &filtered {
        let ts = entry.get("ts").and_then(|v| v.as_str()).unwrap_or("?");
        let event = entry.get("event").and_then(|v| v.as_str()).unwrap_or("?");
        let tool = entry.get("tool").and_then(|v| v.as_str()).unwrap_or("?");
        let decision = entry.get("decision").and_then(|v| v.as_str()).unwrap_or("?");
        let preview = entry
            .get("prompt_preview")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let rules: Vec<String> = entry
            .get("rules")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|r| {
                        format!(
                            "{} {}: {}",
                            r.get("subject").and_then(|v| v.as_str()).unwrap_or(""),
                            r.get("predicate").and_then(|v| v.as_str()).unwrap_or(""),
                            r.get("object").and_then(|v| v.as_str()).unwrap_or(""),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let rules_part = if rules.is_empty() {
            String::new()
        } else {
            format!(" [{}]", rules.join("; "))
        };
        let preview_part = if preview.is_empty() {
            String::new()
        } else {
            // Trim long previews so the MCP response stays compact.
            let short: String = preview.chars().take(80).collect();
            format!(" — {short}")
        };
        lines.push(format!(
            "{ts}  {event:<13} {tool:<6} {decision:<6}{rules_part}{preview_part}"
        ));
    }

    let scope = if session_id.is_empty() {
        format!("last {since}, all sessions")
    } else {
        format!("last {since}, session {session_id}")
    };
    let text = format!(
        "{} recent decision(s) ({scope}):\n{}",
        filtered.len(),
        lines.join("\n"),
    );
    Ok(content_text(&text))
}

/// Tiny duration parser shared with `tool_recent_decisions`.  We don't pull
/// `parse_since` from `main.rs` because that function lives outside the
/// library surface; duplicating ~10 lines is cheaper than restructuring
/// the binary just to share it.
fn parse_since_window(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("`since` cannot be empty".to_string());
    }
    let (num_part, unit_secs): (&str, u64) = match s.chars().last().unwrap() {
        'd' => (&s[..s.len() - 1], 86_400),
        'h' => (&s[..s.len() - 1], 3_600),
        'm' => (&s[..s.len() - 1], 60),
        's' => (&s[..s.len() - 1], 1),
        c if c.is_ascii_digit() => (s, 1),
        other => return Err(format!("`since`: unknown unit '{other}' (use d/h/m/s)")),
    };
    let n: u64 = num_part
        .parse()
        .map_err(|_| format!("`since`: invalid number '{num_part}'"))?;
    let delta = n.checked_mul(unit_secs).ok_or("`since`: overflow")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(now.saturating_sub(delta))
}

fn content_text(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }] })
}

fn success_response(id: Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn error_response(id: Value, code: i32, message: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_advertises_tools() {
        let v = handle_initialize();
        assert!(v.get("capabilities").and_then(|c| c.get("tools")).is_some());
        assert_eq!(v["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(v["serverInfo"]["name"], SERVER_NAME);
    }

    #[test]
    fn tools_list_has_three_entries() {
        let v = handle_tools_list();
        let tools = v["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"arai_add_guard"));
        assert!(names.contains(&"arai_list_guards"));
        assert!(names.contains(&"arai_recent_decisions"));
    }

    #[test]
    fn parse_since_window_basics() {
        assert!(parse_since_window("1h").is_ok());
        assert!(parse_since_window("7d").is_ok());
        assert!(parse_since_window("60").is_ok()); // bare digits = seconds
        assert!(parse_since_window("").is_err());
        assert!(parse_since_window("xyz").is_err());
        assert!(parse_since_window("3y").is_err()); // unknown unit
    }

    #[test]
    fn tool_call_requires_name() {
        let err = handle_tools_call(&json!({})).unwrap_err();
        assert!(err.contains("missing tool name"));
    }

    #[test]
    fn success_response_shape() {
        let s = success_response(json!(1), json!({"ok": true}));
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 1);
        assert_eq!(v["result"]["ok"], true);
    }

    #[test]
    fn error_response_shape() {
        let s = error_response(json!("x"), -32601, "Method not found: foo");
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["error"]["code"], -32601);
        assert_eq!(v["error"]["message"], "Method not found: foo");
    }
}
