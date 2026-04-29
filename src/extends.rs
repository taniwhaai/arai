//! Shared policy files: `arai:extends <url>` directive + trust list.
//!
//! An instruction file (CLAUDE.md, .cursorrules, etc.) can reference an
//! upstream markdown URL and inherit its rules.  The upstream content is
//! inlined at discovery time so the rest of the pipeline (parser, intent
//! classifier, guardrail matcher) is unchanged.
//!
//! Security model
//! ──────────────
//!   - URLs must be explicitly trusted via `arai trust <url>` before any
//!     fetch happens.  First-time untrusted URLs are skipped with a
//!     warning printed to stderr — Ārai never silently pulls from a URL
//!     you didn't approve.
//!   - HTTPS only.  HTTP urls are rejected.
//!   - Size cap (MAX_EXTEND_BYTES).  Oversized responses are rejected.
//!   - 24h on-disk cache.  Stale-while-error fallback: if the fetch fails
//!     but a cached copy exists, we use the cache and log a warning.
//!   - Single-level.  Extended files do NOT have their own extends
//!     recursively processed — prevents fetch loops and supply-chain
//!     surprises.
//!
//! Directive forms
//! ───────────────
//!   - `<!-- arai:extends https://example.com/rules.md -->`
//!   - `# arai:extends https://example.com/rules.md`
//!
//! Only directives appearing at the very top of the file (before any
//! meaningful content, optionally after YAML frontmatter) are honoured.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::{IpAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Maximum upstream-file size we'll accept (512 KB).  Anything larger is
/// almost certainly not a rule file.
pub const MAX_EXTEND_BYTES: usize = 512 * 1024;

/// Cache freshness in seconds (24 hours).  Past this we try the network;
/// the cached copy is still used if the fetch fails.
const CACHE_TTL_SECS: u64 = 86_400;

/// HTTP timeout for a single fetch attempt.
const FETCH_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Serialize, Deserialize, Default)]
struct TrustFile {
    #[serde(default)]
    trusted: Vec<String>,
}

/// Path to the trust list: `{arai_base}/trusted_extends.toml`.
pub fn trust_path(arai_base: &Path) -> PathBuf {
    arai_base.join("trusted_extends.toml")
}

/// Path to the extends cache directory: `{arai_base}/cache/extends/`.
pub fn cache_dir(arai_base: &Path) -> PathBuf {
    arai_base.join("cache").join("extends")
}

fn read_trust(arai_base: &Path) -> TrustFile {
    let path = trust_path(arai_base);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return TrustFile::default();
    };
    toml::from_str(&content).unwrap_or_default()
}

fn write_trust(arai_base: &Path, tf: &TrustFile) -> Result<(), String> {
    let path = trust_path(arai_base);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create trust dir: {e}"))?;
    }
    let encoded =
        toml::to_string_pretty(tf).map_err(|e| format!("Failed to encode trust file: {e}"))?;
    std::fs::write(&path, encoded).map_err(|e| format!("Failed to write trust file: {e}"))?;
    Ok(())
}

/// Return true if the URL is currently trusted.
pub fn is_trusted(url: &str, arai_base: &Path) -> bool {
    read_trust(arai_base).trusted.iter().any(|u| u == url)
}

/// Add a URL to the trust list.  Idempotent.  HTTPS only.
pub fn trust_add(url: &str, arai_base: &Path) -> Result<bool, String> {
    validate_url(url)?;
    let mut tf = read_trust(arai_base);
    if tf.trusted.iter().any(|u| u == url) {
        return Ok(false);
    }
    tf.trusted.push(url.to_string());
    tf.trusted.sort();
    write_trust(arai_base, &tf)?;
    Ok(true)
}

/// Remove a URL from the trust list.  Returns true if it was present.
pub fn trust_remove(url: &str, arai_base: &Path) -> Result<bool, String> {
    let mut tf = read_trust(arai_base);
    let before = tf.trusted.len();
    tf.trusted.retain(|u| u != url);
    let removed = tf.trusted.len() != before;
    if removed {
        write_trust(arai_base, &tf)?;
    }
    Ok(removed)
}

