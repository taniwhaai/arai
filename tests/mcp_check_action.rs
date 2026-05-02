//! End-to-end behavioural test for the `arai_check_action` MCP tool.
//!
//! Spawns the real arai binary in `arai mcp` mode, drives it with a
//! JSON-RPC `initialize` then a `tools/call arai_check_action` request,
//! and confirms:
//!
//!   1. The probe returns the seeded guardrail in `matched[]`.
//!   2. No audit-log entries are written by the probe (it must remain
//!      pure read — distorts `arai stats` if it bleeds into firings).

use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn temp_arai_home() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "arai_check_action_it_{}_{}",
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn walk_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(p) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&p) {
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push(path);
                }
            }
        }
    }
    out
}

#[test]
fn check_action_returns_matched_rule_without_audit_write() {
    let bin = env!("CARGO_BIN_EXE_arai");
    let arai_home = temp_arai_home();
    let project = arai_home.join("project");
    std::fs::create_dir_all(&project).unwrap();
    // Rule that classifies as ToolCall timing on a known tool, so the
    // probe (PreToolUse / Bash) actually exercises the matcher.  The
    // less-specific "Never force-push to main" rule classifies as
    // `principle` timing and only fires on UserPromptSubmit — useful
    // for end-of-session summaries, the wrong shape for this test.
    std::fs::write(
        project.join("CLAUDE.md"),
        "- Never use docker run for local dev\n",
    )
    .unwrap();

    // Seed the store.
    let scan = Command::new(bin)
        .arg("scan")
        .current_dir(&project)
        .env("ARAI_HOME", &arai_home)
        .output()
        .expect("spawn arai scan");
    assert!(
        scan.status.success(),
        "scan failed: {}",
        String::from_utf8_lossy(&scan.stderr)
    );

    // Drive the MCP server.
    let init_req = json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name":"t","version":"0"}}
    });
    let probe_req = json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {
            "name": "arai_check_action",
            "arguments": {
                "tool": "Bash",
                "tool_input": {"command": "docker run -it ubuntu"}
            }
        }
    });
    let stdin_payload = format!(
        "{}\n{}\n",
        serde_json::to_string(&init_req).unwrap(),
        serde_json::to_string(&probe_req).unwrap()
    );

    let mut child = Command::new(bin)
        .arg("mcp")
        .current_dir(&project)
        .env("ARAI_HOME", &arai_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn arai mcp");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin_payload.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait mcp");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();

    // Pluck the probe response.
    let probe_line = stdout
        .lines()
        .find_map(|l| {
            let v: Value = serde_json::from_str(l).ok()?;
            if v.get("id") == Some(&json!(2)) {
                Some(v)
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("no probe response in mcp stdout: {stdout}"));

    let matched = probe_line
        .pointer("/result/structuredContent/matched")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_else(|| panic!("missing matched array: {probe_line}"));

    // Explicit non-empty guard so a future classifier change that turns
    // the seed rule into `principle` timing fails LOUDLY here with a
    // useful message rather than silently passing the .any() below.
    assert!(
        !matched.is_empty(),
        "probe returned no matches — seed rule may have been reclassified \
         away from ToolCall timing.  Probe response: {probe_line}"
    );
    assert!(
        matched.iter().any(|m| {
            m.get("predicate").and_then(|v| v.as_str()) == Some("never")
                && m.get("object")
                    .and_then(|v| v.as_str())
                    .map(|s| s.contains("docker run"))
                    .unwrap_or(false)
        }),
        "expected the docker rule in matched, got: {matched:?}"
    );

    // Probe must not write any non-empty audit JSONL.  Empty files are
    // tolerated (some platforms touch the audit dir on init); a non-zero
    // .jsonl means the probe slipped past the no-audit contract.
    let audit_root = arai_home.join("audit");
    if audit_root.is_dir() {
        for entry in walk_files(&audit_root) {
            let is_jsonl = entry
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e == "jsonl")
                .unwrap_or(false);
            if !is_jsonl {
                continue;
            }
            let len = std::fs::metadata(&entry).map(|m| m.len()).unwrap_or(0);
            assert_eq!(
                len, 0,
                "arai_check_action wrote audit entry at {entry:?} (size {len})"
            );
        }
    }

    let _ = std::fs::remove_dir_all(&arai_home);
}
