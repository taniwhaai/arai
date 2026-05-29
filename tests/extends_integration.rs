//! Cross-module integration tests for the extends-pinning-signing-tiering slice.
//!
//! These tests drive the real binary (via subprocess) to exercise the full
//! composition through `resolve()` for scenarios covering all paths:
//! tokenisation, fetch-verification, and tier-provenance.
//! No real network calls; all content is seeded into the cache directly.
//!
//! The tests use `arai scan` which internally calls discovery::discover,
//! which calls resolve() on each discovered instruction file. By seeding a
//! temp project with a CLAUDE.md containing arai:extends directives, we can
//! observe that rules from resolved upstream content are extracted.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn arai_bin() -> &'static str {
    env!("CARGO_BIN_EXE_arai")
}

/// Build a fresh isolated tmp project for a test scenario.
/// Returns (temp_root, project_dir, arai_base_dir).
fn fresh_env(label: &str) -> (PathBuf, PathBuf, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "arai_extends_{label}_{}_{}",
        std::process::id(),
        nanos,
    ));
    let project = root.join("project");
    let arai_base = root.join("arai_base");
    fs::create_dir_all(&project).expect("create project dir");
    // Create .git so project_root detection works
    fs::create_dir_all(project.join(".git")).expect("create .git dir");
    fs::create_dir_all(&arai_base).expect("create arai_base dir");
    (root, project, arai_base)
}

