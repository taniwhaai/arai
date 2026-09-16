//! Exercise the real stdin/JSON decision contract without executing tool commands.
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Project(PathBuf);

impl Project {
    fn new(rules: &str) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("arai_codex_{}_{stamp}", std::process::id()));
        fs::create_dir_all(root.join("project/.git")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        fs::write(root.join("project/AGENTS.md"), rules).unwrap();
        let project = Self(root);
        let out = project.command().arg("scan").output().unwrap();
        assert!(
            out.status.success(),
            "scan: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        project
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_arai"));
        cmd.current_dir(self.0.join("project"))
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("ARAI_BASE_DIR", self.0.join("state"))
            .env("ARAI_TELEMETRY", "off")
            .env_remove("ARAI_DISABLED")
            .env_remove("ARAI_DENY_MODE")
            .env_remove("GROK_HOOK_EVENT")
            .env_remove("GROK_SESSION_ID")
            .env_remove("CLAUDE_PROJECT_DIR")
            .env_remove("CLAUDE_PLUGIN_ROOT");
        cmd
    }

    fn hook(&self, event: &str, tool: &str, input: Value) -> Option<Value> {
        let mut child = self
            .command()
            .args(["guardrails", "--match-stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let payload = json!({"hook_event_name":event,"tool_name":tool,"tool_input":input,
            "session_id":"codex-test", "tool_response":{"output":"from alembic import op","exit_code":0}});
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "hook: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        if out.stdout.is_empty() {
            None
        } else {
            Some(serde_json::from_slice(&out.stdout).expect("hook JSON"))
        }
    }

    fn denies(&self, tool: &str, input: Value) -> bool {
        self.hook("PreToolUse", tool, input)
            .is_some_and(|out| out["hookSpecificOutput"]["permissionDecision"] == "deny")
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

#[test]
fn standalone_arai_does_not_exempt_compound_shell_commands() {
    let p = Project::new("- Never run cargo clean\n");
    assert!(p.denies("Bash", json!({"command":"cargo clean"})));
    for command in [
        "arai status && cargo clean",
        "arai status; cargo clean",
        "arai status\ncargo clean",
        "arai why $(cargo clean)",
    ] {
        assert!(p.denies("Bash", json!({"command":command})), "{command}");
    }
    assert!(!p.denies("Bash", json!({"command":"arai why cargo clean"})));
    assert!(!p.denies("Bash", json!({"command":"cargo check"})));
}

#[test]
fn patch_add_is_write_and_existing_update_is_edit() {
    let p = Project::new("## Alembic\n\n- Never hand-write migration files\n");
    for patch in [
        "*** Begin Patch\n*** Add File: alembic/notes.txt\n+hello\n*** End Patch",
        "*** Begin Patch\n*** Add File: generated.py\n+from alembic import op\n*** End Patch",
        "*** Begin Patch\n*** Add File: README.md\n+# hello\n*** Add File: alembic/notes.txt\n+++ harmless\n*** End Patch",
        "*** Begin Patch\n*** Update File: scratch.py\n*** Move to: alembic/new.py\n@@\n-pass\n+from alembic import op\n*** End Patch",
    ] {
        assert!(p.denies("apply_patch", json!({"command":patch})), "{patch}");
    }
    assert!(!p.denies("apply_patch", json!({"command":"*** Begin Patch\n*** Update File: alembic/notes.txt\n@@\n-old\n+new\n*** End Patch"})));
    assert!(!p.denies(
        "apply_patch",
        json!({"command":"*** Begin Patch\n*** Add File: README.md\n+# hello\n*** End Patch"})
    ));
    // Other hosts' historical file-shaped alias remains compatible.
    assert!(!p.denies(
        "apply_patch",
        json!({"file_path":"alembic/notes.txt","old_string":"old","new_string":"new"})
    ));
    assert!(p.denies("apply_patch", json!({"file_path":"README.md", "command":"*** Begin Patch\n*** Add File: alembic/new.py\n+hello\n*** End Patch"})));
    #[cfg(windows)]
    assert!(p.denies(
        "apply_patch",
        json!({"command":"*** Begin Patch\n*** Add File: alembic\\new.py\n+hello\n*** End Patch"})
    ));
}

#[test]
fn malformed_patch_fails_closed_and_post_accepts_structured_response() {
    let p = Project::new("## Alembic\n\n- Never hand-write migration files\n");
    for input in [
        json!({}),
        json!({"command":"bad patch"}),
        json!({"command":"*** Begin Patch\n*** Add File: alembic/x\n+x"}),
    ] {
        assert!(p.denies("apply_patch", input));
    }
    let input =
        json!({"command":"*** Begin Patch\n*** Add File: README.md\n+# hello\n*** End Patch"});
    p.hook("PostToolUse", "apply_patch", input);
    let session = fs::read_to_string(p.0.join("state/sessions/codex-test.json")).unwrap();
    assert!(
        session.contains("alembic"),
        "structured tool_response must seed observed terms: {session}"
    );
}