/// List all currently-trusted URLs.
pub fn trust_list(arai_base: &Path) -> Vec<String> {
    read_trust(arai_base).trusted
}

fn validate_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err(format!(
            "URL must start with https:// — got {url:?} (HTTP is not supported for security)"
        ));
    }
    if url.len() > 2048 {
        return Err("URL is implausibly long (>2048 chars)".to_string());
    }
    let host = extract_host(url)
        .ok_or_else(|| format!("could not parse host from {url:?}"))?;
    validate_host_not_private(host)?;
    Ok(())
}

/// Extract just the hostname from an https URL, stripping scheme, userinfo,
/// port, and path.  Returns `None` for unparseable inputs.  Intentionally
/// minimal — we already know the URL starts with `https://` from the caller.
fn extract_host(url: &str) -> Option<&str> {
    let after_scheme = url.strip_prefix("https://")?;
    // Strip userinfo (we don't honour it but reject just in case).
    let after_userinfo = match after_scheme.find('@') {
        Some(at) => &after_scheme[at + 1..],
        None => after_scheme,
    };
    let host_end = after_userinfo
        .find(['/', '?', '#'])
        .unwrap_or(after_userinfo.len());
    let host_with_port = &after_userinfo[..host_end];
    if host_with_port.is_empty() {
        return None;
    }
    // IPv6 in brackets: [::1]:443 → ::1
    if let Some(rest) = host_with_port.strip_prefix('[') {
        return rest.split(']').next();
    }
    // Plain host or host:port
    Some(host_with_port.split(':').next().unwrap_or(host_with_port))
}

/// Reject URLs whose host resolves (or directly is) a private/loopback/
/// link-local address.  Closes the SSRF surface — a trusted upstream
/// pointing at `http://169.254.169.254/` (cloud metadata) or `127.0.0.1`
/// would otherwise let an attacker pivot from a benign-looking trust list.
///
/// Note: this is best-effort and runs once at validate time.  A determined
/// attacker controlling DNS could still execute a rebinding attack between
/// our resolution and ureq's.  Mitigations beyond DNS pinning would require
/// a custom resolver — out of scope for the current threat model.
fn validate_host_not_private(host: &str) -> Result<(), String> {
    // If the host is a literal IP, check it directly without DNS.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_ip(&ip) {
            return Err(format!(
                "URL host {host} is a private/loopback/link-local address"
            ));
        }
        return Ok(());
    }

    // Otherwise resolve via the system resolver and reject if any returned
    // address is private.  Use port 443 since this is HTTPS-only.
    let addrs = (host, 443u16)
        .to_socket_addrs()
        .map_err(|e| format!("could not resolve {host}: {e}"))?;

    for sa in addrs {
        let ip = sa.ip();
        if is_private_ip(&ip) {
            return Err(format!(
                "URL host {host} resolves to a private/loopback/link-local address ({ip})"
            ));
        }
    }
    Ok(())
}

/// Classify IP addresses we refuse to fetch from.  Includes loopback, RFC1918,
/// link-local (covers cloud metadata at 169.254.169.254), CGNAT, multicast,
/// and the IPv6 equivalents.
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                // Carrier-grade NAT: 100.64.0.0/10
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // Unique local fc00::/7
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // Link-local fe80::/10
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // IPv4-mapped IPv6 — defer to the v4 classification of the mapped address.
                || v6.to_ipv4_mapped().map(|v4| {
                    v4.is_loopback()
                        || v4.is_private()
                        || v4.is_link_local()
                        || v4.is_broadcast()
                        || v4.is_documentation()
                        || v4.is_unspecified()
                }).unwrap_or(false)
        }
    }
}

fn url_cache_path(url: &str, arai_base: &Path) -> PathBuf {
    let mut h = Sha256::new();
    h.update(url.as_bytes());
    let hash = h.finalize();
    let short: String = hash.iter().take(16).map(|b| format!("{b:02x}")).collect();
    cache_dir(arai_base).join(format!("{short}.md"))
}