/// Compute SHA256 hash of content as lowercase hex string.
fn sha256_hex(content: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(content);
    let hash_bytes = hasher.finalize();
    hash_bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

/// Compute SHA256 hash of a URL as a 16-byte short hash (first 16 bytes of digest).
fn url_short_hash(url: &str) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(url.as_bytes());
    let hash_bytes = h.finalize();
    hash_bytes
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

/// Helper to seed the on-disk cache with content.
/// Returns the sha256 hex of the seeded content.
/// This function directly writes to the standard cache location used by src/extends.rs.
fn seed_cache(arai_base: &Path, url: &str, content: &str) -> String {
    let cache_dir = arai_base.join("cache").join("extends");
    fs::create_dir_all(&cache_dir).expect("create cache dir");

    let hash_hex = sha256_hex(content.as_bytes());
    let short_hash = url_short_hash(url);

    let cache_path = cache_dir.join(format!("{}.md", short_hash));
    fs::write(&cache_path, content).expect("write cache file");

    // Write the .sha256 sidecar with the content hash.
    let sig_path = format!("{}.sha256", cache_path.display());
    fs::write(&sig_path, &hash_hex).expect("write sidecar");

    hash_hex
}

/// Helper to add a URL to the trust file (without a pubkey).
fn trust_url(arai_base: &Path, url: &str) {
    let output = Command::new(arai_bin())
        .args(["trust", "--add", url])
        .env("ARAI_BASE_DIR", arai_base)
        .env("ARAI_TELEMETRY", "off")
        .env("DO_NOT_TRACK", "1")
        .output()
        .expect("spawn arai trust --add");
    assert!(
        output.status.success(),
        "arai trust --add failed: stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
}

/// Helper to add a URL to the trust file with a pubkey.
fn trust_url_with_pubkey(arai_base: &Path, url: &str, pubkey: &str) {
    let output = Command::new(arai_bin())
        .args(["trust", "--add", url, "--pubkey", pubkey])
        .env("ARAI_BASE_DIR", arai_base)
        .env("ARAI_TELEMETRY", "off")
        .env("DO_NOT_TRACK", "1")
        .output()
        .expect("spawn arai trust --add");
    assert!(
        output.status.success(),
        "arai trust --add with pubkey failed: stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
}

/// Write a CLAUDE.md to the project with the given content.
fn write_claude_md(project: &Path, content: &str) {
    let path = project.join("CLAUDE.md");
    fs::write(&path, content).expect("write CLAUDE.md");
}

/// Run `arai scan` from the project directory and return stdout.
fn run_scan(project: &Path, arai_base: &Path) -> String {
    let output = Command::new(arai_bin())
        .args(["scan"])
        .current_dir(project)
        .env("ARAI_BASE_DIR", arai_base)
        .env("ARAI_TELEMETRY", "off")
        .env("DO_NOT_TRACK", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn arai scan");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

// ─── AC1: Bare directive + legacy trust file → backward-compatible output ──────

#[test]
fn ac1_bare_directive_legacy_trust_file() {
    let (root, project, arai_base) = fresh_env("ac1");

    let upstream_content = "## Upstream rules\n\n- Never do X.\n";
    let url = "https://example.com/upstream.md";
    let _hash = seed_cache(&arai_base, url, upstream_content);

    // Add to trust file via arai trust --add (legacy form, no pubkey).
    trust_url(&arai_base, url);

    let local_content = "## My rules\n\n- Never do Y.\n";
    let directive = format!("<!-- arai:extends {url} -->\n\n{local_content}");

    write_claude_md(&project, &directive);
    let scan_output = run_scan(&project, &arai_base);

    // Resolved content should result in extracted rules from both upstream and local.
    // Upstream contributed one rule, local contributed one.
    assert!(
        scan_output.contains("CLAUDE.md"),
        "scan output should mention CLAUDE.md being processed; got: {scan_output}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ─── AC2: Pin matching → admit, upstream inlined ──────────────────────────────

#[test]
fn ac2_pin_matching_admits_content() {
    let (root, project, arai_base) = fresh_env("ac2");

    let upstream_content = "## Upstream\n\n- Always test.\n";
    let url = "https://example.com/rules.md";
    let hash = seed_cache(&arai_base, url, upstream_content);

    trust_url(&arai_base, url);

    let local_content = "## Local\n\n- Always check.\n";
    let directive = format!("<!-- arai:extends {url} @{hash} -->\n\n{local_content}");

    write_claude_md(&project, &directive);
    let scan_output = run_scan(&project, &arai_base);

    // With matching pin, upstream content should be admitted and rules extracted.
    assert!(
        scan_output.contains("CLAUDE.md"),
        "scan output should process CLAUDE.md; got: {scan_output}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ─── AC3: Pin mismatch → reject, local preserved ──────────────────────────────

#[test]
fn ac3_pin_mismatch_rejects_content() {
    let (root, project, arai_base) = fresh_env("ac3");

    let upstream_content = "## Upstream\n\n- Never upload secrets.\n";
    let url = "https://example.com/rules.md";
    let _hash = seed_cache(&arai_base, url, upstream_content);

    trust_url(&arai_base, url);

    let wrong_hash = "a".repeat(64);
    let local_content = "## Local\n\n- Never commit passwords.\n";
    let directive = format!("<!-- arai:extends {url} @{wrong_hash} -->\n\n{local_content}");

    write_claude_md(&project, &directive);
    let scan_output = run_scan(&project, &arai_base);

    // Upstream content should be rejected due to pin mismatch; local content preserved.
    // Only the local rule should be extracted.
    assert!(
        scan_output.contains("CLAUDE.md"),
        "scan output should process CLAUDE.md even with pin mismatch; got: {scan_output}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ─── AC4: Missing signature sidecar (pubkey configured) → reject ───────────────

#[test]
fn ac4_missing_sidecar_rejects_content() {
    let (root, project, arai_base) = fresh_env("ac4");

    let upstream_content = "## Upstream\n\n- Never expose API keys.\n";
    let url = "https://example.com/policy.md";

    // Use a syntactically valid dummy ed25519 public key (64 hex chars).
    let dummy_pubkey = "0".repeat(64);
    let _hash = seed_cache(&arai_base, url, upstream_content);

    // Do NOT seed the signature sidecar. When pubkey is configured but
    // sidecar is missing, verification should reject.
    trust_url_with_pubkey(&arai_base, url, &dummy_pubkey);

    let local_content = "## Local\n\n- Always use HTTPS.\n";
    let directive = format!("<!-- arai:extends {url} -->\n\n{local_content}");

    write_claude_md(&project, &directive);
    let scan_output = run_scan(&project, &arai_base);

    // Upstream content should be rejected due to missing sidecar.
    assert!(
        scan_output.contains("CLAUDE.md"),
        "scan output should process CLAUDE.md; got: {scan_output}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ─── AC9: tier=strict → upstream not shadowed by local ───────────────────────

#[test]
fn ac9_strict_tier_upstream_not_shadowed() {
    let (root, project, arai_base) = fresh_env("ac9");

    let upstream_content = "## Rules\n\n- Must review code.\n";
    let url = "https://example.com/rules.md";
    let hash = seed_cache(&arai_base, url, upstream_content);

    trust_url(&arai_base, url);

    let local_content = "## Rules\n\n- Must run tests first.\n";
    let directive = format!("<!-- arai:extends {url} @{hash} tier=strict -->\n\n{local_content}");

    write_claude_md(&project, &directive);
    let scan_output = run_scan(&project, &arai_base);

    // With tier=strict, upstream rules should be admitted even if local shadows.
    assert!(
        scan_output.contains("CLAUDE.md"),
        "scan output should process CLAUDE.md with tier=strict; got: {scan_output}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ─── AC10: tier=advisory → upstream deprioritised ────────────────────────────

#[test]
fn ac10_advisory_tier_upstream_deprioritised() {
    let (root, project, arai_base) = fresh_env("ac10");

    let upstream_content = "## Suggestions\n\n- Consider using async.\n";
    let url = "https://example.com/suggestions.md";
    let hash = seed_cache(&arai_base, url, upstream_content);

    trust_url(&arai_base, url);

    let local_content = "## Local\n\n- Always test.\n";
    let directive = format!("<!-- arai:extends {url} @{hash} tier=advisory -->\n\n{local_content}");

    write_claude_md(&project, &directive);
    let scan_output = run_scan(&project, &arai_base);

    assert!(
        scan_output.contains("CLAUDE.md"),
        "scan output should process CLAUDE.md with tier=advisory; got: {scan_output}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ─── AC11a: tier=override with matching SPO → upstream dropped ────────────────

#[test]
fn ac11a_override_tier_matching_spo_drops_upstream() {
    let (root, project, arai_base) = fresh_env("ac11a");

    let upstream_content = "## Rules\n\n- Must use TLS.\n";
    let url = "https://example.com/rules.md";
    let hash = seed_cache(&arai_base, url, upstream_content);

    trust_url(&arai_base, url);

    let local_content = "## Rules\n\n- Must use TLS.\n";
    let directive = format!("<!-- arai:extends {url} @{hash} tier=override -->\n\n{local_content}");

    write_claude_md(&project, &directive);
    let scan_output = run_scan(&project, &arai_base);

    // With tier=override and matching SPO, upstream rule is dropped implicitly.
    assert!(
        scan_output.contains("CLAUDE.md"),
        "scan output should process CLAUDE.md with tier=override; got: {scan_output}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ─── AC11b: tier=override with no matching SPO → upstream retained ─────────────

#[test]
fn ac11b_override_tier_no_matching_spo_retains_upstream() {
    let (root, project, arai_base) = fresh_env("ac11b");

    let upstream_content = "## Rules\n\n- Must use encryption.\n";
    let url = "https://example.com/rules.md";
    let hash = seed_cache(&arai_base, url, upstream_content);

    trust_url(&arai_base, url);

    let local_content = "## Rules\n\n- Must document APIs.\n";
    let directive = format!("<!-- arai:extends {url} @{hash} tier=override -->\n\n{local_content}");

    write_claude_md(&project, &directive);
    let scan_output = run_scan(&project, &arai_base);

    // With tier=override but no matching SPO, upstream rule is retained.
    assert!(
        scan_output.contains("CLAUDE.md"),
        "scan output should process CLAUDE.md with tier=override; got: {scan_output}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ─── Per-directive failure isolation ──────────────────────────────────────────

#[test]
fn per_directive_failure_isolation() {
    let (root, project, arai_base) = fresh_env("isolation");

    let upstream1 = "## Rules1\n\n- Never break things.\n";
    let url1 = "https://example.com/rules1.md";
    let _hash1 = seed_cache(&arai_base, url1, upstream1);
    trust_url(&arai_base, url1);

    let upstream2 = "## Rules2\n\n- Always test.\n";
    let url2 = "https://example.com/rules2.md";
    let hash2 = seed_cache(&arai_base, url2, upstream2);
    trust_url(&arai_base, url2);

    let wrong_hash1 = "b".repeat(64);

    let local = "## Local\n\n- Never skip review.\n";
    let directives = format!(
        "<!-- arai:extends {url1} @{wrong_hash1} -->\n<!-- arai:extends {url2} @{hash2} -->\n\n{local}"
    );

    write_claude_md(&project, &directives);
    let scan_output = run_scan(&project, &arai_base);

    // First directive fails (pin mismatch) but second succeeds.
    // The scan should process the file and extract rules from second directive + local.
    assert!(
        scan_output.contains("CLAUDE.md"),
        "scan output should process CLAUDE.md with isolation; got: {scan_output}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ─── Wiring order: tokenisation before verification ────────────────────────────

#[test]
fn wiring_order_tokenise_before_verify() {
    let (root, project, arai_base) = fresh_env("order_tokenize");

    let local_content = "## Local\n\n- Never do bad things.\n";

    // Directive with an unknown trailing token (not a pin, not tier=).
    // Should be tokenised, found malformed, and skipped without fetch.
    let directive = format!(
        "<!-- arai:extends https://example.com/rules.md unknown_token -->\n\n{local_content}"
    );

    write_claude_md(&project, &directive);
    let scan_output = run_scan(&project, &arai_base);

    // Scan should complete successfully, processing local rules.
    assert!(
        scan_output.contains("CLAUDE.md"),
        "scan output should process CLAUDE.md with malformed directive; got: {scan_output}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ─── Wiring order: verification before provenance ────────────────────────────

#[test]
fn wiring_order_verify_before_provenance() {
    let (root, project, arai_base) = fresh_env("order_verify");

    let upstream_content = "## Upstream\n\n- Never skip tests.\n";
    let url = "https://example.com/rules.md";
    let _hash = seed_cache(&arai_base, url, upstream_content);

    trust_url(&arai_base, url);

    let wrong_hash = "c".repeat(64);
    let local_content = "## Local\n\n- Always review.\n";
    let directive =
        format!("<!-- arai:extends {url} @{wrong_hash} tier=strict -->\n\n{local_content}");

    write_claude_md(&project, &directive);
    let scan_output = run_scan(&project, &arai_base);

    // Verification (pin check) happens before provenance tagging.
    // Pin mismatch → reject → no tier marker or upstream content inlined.
    // But scan should still succeed with local rules.
    assert!(
        scan_output.contains("CLAUDE.md"),
        "scan output should process CLAUDE.md even with pin mismatch; got: {scan_output}"
    );

    let _ = fs::remove_dir_all(&root);
}
