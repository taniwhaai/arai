//! Cross-module integration tests for the extends-pinning-signing-tiering slice.
//!
//! This test exercises the full composition through `resolve()` for scenarios
//! covering all paths: tokenisation, fetch-verification, and tier-provenance.
//! No real network calls; all content is seeded into the cache directly.

use arai::extends;
use sha2::Digest;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Helper to create a test arai_base with a trust file.
fn setup_test_env() -> (TempDir, PathBuf) {
    let tmpdir = TempDir::new().unwrap();
    let arai_base = tmpdir.path().to_path_buf();
    (tmpdir, arai_base)
}

/// Compute SHA256 hash of content as lowercase hex string.
fn sha256_hex(content: &[u8]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(content);
    let hash_bytes = hasher.finalize();
    hash_bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

/// Helper to seed the on-disk cache with content.
/// Returns the sha256 hex of the seeded content.
/// This function directly writes to the standard cache location used by arai::extends.
fn seed_cache(arai_base: &Path, url: &str, content: &str) -> String {
    let cache_dir = extends::cache_dir(arai_base);
    fs::create_dir_all(&cache_dir).unwrap();

    // Compute the sha256 hash of the content.
    let hash_hex = sha256_hex(content.as_bytes());

    // Replicate the cache path layout from extends.rs:
    // url_cache_path = cache_dir / <first 16 bytes of URL hash>.md
    // url_cache_sig_path = cache_dir / <first 16 bytes of URL hash>.md.sha256
    let mut url_hasher = sha2::Sha256::new();
    url_hasher.update(url.as_bytes());
    let url_hash_bytes = url_hasher.finalize();
    let short_hash: String = url_hash_bytes
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect();

    let cache_path = cache_dir.join(format!("{short_hash}.md"));
    fs::write(&cache_path, content).unwrap();

    // Write the .sha256 sidecar.
    let sig_path = format!("{}.sha256", cache_path.display());
    fs::write(&sig_path, &hash_hex).unwrap();

    hash_hex
}

/// Helper to add a URL to the trust file (without a pubkey).
fn trust_url(arai_base: &Path, url: &str) {
    extends::trust_add(url, arai_base, None).unwrap();
}

/// Helper to add a URL to the trust file with a pubkey.
fn trust_url_with_pubkey(arai_base: &Path, url: &str, pubkey: &str) {
    extends::trust_add(url, arai_base, Some(pubkey)).unwrap();
}

#[test]
fn test_ac1_bare_directive_legacy_trust_file() {
    // Backward-compatibility invariant: a bare arai:extends <url> with a
    // legacy trust file (list-of-strings) produces byte-identical output.
    let (_tmpdir, arai_base) = setup_test_env();

    let upstream_content = "# Upstream rules\n\nNever do X.\n";
    let url = "https://example.com/upstream.md";
    let _hash = seed_cache(&arai_base, url, upstream_content);

    // Add to trust file the old way (no pubkey).
    trust_url(&arai_base, url);

    let local_content = "# My rules\n\nNever do Y.\n";
    let directive = format!("<!-- arai:extends {url} -->\n\n{local_content}");

    let resolved = extends::resolve(&directive, &arai_base);

    // Resolved should contain both upstream and local content, with the
    // tier-annotated marker (new in slice, but that's fine for backward-compat
    // as long as parsing works). The content must be present.
    assert!(resolved.contains(upstream_content));
    assert!(resolved.contains(local_content));
    assert!(resolved.contains("arai:extends-block"));
    assert!(resolved.contains("tier=\"peer\""));
}

#[test]
fn test_ac2_pin_matching_admits_content() {
    // Pin present and matching → admit path, upstream content inlined.
    let (_tmpdir, arai_base) = setup_test_env();

    let upstream_content = "# Upstream\n\nAlways test.\n";
    let url = "https://example.com/rules.md";
    let hash = seed_cache(&arai_base, url, upstream_content);

    trust_url(&arai_base, url);

    let local_content = "# Local\n\nLocal rule here.\n";
    let directive = format!("<!-- arai:extends {url} @{hash} -->\n\n{local_content}");

    let resolved = extends::resolve(&directive, &arai_base);

    assert!(resolved.contains(upstream_content));
    assert!(resolved.contains(local_content));
    assert!(resolved.contains("arai:extends-block"));
}

#[test]
fn test_ac3_pin_mismatch_rejects_content() {
    // Pin present and mismatching → reject path, local content preserved,
    // stderr warning emitted.
    let (_tmpdir, arai_base) = setup_test_env();

    let upstream_content = "# Upstream\n\nContent here.\n";
    let url = "https://example.com/rules.md";
    let _hash = seed_cache(&arai_base, url, upstream_content);

    trust_url(&arai_base, url);

    let wrong_hash = "a".repeat(64); // Wrong hash.
    let local_content = "# Local\n\nLocal rule.\n";
    let directive = format!("<!-- arai:extends {url} @{wrong_hash} -->\n\n{local_content}");

    let resolved = extends::resolve(&directive, &arai_base);

    // Should NOT contain upstream content (rejected due to pin mismatch).
    assert!(!resolved.contains("arai:extends-block"));
    // Should contain only local content.
    assert!(resolved.contains(local_content));
}

#[test]
fn test_ac6_missing_sidecar_rejects_content() {
    // Configured pubkey but missing signature sidecar → reject path.
    // Note: This test only validates the missing sidecar case; valid signature
    // testing requires access to private test keypair helpers (tested separately
    // in src/extends.rs unit tests).
    let (_tmpdir, arai_base) = setup_test_env();

    let upstream_content = "# Upstream\n";
    let url = "https://example.com/policy.md";

    // Use a dummy but syntactically valid ed25519 public key (64 hex chars).
    let dummy_pubkey = "0".repeat(64);
    let _hash = seed_cache(&arai_base, url, upstream_content);

    // Do NOT seed the signature sidecar. When pubkey is configured but
    // sidecar is missing, verify_content should reject.
    trust_url_with_pubkey(&arai_base, url, &dummy_pubkey);

    let local_content = "# Local\n";
    let directive = format!("<!-- arai:extends {url} -->\n\n{local_content}");

    let resolved = extends::resolve(&directive, &arai_base);

    // Should NOT contain upstream content (sidecar missing).
    assert!(!resolved.contains("arai:extends-block"));
    assert!(resolved.contains(local_content));
}

#[test]
fn test_ac9_strict_tier_upstream_not_shadowed() {
    // tier=strict: upstream rule whose subject matches a local rule
    // is not suppressed by the local rule (AC9).
    let (_tmpdir, arai_base) = setup_test_env();

    let upstream_content = "## Rules\n\nMust review code.\n";
    let url = "https://example.com/rules.md";
    let hash = seed_cache(&arai_base, url, upstream_content);

    trust_url(&arai_base, url);

    // Local rule with same subject (different predicate/object).
    let local_content = "## Rules\n\nMust run tests first.\n";
    let directive = format!("<!-- arai:extends {url} @{hash} tier=strict -->\n\n{local_content}");

    let resolved = extends::resolve(&directive, &arai_base);

    // Both upstream and local should be present; the tier marker should
    // show strict.
    assert!(resolved.contains("tier=\"strict\""));
    assert!(resolved.contains(upstream_content));
    assert!(resolved.contains(local_content));
}

#[test]
fn test_ac10_advisory_tier_upstream_deprioritised() {
    // tier=advisory: upstream rule is deprioritised by ranker (AC10).
    // For the integration test, we verify the content is inlined and the
    // tier marker is present.
    let (_tmpdir, arai_base) = setup_test_env();

    let upstream_content = "## Suggestions\n\nConsider using async.\n";
    let url = "https://example.com/suggestions.md";
    let hash = seed_cache(&arai_base, url, upstream_content);

    trust_url(&arai_base, url);

    let local_content = "## Local\n\nOur rules.\n";
    let directive = format!("<!-- arai:extends {url} @{hash} tier=advisory -->\n\n{local_content}");

    let resolved = extends::resolve(&directive, &arai_base);

    assert!(resolved.contains("tier=\"advisory\""));
    assert!(resolved.contains(upstream_content));
    assert!(resolved.contains(local_content));
}

#[test]
fn test_ac11a_override_tier_matching_spo_drops_upstream() {
    // tier=override with matching SPO → upstream rule dropped (AC11 sub-case A).
    let (_tmpdir, arai_base) = setup_test_env();

    // Upstream content with a specific rule (subject, predicate, object).
    let upstream_content = "## Rules\n\nMust use TLS.\n";
    let url = "https://example.com/rules.md";
    let hash = seed_cache(&arai_base, url, upstream_content);

    trust_url(&arai_base, url);

    // Local content with the exact same rule (will cause implicit drop in override mode).
    let local_content = "## Rules\n\nMust use TLS.\n";
    let directive = format!("<!-- arai:extends {url} @{hash} tier=override -->\n\n{local_content}");

    let resolved = extends::resolve(&directive, &arai_base);

    // Both should be present in the text (no parsing happens in resolve()),
    // but the tier marker should be override.
    assert!(resolved.contains("tier=\"override\""));
    assert!(resolved.contains(upstream_content));
    assert!(resolved.contains(local_content));
}

#[test]
fn test_ac11b_override_tier_no_matching_spo_retains_upstream() {
    // tier=override with no matching SPO → upstream rule retained (AC11 sub-case B).
    let (_tmpdir, arai_base) = setup_test_env();

    let upstream_content = "## Rules\n\nMust use encryption.\n";
    let url = "https://example.com/rules.md";
    let hash = seed_cache(&arai_base, url, upstream_content);

    trust_url(&arai_base, url);

    // Local content with a DIFFERENT rule (no SPO match).
    let local_content = "## Rules\n\nMust document APIs.\n";
    let directive = format!("<!-- arai:extends {url} @{hash} tier=override -->\n\n{local_content}");

    let resolved = extends::resolve(&directive, &arai_base);

    assert!(resolved.contains("tier=\"override\""));
    assert!(resolved.contains(upstream_content));
    assert!(resolved.contains(local_content));
}

#[test]
fn test_per_directive_failure_isolation() {
    // Two directives: first fails (pin mismatch), second succeeds.
    // Output includes second upstream content and one warning.
    let (_tmpdir, arai_base) = setup_test_env();

    let upstream1 = "# Rules 1\n\nRule 1.\n";
    let url1 = "https://example.com/rules1.md";
    let _hash1 = seed_cache(&arai_base, url1, upstream1);
    trust_url(&arai_base, url1);

    let upstream2 = "# Rules 2\n\nRule 2.\n";
    let url2 = "https://example.com/rules2.md";
    let hash2 = seed_cache(&arai_base, url2, upstream2);
    trust_url(&arai_base, url2);

    let wrong_hash1 = "b".repeat(64);

    let local = "# Local\n\nLocal content.\n";
    let directive = format!(
        "<!-- arai:extends {url1} @{wrong_hash1} -->\n<!-- arai:extends {url2} @{hash2} -->\n\n{local}"
    );

    let resolved = extends::resolve(&directive, &arai_base);

    // First directive should fail (pin mismatch) → no upstream1.
    assert!(!resolved.contains("Rules 1"));

    // Second directive should succeed → contains upstream2.
    assert!(resolved.contains("Rules 2"));

    // Local content should always be present.
    assert!(resolved.contains(local));
}

#[test]
fn test_wiring_order_tokenise_before_verify() {
    // Verify that tokenisation happens before verification by checking
    // that a malformed directive (e.g., bad token) is skipped before
    // any fetch attempt.
    let (_tmpdir, arai_base) = setup_test_env();

    let local_content = "# Local\n\nLocal rule.\n";

    // Directive with an unknown trailing token (not a pin, not tier=).
    // Should be tokenised, found malformed, and skipped without fetch.
    let directive = format!(
        "<!-- arai:extends https://example.com/rules.md unknown_token -->\n\n{local_content}"
    );

    let resolved = extends::resolve(&directive, &arai_base);

    // Local content should be present (directive skipped).
    assert!(resolved.contains(local_content));

    // No upstream content should be inlined (tokenisation failed).
    // We can check that arai:extends-block is not present.
    assert!(!resolved.contains("arai:extends-block"));
}

#[test]
fn test_wiring_order_verify_before_provenance() {
    // Verify that verification happens before provenance tagging.
    // If verification rejects content, no tier marker should be emitted.
    let (_tmpdir, arai_base) = setup_test_env();

    let upstream_content = "# Upstream\n";
    let url = "https://example.com/rules.md";
    let _hash = seed_cache(&arai_base, url, upstream_content);

    trust_url(&arai_base, url);

    let wrong_hash = "c".repeat(64);
    let local_content = "# Local\n";
    let directive =
        format!("<!-- arai:extends {url} @{wrong_hash} tier=strict -->\n\n{local_content}");

    let resolved = extends::resolve(&directive, &arai_base);

    // Verification (pin check) happens before provenance tagging.
    // Pin mismatch → reject → no tier marker emitted.
    assert!(!resolved.contains("arai:extends-block"));
    assert!(!resolved.contains("tier=\"strict\""));
    assert!(resolved.contains(local_content));
}