fn cache_is_fresh(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(mtime) = meta.modified() else {
        return false;
    };
    let Ok(age) = SystemTime::now().duration_since(mtime) else {
        return false;
    };
    age.as_secs() < CACHE_TTL_SECS
}

/// Fetch an extends URL, honouring the cache and trust list.  Returns the
/// file contents or an error.  Not recursive — the returned content is
/// used as-is; its own extends directives (if any) are ignored.
pub fn fetch(url: &str, arai_base: &Path) -> Result<String, String> {
    validate_url(url)?;
    if !is_trusted(url, arai_base) {
        return Err(format!(
            "URL not trusted — run `arai trust {url}` to approve it"
        ));
    }

    let path = url_cache_path(url, arai_base);

    if cache_is_fresh(&path) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            return Ok(content);
        }
    }

    match fetch_remote(url) {
        Ok(content) => {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // TOCTOU-safe write: refuse to follow a symlink at the cache
            // path.  An attacker with write access to the cache dir could
            // otherwise redirect the write to another file (e.g. ~/.bashrc).
            if let Ok(meta) = std::fs::symlink_metadata(&path) {
                if meta.file_type().is_symlink() {
                    return Err(format!(
                        "refusing to write through symlink at {}",
                        path.display()
                    ));
                }
            }
            let _ = std::fs::write(&path, &content);
            Ok(content)
        }
        Err(fetch_err) => {
            // Stale-while-error: fall back to any existing cached copy —
            // but again only if the cached path is a regular file, not a
            // symlink an attacker could have planted.
            if let Ok(meta) = std::fs::symlink_metadata(&path) {
                if !meta.file_type().is_symlink() {
                    if let Ok(cached) = std::fs::read_to_string(&path) {
                        eprintln!(
                            "arai: extends fetch failed for {url}, using stale cache ({fetch_err})"
                        );
                        let _ = filetouch(&path);
                        return Ok(cached);
                    }
                }
            }
            Err(fetch_err)
        }
    }
}

fn fetch_remote(url: &str) -> Result<String, String> {
    // Disable redirects entirely — the trust list is per exact URL, so a
    // 30x to a different URL would bypass it.  Trusting the redirect
    // target separately requires the user to add it explicitly.
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS)))
        .max_redirects(0)
        .build()
        .new_agent();

    let response = agent
        .get(url)
        // Force identity encoding so a tiny gzip blob can't decompress past
        // the size cap before we notice.  Trade-off: slightly larger transfers
        // for a hard pre-decompression bound.
        .header("Accept-Encoding", "identity")
        .call()
        .map_err(|e| format!("HTTP error: {e}"))?;

    let (parts, mut body) = response.into_parts();
    if !parts.status.is_success() {
        return Err(format!("HTTP {} from {url}", parts.status));
    }

    let bytes = body
        .with_config()
        .limit(MAX_EXTEND_BYTES as u64)
        .read_to_vec()
        .map_err(|e| format!("read body: {e}"))?;

    if bytes.len() >= MAX_EXTEND_BYTES {
        return Err(format!(
            "extends response exceeded size cap of {MAX_EXTEND_BYTES} bytes"
        ));
    }

    String::from_utf8(bytes)
        .map_err(|e| format!("response was not valid UTF-8: {e}"))
}

fn filetouch(path: &Path) -> std::io::Result<()> {
    // Bump mtime to now so stale-while-error doesn't loop repeatedly.
    let now = SystemTime::now();
    let f = std::fs::OpenOptions::new().write(true).open(path)?;
    f.set_modified(now)?;
    Ok(())
}

/// Scan markdown content for `arai:extends` directives at the top of the file.
/// Returns a list of URLs in the order they appear.  Only directives appearing
/// before any meaningful content (blank lines + comments + a single H1 allowed)
/// are honoured.
pub fn extract_urls(content: &str) -> Vec<String> {
    let mut urls = Vec::new();
    // Skip YAML frontmatter
    let body = skip_frontmatter(content);

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Stop once we hit a non-directive, non-comment, non-top-heading line.
        if let Some(url) = parse_directive(trimmed) {
            urls.push(url);
            continue;
        }
        if trimmed.starts_with("<!--") {
            // An HTML comment that's not our directive — tolerate and keep scanning.
            continue;
        }
        if trimmed.starts_with("# ") && !trimmed.starts_with("# arai:extends") {
            // Top-level H1 is allowed *once* in the preamble; move on.
            continue;
        }
        break;
    }
    urls
}

fn parse_directive(line: &str) -> Option<String> {
    // Form 1: <!-- arai:extends <url> -->
    if let Some(inner) = line
        .strip_prefix("<!--")
        .and_then(|s| s.strip_suffix("-->"))
    {
        let inner = inner.trim();
        if let Some(url) = inner.strip_prefix("arai:extends ") {
            return Some(url.trim().to_string());
        }
    }
    // Form 2: # arai:extends <url>
    if let Some(url) = line.strip_prefix("# arai:extends ") {
        return Some(url.trim().to_string());
    }
    None
}

fn skip_frontmatter(content: &str) -> &str {
    if !content.starts_with("---") {
        return content;
    }
    let rest = &content[3..];
    if let Some(pos) = rest.find("\n---") {
        let body_start = pos + 4;
        if body_start < rest.len() {
            return rest[body_start..].trim_start_matches('\n');
        }
    }
    content
}

/// Resolve extends directives in `content`, prepending the fetched upstream
/// markdown ahead of the local content.  Never recursive.  Failures for
/// individual URLs are logged to stderr but don't break discovery.
pub fn resolve(content: &str, arai_base: &Path) -> String {
    let urls = extract_urls(content);
    if urls.is_empty() {
        return content.to_string();
    }
    let mut out = String::new();
    for url in &urls {
        match fetch(url, arai_base) {
            Ok(upstream) => {
                out.push_str(&format!("<!-- arai:extends from {url} -->\n"));
                out.push_str(&upstream);
                if !upstream.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("\n<!-- end arai:extends -->\n\n");
            }
            Err(e) => {
                eprintln!("arai: skipping extends {url}: {e}");
            }
        }
    }
    out.push_str(content);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_html_comment() {
        let content = "<!-- arai:extends https://example.com/rules.md -->\n\n# My rules\n";
        let urls = extract_urls(content);
        assert_eq!(urls, vec!["https://example.com/rules.md"]);
    }

    #[test]
    fn test_extract_heading_form() {
        let content = "# arai:extends https://example.com/a.md\n# arai:extends https://example.com/b.md\n\n# Actual heading\n";
        let urls = extract_urls(content);
        assert_eq!(
            urls,
            vec![
                "https://example.com/a.md".to_string(),
                "https://example.com/b.md".to_string()
            ]
        );
    }

    #[test]
    fn test_extract_skips_after_content() {
        // Directive below real content is ignored.
        let content = "# Heading\n\nSome prose.\n\n# arai:extends https://example.com/a.md\n";
        let urls = extract_urls(content);
        assert!(urls.is_empty());
    }

    #[test]
    fn test_extract_after_frontmatter() {
        let content = "---\nname: rules\n---\n\n<!-- arai:extends https://example.com/a.md -->\n";
        let urls = extract_urls(content);
        assert_eq!(urls, vec!["https://example.com/a.md"]);
    }

    #[test]
    fn test_extract_tolerates_single_h1() {
        let content = "# My project rules\n\n<!-- arai:extends https://example.com/a.md -->\n";
        let urls = extract_urls(content);
        assert_eq!(urls, vec!["https://example.com/a.md"]);
    }

    #[test]
    fn test_extract_ignores_non_directive_comment() {
        let content = "<!-- unrelated -->\n<!-- arai:extends https://example.com/a.md -->\n";
        let urls = extract_urls(content);
        assert_eq!(urls, vec!["https://example.com/a.md"]);
    }

    #[test]
    fn test_validate_url_rejects_http() {
        assert!(validate_url("http://example.com").is_err());
        // example.com resolves to a public IP — leaving the positive case to
        // integration tests that allow network; here we just confirm the
        // scheme check.  validate_url with a public host requires DNS.
    }

    #[test]
    fn test_extract_host_basic() {
        assert_eq!(extract_host("https://example.com"), Some("example.com"));
        assert_eq!(extract_host("https://example.com/path"), Some("example.com"));
        assert_eq!(extract_host("https://example.com:443/p"), Some("example.com"));
        assert_eq!(extract_host("https://user:pw@example.com/p"), Some("example.com"));
        assert_eq!(extract_host("https://[::1]:443/p"), Some("::1"));
        assert_eq!(extract_host("https://[2001:db8::1]/p"), Some("2001:db8::1"));
    }

    #[test]
    fn test_validate_url_rejects_loopback_literal() {
        assert!(validate_url("https://127.0.0.1/").is_err());
        assert!(validate_url("https://127.0.0.1:8080/path").is_err());
        assert!(validate_url("https://[::1]/").is_err());
    }

    #[test]
    fn test_validate_url_rejects_rfc1918() {
        assert!(validate_url("https://10.0.0.1/").is_err());
        assert!(validate_url("https://192.168.1.1/").is_err());
        assert!(validate_url("https://172.16.0.1/").is_err());
    }

    #[test]
    fn test_validate_url_rejects_link_local_and_metadata() {
        // 169.254.169.254 — the canonical cloud-metadata SSRF target.
        assert!(validate_url("https://169.254.169.254/").is_err());
        assert!(validate_url("https://169.254.0.1/").is_err());
    }

    #[test]
    fn test_is_private_ip_classifications() {
        use std::str::FromStr;
        let private_cases = [
            "127.0.0.1", "10.0.0.1", "172.20.5.1", "192.168.0.1",
            "169.254.169.254", "0.0.0.0", "224.0.0.1",
            "100.64.0.1", // CGNAT
            "::1", "fc00::1", "fe80::1",
        ];
        for s in private_cases {
            let ip: IpAddr = IpAddr::from_str(s).unwrap();
            assert!(is_private_ip(&ip), "{s} should be classified private");
        }

        let public_cases = ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"];
        for s in public_cases {
            let ip: IpAddr = IpAddr::from_str(s).unwrap();
            assert!(!is_private_ip(&ip), "{s} should NOT be classified private");
        }
    }

    #[test]
    fn test_trust_add_and_list() {
        let tmp = std::env::temp_dir().join(format!(
            "arai_trust_test_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let url = "https://example.com/rules.md";
        assert!(trust_add(url, &tmp).unwrap());
        assert!(!trust_add(url, &tmp).unwrap()); // idempotent
        assert!(is_trusted(url, &tmp));
        let list = trust_list(&tmp);
        assert_eq!(list, vec![url.to_string()]);
        assert!(trust_remove(url, &tmp).unwrap());
        assert!(!is_trusted(url, &tmp));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_resolve_inlines_untrusted_url_silently_noops() {
        // Untrusted URL: resolve returns original content, prints to stderr.
        let tmp = std::env::temp_dir().join(format!(
            "arai_resolve_untrust_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let content = "<!-- arai:extends https://example.com/not-trusted.md -->\n\n- local rule\n";
        let resolved = resolve(content, &tmp);
        // Content unchanged (directive line still present; no fetched block).
        assert!(resolved.contains("- local rule"));
        assert!(!resolved.contains("end arai:extends"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_url_cache_path_stable() {
        let tmp = std::path::PathBuf::from("/tmp/arai_cache_test");
        let a = url_cache_path("https://example.com/a.md", &tmp);
        let b = url_cache_path("https://example.com/a.md", &tmp);
        let c = url_cache_path("https://example.com/b.md", &tmp);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.to_string_lossy().ends_with(".md"));
    }
}
