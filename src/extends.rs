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
//!
//! ## Directive tokenisation
//!
//! A directive line may carry optional trailing tokens after the URL:
//!   - `@<pin>` — a 64-character lowercase hex SHA-256 content pin.
//!   - `tier=strict|advisory|override` — the tier for the upstream block.
//!
//! [`classify_directive`] parses a raw directive body (the text after the
//! `arai:extends` marker and its leading whitespace have been stripped) into
//! either a [`ParsedDirective`] or a [`MalformedDirective`].  A bare
//! `arai:extends <url>` (no trailing tokens) produces a `ParsedDirective`
//! with `pin` absent and `tier` absent — identical to the pre-slice behaviour.

use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Deserializer, Serialize};
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

// ─── Directive tokenisation types ────────────────────────────────────────────

/// How strongly an upstream block's rules bind relative to local rules.
///
/// Only `Strict`, `Advisory`, and `Override` are valid written values in a
/// directive.  `Peer` is the implicit default when `tier=` is absent; it is
/// never written in a directive.
///
/// `Peer` is part of the public contract for downstream modules (tier-provenance,
/// resolve-composition) even though this module never constructs it directly —
/// those modules apply the Peer default when `ParsedDirective.tier` is absent.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[allow(dead_code)]
pub enum Tier {
    /// Upstream rules whose subject matches a local rule take precedence.
    Strict,
    /// Upstream rules are deprioritised by the ranker.
    Advisory,
    /// Local rules may implicitly drop matching upstream rules by triple-equality.
    Override,
    /// No shadowing change, no deprioritisation, no implicit drop.  Applied when
    /// `tier=` is absent.  Never written in a directive.
    Peer,
}

/// The structured result of successfully classifying one `arai:extends`
/// directive line.
///
/// Produced by [`classify_directive`] on the success path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDirective {
    /// The URL token, exactly as it appears in the directive.
    pub url: String,
    /// The 64-character lowercase hex SHA-256 content pin, if present.
    /// The leading `@` sigil is stripped.
    pub pin: Option<String>,
    /// The declared tier for this upstream block, if present.
    /// When absent, callers apply the [`Tier::Peer`] default.
    pub tier: Option<Tier>,
}

/// The outcome produced by [`classify_directive`] when a directive line cannot
/// be classified.
///
/// The caller is responsible for emitting a stderr warning and skipping the
/// directive; local content is preserved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MalformedDirective {
    /// The whitespace-separated token that caused the failure.
    pub offending_token: String,
    /// A short human-readable description of why classification failed.
    pub reason: String,
}

/// Classify a raw `arai:extends` directive body into a [`ParsedDirective`] or
/// [`MalformedDirective`].
///
/// # Input
///
/// `directive_line` is the text of the directive **after** the `arai:extends`
/// marker has been stripped.  Both the `#` form and the `<!-- ... -->` form
/// are accepted: if the input still carries `<!-- ... -->` comment delimiters
/// they are stripped before tokenisation.
///
/// # Token grammar
///
/// ```text
/// <directive_line> ::= <url> (<ws> <token>)*
/// <token>          ::= "@" <64hex>         # content-pin
///                    | "tier=" <tier_val>  # tier declaration
/// <tier_val>       ::= "strict" | "advisory" | "override"
/// ```
///
/// An `@` character **inside** the URL is not a pin token — it is
/// whitespace-delimited tokens only.
///
/// # Fail-closed rules (BINDING: AC12_duplicate_token)
///
/// - Two or more `@<pin>` tokens → [`MalformedDirective`] naming the second.
/// - Two or more `tier=` tokens → [`MalformedDirective`] naming the second.
/// - Any token that does not match either shape → [`MalformedDirective`].
/// - A `tier=` token with an unknown value → [`MalformedDirective`].
/// - An `@`-prefixed token whose remainder is not exactly 64 lowercase hex
///   characters → [`MalformedDirective`].
///
/// # Backward-compatibility invariant (AC1)
///
/// A directive with no trailing tokens (bare `arai:extends <url>`) produces a
/// `ParsedDirective` with `pin` absent and `tier` absent.  No new code path is
/// entered.
pub fn classify_directive(directive_line: &str) -> Result<ParsedDirective, MalformedDirective> {
    // Strip HTML comment delimiters if present so both surface forms are
    // handled identically.
    let inner = if let Some(stripped) = directive_line
        .trim()
        .strip_prefix("<!--")
        .and_then(|s| s.strip_suffix("-->"))
    {
        stripped.trim()
    } else {
        directive_line.trim()
    };

    // Strip the "arai:extends " prefix if it is still present (callers may
    // pass the already-stripped body, but we tolerate the full prefix too).
    let body = if let Some(rest) = inner.strip_prefix("arai:extends ") {
        rest.trim_start()
    } else {
        inner
    };

    let mut tokens = body.split_whitespace();

    // First token must be the URL.
    let url = match tokens.next() {
        Some(u) => u.to_string(),
        None => {
            return Err(MalformedDirective {
                offending_token: String::new(),
                reason: "directive body is empty — no URL found".to_string(),
            });
        }
    };

    let mut pin: Option<String> = None;
    let mut tier: Option<Tier> = None;

    for token in tokens {
        if let Some(hex_part) = token.strip_prefix('@') {
            // Pin token: @ followed by exactly 64 lowercase hex chars.
            if pin.is_some() {
                // AC12d: duplicate pin token → fail-closed.
                return Err(MalformedDirective {
                    offending_token: token.to_string(),
                    reason: format!("duplicate @pin token: {token}"),
                });
            }
            if is_valid_pin_hex(hex_part) {
                pin = Some(hex_part.to_ascii_lowercase());
            } else {
                return Err(MalformedDirective {
                    offending_token: token.to_string(),
                    reason: format!("malformed pin: {token} — expected @<64-char lowercase hex>"),
                });
            }
        } else if let Some(value) = token.strip_prefix("tier=") {
            // Tier token: tier=strict|advisory|override
            if tier.is_some() {
                // AC12e: duplicate tier= token → fail-closed.
                return Err(MalformedDirective {
                    offending_token: token.to_string(),
                    reason: format!("duplicate tier= token: {token}"),
                });
            }
            match value {
                "strict" => tier = Some(Tier::Strict),
                "advisory" => tier = Some(Tier::Advisory),
                "override" => tier = Some(Tier::Override),
                _ => {
                    return Err(MalformedDirective {
                        offending_token: token.to_string(),
                        reason: format!(
                            "unknown tier value {value:?} — expected strict, advisory, or override"
                        ),
                    });
                }
            }
        } else {
            // AC12a: unknown trailing token → fail-closed.
            return Err(MalformedDirective {
                offending_token: token.to_string(),
                reason: format!("unknown directive token: {token}"),
            });
        }
    }

    Ok(ParsedDirective { url, pin, tier })
}

/// Returns `true` when `hex` is exactly 64 characters all in `[0-9a-fA-F]`.
/// Uppercase letters are accepted here and normalised to lowercase by the
/// caller (per the cross-cutting hex-encoding convention).
fn is_valid_pin_hex(hex: &str) -> bool {
    hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())
}

// ─── End directive tokenisation types ────────────────────────────────────────

// ─── Trust file types ────────────────────────────────────────────────────────

/// A per-URL trust record in the trust file.
///
/// `pubkey` is an optional 64-character lowercase hex string encoding a
/// 32-byte ed25519 public key.  When absent, no signature check is performed
/// for this URL.  When present, a detached ed25519 signature over the fetched
/// content is required (fetched from `<url>.sig`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustEntry {
    /// The trusted URL.
    pub url: String,
    /// Optional hex-encoded ed25519 public key (64 chars = 32 bytes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<String>,
    /// Optional name of an environment variable holding a bearer token for
    /// this URL.  Only the variable *name* is stored — the secret itself
    /// never touches disk, logs, audit, or telemetry.  When set and the
    /// variable is non-empty at fetch time, requests to this exact URL (and
    /// its `<url>.sig` sidecar) carry `Authorization: Bearer <token>`.
    /// Redirects are already hard-disabled on the fetch path, so the header
    /// can never follow a 30x to another host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bearer_env: Option<String>,
}

/// The in-memory representation of `trusted_extends.toml`.
///
/// Supports a dual on-disk form:
///   - Legacy: `trusted = ["https://..."]` — each string maps to a
///     `TrustEntry` with `pubkey` absent.
///   - New: `trusted = [{url = "https://...", pubkey = "..."}]`
///
/// Both forms deserialise to the same `Vec<TrustEntry>`.  The serialiser
/// always writes the new per-entry form.  A legacy file is NOT rewritten on
/// read.
#[derive(Debug, Default, Serialize)]
pub struct TrustFile {
    /// Ordered list of trust entries.
    #[serde(default)]
    pub trusted: Vec<TrustEntry>,
}

/// Helper enum for dual-form deserialisation: each element of the `trusted`
/// array can be either a plain URL string (legacy) or an inline table (new).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TrustEntryOrString {
    /// New form: inline table with `url` and optional `pubkey`.
    Entry(TrustEntry),
    /// Legacy form: a bare URL string.
    Url(String),
}

impl From<TrustEntryOrString> for TrustEntry {
    fn from(v: TrustEntryOrString) -> Self {
        match v {
            TrustEntryOrString::Entry(e) => e,
            TrustEntryOrString::Url(u) => TrustEntry {
                url: u,
                pubkey: None,
                bearer_env: None,
            },
        }
    }
}

/// Newtype wrapper so we can implement a custom `Deserialize` for `TrustFile`
/// that accepts both the legacy and new on-disk forms.
#[derive(Deserialize)]
struct TrustFileRaw {
    #[serde(default)]
    trusted: Vec<TrustEntryOrString>,
}

impl<'de> Deserialize<'de> for TrustFile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = TrustFileRaw::deserialize(deserializer)?;
        let trusted = raw.trusted.into_iter().map(TrustEntry::from).collect();
        Ok(TrustFile { trusted })
    }
}

// ─── End trust file types ─────────────────────────────────────────────────────

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
        std::fs::create_dir_all(parent).map_err(|e| format!("Could not create trust dir: {e}"))?;
    }
    let encoded =
        toml::to_string_pretty(tf).map_err(|e| format!("Could not encode trust file: {e}"))?;
    std::fs::write(&path, encoded).map_err(|e| format!("Could not write trust file: {e}"))?;
    Ok(())
}

/// Return the `TrustEntry` for a URL if it is currently trusted.
fn find_trust_entry(url: &str, arai_base: &Path) -> Option<TrustEntry> {
    read_trust(arai_base)
        .trusted
        .into_iter()
        .find(|e| e.url == url)
}

/// Return true if the URL is currently trusted.
pub fn is_trusted(url: &str, arai_base: &Path) -> bool {
    find_trust_entry(url, arai_base).is_some()
}

/// Add a URL to the trust list.  Idempotent.  HTTPS only.
/// `pubkey` is an optional 64-character lowercase hex string.
/// `bearer_env` is an optional environment-variable *name* holding a bearer
/// token for this URL — the secret itself is never written anywhere.
pub fn trust_add(
    url: &str,
    arai_base: &Path,
    pubkey: Option<&str>,
    bearer_env: Option<&str>,
) -> Result<bool, String> {
    validate_url(url)?;
    if let Some(pk) = pubkey {
        validate_pubkey_hex(pk)?;
    }
    if let Some(name) = bearer_env {
        validate_env_var_name(name)?;
    }
    let mut tf = read_trust(arai_base);
    if let Some(existing) = tf.trusted.iter_mut().find(|e| e.url == url) {
        // URL already present; update pubkey/bearer_env only if changed.
        let new_pk = pubkey.map(|s| s.to_ascii_lowercase());
        let new_bearer = bearer_env.map(|s| s.to_string());
        if existing.pubkey == new_pk && existing.bearer_env == new_bearer {
            return Ok(false); // idempotent: no change
        }
        existing.pubkey = new_pk;
        existing.bearer_env = new_bearer;
        tf.trusted.sort_by(|a, b| a.url.cmp(&b.url));
        write_trust(arai_base, &tf)?;
        return Ok(true);
    }
    tf.trusted.push(TrustEntry {
        url: url.to_string(),
        pubkey: pubkey.map(|s| s.to_ascii_lowercase()),
        bearer_env: bearer_env.map(|s| s.to_string()),
    });
    tf.trusted.sort_by(|a, b| a.url.cmp(&b.url));
    write_trust(arai_base, &tf)?;
    Ok(true)
}

/// Validate an environment-variable name for `bearer_env`.  Conservative
/// POSIX shape — a name that fails this was almost certainly a pasted token
/// rather than a variable name, and rejecting it keeps secrets out of the
/// trust file.  The error message deliberately does not echo the value for
/// the same reason.
fn validate_env_var_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let head_ok = chars
        .next()
        .map(|c| c.is_ascii_alphabetic() || c == '_')
        .unwrap_or(false);
    if !head_ok || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(
            "invalid --bearer-env: must be an environment variable NAME \
             ([A-Za-z_][A-Za-z0-9_]*), not the token itself"
                .to_string(),
        );
    }
    Ok(())
}

/// Resolve the bearer token for a trust entry from the environment.
/// Pure with respect to the environment via `env_lookup` so the decision
/// table is unit-testable.  Returns `None` when the entry has no
/// `bearer_env` configured or the variable is unset/empty.
fn resolve_bearer_with(
    entry: &TrustEntry,
    env_lookup: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let name = entry.bearer_env.as_deref()?;
    env_lookup(name).filter(|v| !v.is_empty())
}

/// Remove every occurrence of `secret` from a message before it can reach
/// stderr or an `Err` return.  Belt-and-braces: none of our own messages
/// interpolate the token, but transport-layer errors (`ureq`) are outside
/// our control and this makes the hygiene guarantee unconditional.
fn scrub_secret(msg: String, secret: Option<&str>) -> String {
    match secret {
        Some(s) if !s.is_empty() => msg.replace(s, "[redacted]"),
        _ => msg,
    }
}

/// Remove a URL from the trust list.  Returns true if it was present.
pub fn trust_remove(url: &str, arai_base: &Path) -> Result<bool, String> {
    let mut tf = read_trust(arai_base);
    let before = tf.trusted.len();
    tf.trusted.retain(|e| e.url != url);
    let removed = tf.trusted.len() != before;
    if removed {
        write_trust(arai_base, &tf)?;
    }
    Ok(removed)
}

/// List all currently-trusted entries.
pub fn trust_list_entries(arai_base: &Path) -> Vec<TrustEntry> {
    read_trust(arai_base).trusted
}

/// List all currently-trusted URLs (for backward-compat with callers that only
/// need the URL string).
pub fn trust_list(arai_base: &Path) -> Vec<TrustEntry> {
    trust_list_entries(arai_base)
}

/// Validate that a pubkey string is a 64-character lowercase hex string.
/// Returns an error string if invalid.
fn validate_pubkey_hex(hex: &str) -> Result<(), String> {
    let lower = hex.to_ascii_lowercase();
    if lower.len() != 64 || !lower.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "pubkey must be a 64-character hex string (got {} chars)",
            hex.len()
        ));
    }
    // Also verify it decodes to a valid ed25519 public key.
    decode_verifying_key(&lower)?;
    Ok(())
}

/// Decode a hex string into bytes.  Returns an error if any character is not
/// a valid hex digit or if the string has an odd length.
fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, String> {
    if !hex.len().is_multiple_of(2) {
        return Err(format!("hex string has odd length: {}", hex.len()));
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| format!("invalid hex byte at offset {i}: {:?}", &hex[i..i + 2]))
        })
        .collect()
}

/// Decode a 64-char lowercase hex string to a `VerifyingKey`.
/// Returns an error string on any failure (malformed hex, invalid key bytes).
fn decode_verifying_key(hex: &str) -> Result<VerifyingKey, String> {
    let bytes = hex_to_bytes(hex).map_err(|e| format!("malformed pubkey hex for ed25519: {e}"))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "pubkey must decode to exactly 32 bytes".to_string())?;
    VerifyingKey::from_bytes(&arr).map_err(|e| format!("invalid ed25519 public key: {e}"))
}

// ─── Content verification ─────────────────────────────────────────────────────

/// Verify obtained content against the configured pin and/or ed25519 signature.
///
/// # Arguments
///
/// - `url`: the upstream URL (used for the sidecar URL and warning messages)
/// - `pin`: optional 64-char lowercase hex SHA-256 pin from the `ParsedDirective`
/// - `obtained_content`: the upstream content bytes to verify
/// - `entry`: the `TrustEntry` for this URL (supplies optional pubkey)
/// - `fetch_sig`: callback to fetch the `<url>.sig` sidecar; receives the sidecar
///   URL as a `&str` and returns `Result<Vec<u8>, String>`.  Used for testability.
///
/// # Returns
///
/// `Ok(true)` when all configured checks pass (or when no checks are configured).
/// `Err(String)` with a human-readable warning on any failure.
///
/// # Backward-compatibility
///
/// When `pin` is absent **and** `entry.pubkey` is absent, no checks run and
/// the function always returns `Ok(true)`.  Behaviour is byte-identical to
/// before this function existed.
pub fn verify_content(
    url: &str,
    pin: Option<&str>,
    obtained_content: &[u8],
    entry: &TrustEntry,
    fetch_sig: impl Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<bool, String> {
    // ── Pin check ──────────────────────────────────────────────────────────
    if let Some(expected_pin) = pin {
        let actual = bytes_sha256_hex(obtained_content);
        let expected_lower = expected_pin.to_ascii_lowercase();
        if actual != expected_lower {
            return Err(format!(
                "arai: extends content for {url} failed pin check \
                 (expected {}, got {})",
                short_hash(&expected_lower),
                short_hash(&actual)
            ));
        }
    }

    // ── Signature check (only when pubkey configured) ─────────────────────
    if let Some(pubkey_hex) = &entry.pubkey {
        // Fail-closed: a malformed key must never silently downgrade to no-check.
        let verifying_key = decode_verifying_key(pubkey_hex)
            .map_err(|e| format!("arai: extends for {url} has malformed configured pubkey: {e}"))?;

        // Fetch the detached signature sidecar.
        let sig_url = format!("{url}.sig");
        let sig_bytes = fetch_sig(&sig_url).map_err(|e| {
            format!(
                "arai: extends for {url} missing or unreachable signature sidecar \
                 ({sig_url}): {e}"
            )
        })?;

        // Parse the sidecar: raw 64 bytes or hex-encoded (with optional
        // trailing whitespace/newlines).
        let sig_raw = if sig_bytes.len() == 64 {
            sig_bytes.clone()
        } else {
            // Try treating the sidecar as a hex-encoded signature (128 hex
            // chars = 64 bytes), optionally followed by whitespace.
            let maybe_hex = std::str::from_utf8(&sig_bytes)
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if maybe_hex.len() == 128 && maybe_hex.chars().all(|c| c.is_ascii_hexdigit()) {
                hex_to_bytes(&maybe_hex).map_err(|e| {
                    format!("arai: signature sidecar for {url} is not valid hex: {e}")
                })?
            } else {
                return Err(format!(
                    "arai: signature sidecar for {url} has unexpected length \
                     ({} bytes — expected 64 raw bytes or 128 hex chars)",
                    sig_bytes.len()
                ));
            }
        };

        let sig_arr: [u8; 64] = sig_raw
            .try_into()
            .map_err(|_| format!("arai: signature sidecar for {url} did not decode to 64 bytes"))?;
        let sig = Signature::from_bytes(&sig_arr);

        use ed25519_dalek::Verifier;
        verifying_key.verify(obtained_content, &sig).map_err(|_| {
            format!(
                "arai: extends for {url} failed ed25519 signature verification — \
                 content may have been tampered with"
            )
        })?;
    }

    Ok(true)
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
    let host = extract_host(url).ok_or_else(|| format!("could not parse host from {url:?}"))?;
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

/// Sidecar SHA-256 file path: `<cache>/<hash>.md.sha256`.  Written alongside
/// the cached content so a tampered cache file is detected before its rules
/// reach the parser.  Without this, an attacker with write access to the
/// cache directory could swap a cached policy out from under the trust list
/// (the URL is still trusted; the content at rest is not).
fn url_cache_sig_path(url: &str, arai_base: &Path) -> PathBuf {
    let primary = url_cache_path(url, arai_base);
    let mut s = primary.into_os_string();
    s.push(".sha256");
    PathBuf::from(s)
}

fn content_sha256_hex(content: &str) -> String {
    bytes_sha256_hex(content.as_bytes())
}

/// Compute the SHA-256 hash of raw bytes and return it as a lowercase hex string.
fn bytes_sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    let hash = h.finalize();
    hash.iter().map(|b| format!("{b:02x}")).collect()
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
///
/// `pin` is the optional 64-char lowercase hex SHA-256 content pin from the
/// `arai:extends` directive.  When present, the fetched/cached content is
/// checked against it (and against an ed25519 signature when the URL has a
/// configured public key in the trust file).
///
/// When `pin` is absent and the URL has no configured pubkey the function
/// behaves byte-identically to its pre-verification incarnation.
pub fn fetch(url: &str, pin: Option<&str>, arai_base: &Path) -> Result<String, String> {
    validate_url(url)?;
    if !is_trusted(url, arai_base) {
        return Err(format!(
            "URL not trusted — run `arai trust {url}` to approve it"
        ));
    }
    // Unwrap is safe: is_trusted just confirmed the entry exists.
    let entry = find_trust_entry(url, arai_base).unwrap();

    // Resolve the per-URL bearer token (if configured) from the environment.
    // The token is sent ONLY to this exact trusted URL and its `.sig`
    // sidecar — never logged, never audited, and redirects are disabled so
    // it cannot leak to another host via a 30x.
    let bearer = resolve_bearer_with(&entry, |n| std::env::var(n).ok());
    if bearer.is_none() {
        if let Some(name) = entry.bearer_env.as_deref() {
            eprintln!(
                "arai: bearer env var {name} is unset or empty; fetching {url} unauthenticated"
            );
        }
    }
    let fetch_sig = |sig_url: &str| fetch_sig_remote(sig_url, bearer.as_deref());

    let path = url_cache_path(url, arai_base);
    let sig_path = url_cache_sig_path(url, arai_base);

    if cache_is_fresh(&path) {
        if let Some(content) = read_cache_verified(&path, &sig_path, url) {
            // Run pin + signature verification on the cached content.
            // AC4: pin check runs on the stale-cache path too.
            verify_content(url, pin, content.as_bytes(), &entry, fetch_sig).map_err(|e| {
                eprintln!("{e}");
                e
            })?;
            return Ok(content);
        }
        // Verification failed (or sidecar missing on a chain-aware install) —
        // fall through and re-fetch.  We deliberately do NOT silently use a
        // mismatched cache file; that's the whole point of the signature.
    }

    match fetch_remote(url, bearer.as_deref()) {
        Ok(content) => {
            // Pin + signature check on freshly-fetched content before we cache it.
            verify_content(url, pin, content.as_bytes(), &entry, fetch_sig).map_err(|e| {
                eprintln!("{e}");
                e
            })?;

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
            if let Ok(meta) = std::fs::symlink_metadata(&sig_path) {
                if meta.file_type().is_symlink() {
                    return Err(format!(
                        "refusing to write through symlink at {}",
                        sig_path.display()
                    ));
                }
            }
            let _ = std::fs::write(&path, &content);
            let _ = std::fs::write(&sig_path, content_sha256_hex(&content));
            Ok(content)
        }
        Err(fetch_err) => {
            // Stale-while-error: fall back to any existing cached copy, but
            // only if it still matches its sidecar signature.  Silently
            // accepting an unverified stale copy would let cache tampering
            // outlive the TTL.
            if let Some(cached) = read_cache_verified(&path, &sig_path, url) {
                // AC4: pin + signature check on stale-cache fallback too.
                match verify_content(url, pin, cached.as_bytes(), &entry, fetch_sig) {
                    Ok(_) => {
                        eprintln!(
                            "arai: extends fetch failed for {url}, using verified stale cache ({fetch_err})"
                        );
                        let _ = filetouch(&path);
                        return Ok(cached);
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        return Err(e);
                    }
                }
            }
            Err(fetch_err)
        }
    }
}

/// Fetch the signature sidecar for a URL using the existing HTTPS fetch posture.
/// Used as the default `fetch_sig` callback in the live code path.
/// `bearer` is the resolved token for the *parent* trusted URL — the sidecar
/// lives at `<url>.sig` on the same origin, so a private policy source
/// protects both with the same credential.
fn fetch_sig_remote(sig_url: &str, bearer: Option<&str>) -> Result<Vec<u8>, String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS)))
        .max_redirects(0)
        .build()
        .new_agent();

    let mut request = agent.get(sig_url).header("Accept-Encoding", "identity");
    if let Some(token) = bearer {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let response = request
        .call()
        .map_err(|e| scrub_secret(format!("HTTP error: {e}"), bearer))?;

    let (parts, mut body) = response.into_parts();
    if !parts.status.is_success() {
        return Err(format!("HTTP {} from {sig_url}", parts.status));
    }

    // Signature sidecar is at most 128 bytes (64-byte raw sig or 128-char hex).
    body.with_config()
        .limit(256)
        .read_to_vec()
        .map_err(|e| scrub_secret(format!("read body: {e}"), bearer))
}

/// Read a cached upstream policy and verify its SHA-256 against the sidecar.
/// Returns `Some(content)` only when both the content and the sidecar are
/// regular files (not symlinks) and the hash matches.  Sidecar miss or
/// mismatch is treated the same as a missing cache — the caller decides
/// whether to fetch or surface the error.  Mismatches go to stderr so the
/// user sees the cause when troubleshooting a refused fetch.
fn read_cache_verified(path: &Path, sig_path: &Path, url: &str) -> Option<String> {
    // Reject symlinks at either path.  An attacker who could place a symlink
    // here would otherwise sidestep the signature check by pointing at a
    // file with a matching pre-computed sidecar.
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return None;
        }
    } else {
        return None;
    }
    if let Ok(meta) = std::fs::symlink_metadata(sig_path) {
        if meta.file_type().is_symlink() {
            return None;
        }
    } else {
        // No sidecar — treat as cache miss.  Existing pre-signature caches
        // will fall into this branch once and be re-fetched, which writes a
        // sidecar on the next successful fetch.
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    let expected = std::fs::read_to_string(sig_path).ok()?;
    let expected = expected.trim();
    let actual = content_sha256_hex(&content);
    if actual != expected {
        eprintln!(
            "arai: cached extends for {url} failed signature check (expected {}, got {}), refusing to use",
            short_hash(expected),
            short_hash(&actual)
        );
        return None;
    }
    Some(content)
}

fn short_hash(h: &str) -> String {
    h.chars().take(12).collect()
}

fn fetch_remote(url: &str, bearer: Option<&str>) -> Result<String, String> {
    // Disable redirects entirely — the trust list is per exact URL, so a
    // 30x to a different URL would bypass it.  Trusting the redirect
    // target separately requires the user to add it explicitly.  This is
    // also what guarantees a configured bearer token is only ever sent to
    // the exact trusted URL: there is no follow-up request to leak it to.
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS)))
        .max_redirects(0)
        .build()
        .new_agent();

    let mut request = agent
        .get(url)
        // Force identity encoding so a tiny gzip blob can't decompress past
        // the size cap before we notice.  Trade-off: slightly larger transfers
        // for a hard pre-decompression bound.
        .header("Accept-Encoding", "identity");
    if let Some(token) = bearer {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let response = request
        .call()
        // Transport errors come from outside our control — scrub the token
        // in case a library error ever echoes request headers.
        .map_err(|e| scrub_secret(format!("HTTP error: {e}"), bearer))?;

    let (parts, mut body) = response.into_parts();
    if !parts.status.is_success() {
        return Err(format!("HTTP {} from {url}", parts.status));
    }

    let bytes = body
        .with_config()
        .limit(MAX_EXTEND_BYTES as u64)
        .read_to_vec()
        .map_err(|e| scrub_secret(format!("read body: {e}"), bearer))?;

    if bytes.len() >= MAX_EXTEND_BYTES {
        return Err(format!(
            "extends response exceeded size cap of {MAX_EXTEND_BYTES} bytes"
        ));
    }

    String::from_utf8(bytes).map_err(|e| format!("response was not valid UTF-8: {e}"))
}

fn filetouch(path: &Path) -> std::io::Result<()> {
    // Bump mtime to now so stale-while-error doesn't loop repeatedly.
    let now = SystemTime::now();
    let f = std::fs::OpenOptions::new().write(true).open(path)?;
    f.set_modified(now)?;
    Ok(())
}

/// Scan markdown content for `arai:extends` directives at the top of the file.
/// Returns a list of `ParsedDirective`s (carrying URL, pin, and tier) in the
/// order they appear.  Only directives appearing before any meaningful content
/// (blank lines + comments + a single H1 allowed) are honoured.
pub fn extract_directives(content: &str) -> Vec<ParsedDirective> {
    let mut directives = Vec::new();
    let body = skip_frontmatter(content);

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(body) = parse_directive_body(trimmed) {
            match classify_directive(body) {
                Ok(pd) => {
                    directives.push(pd);
                    continue;
                }
                Err(bad) => {
                    eprintln!(
                        "arai: malformed extends directive — {}: {}",
                        bad.offending_token, bad.reason
                    );
                    // A malformed directive is treated as a stop-word — stop scanning.
                    break;
                }
            }
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
    directives
}

/// Scan markdown content for `arai:extends` directives at the top of the file.
/// Returns a list of URLs in the order they appear.  Only directives appearing
/// before any meaningful content (blank lines + comments + a single H1 allowed)
/// are honoured.
///
/// This is a convenience wrapper over [`extract_directives`] for callers that
/// only need URL strings.  For pin- and tier-aware processing, use
/// [`extract_directives`] directly.
#[allow(dead_code)]
pub fn extract_urls(content: &str) -> Vec<String> {
    extract_directives(content)
        .into_iter()
        .map(|pd| pd.url)
        .collect()
}

/// Extract the directive body (text after the `arai:extends` marker) from a
/// single markdown line, for both directive surface forms.  Returns `None` if
/// the line is not a directive.
fn parse_directive_body(line: &str) -> Option<&str> {
    // Form 1: <!-- arai:extends <body> -->
    if let Some(inner) = line
        .strip_prefix("<!--")
        .and_then(|s| s.strip_suffix("-->"))
    {
        let inner = inner.trim();
        if let Some(body) = inner.strip_prefix("arai:extends") {
            return Some(body.trim_start());
        }
    }
    // Form 2: # arai:extends <body>
    if let Some(body) = line.strip_prefix("# arai:extends") {
        return Some(body.trim_start());
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
///
/// When a directive carries a `tier=` token, the tier and source URL are
/// embedded in the block-start comment so downstream rule extraction can stamp
/// per-rule provenance without a second parse of the directives.  The marker
/// format is:
/// ```ignore
/// <!-- arai:extends-block url="<url>" tier="<tier>" -->
/// ```
/// Rules extracted from this block are tagged with the tier and URL.  Existing
/// `<!-- arai:extends from <url> -->` style markers (without tier) remain for
/// backward-compat; the new `arai:extends-block` form is additive.
pub fn resolve(content: &str, arai_base: &Path) -> String {
    let directives = extract_directives(content);
    if directives.is_empty() {
        return content.to_string();
    }
    let mut out = String::new();
    for pd in &directives {
        let url = &pd.url;
        let pin = pd.pin.as_deref();
        match fetch(url, pin, arai_base) {
            Ok(upstream) => {
                // Emit the tier-annotated block-start marker.  The tier
                // defaults to "peer" when absent (AC1 backward-compat).
                let tier_str = match &pd.tier {
                    Some(Tier::Strict) => "strict",
                    Some(Tier::Advisory) => "advisory",
                    Some(Tier::Override) => "override",
                    Some(Tier::Peer) | None => "peer",
                };
                out.push_str(&format!(
                    "<!-- arai:extends-block url=\"{url}\" tier=\"{tier_str}\" -->\n"
                ));
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
        assert_eq!(
            extract_host("https://example.com/path"),
            Some("example.com")
        );
        assert_eq!(
            extract_host("https://example.com:443/p"),
            Some("example.com")
        );
        assert_eq!(
            extract_host("https://user:pw@example.com/p"),
            Some("example.com")
        );
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
            "127.0.0.1",
            "10.0.0.1",
            "172.20.5.1",
            "192.168.0.1",
            "169.254.169.254",
            "0.0.0.0",
            "224.0.0.1",
            "100.64.0.1", // CGNAT
            "::1",
            "fc00::1",
            "fe80::1",
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
        let tmp = std::env::temp_dir().join(format!("arai_trust_test_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let url = "https://example.com/rules.md";
        assert!(trust_add(url, &tmp, None, None).unwrap());
        assert!(!trust_add(url, &tmp, None, None).unwrap()); // idempotent
        assert!(is_trusted(url, &tmp));
        let list = trust_list(&tmp);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].url, url);
        assert!(list[0].pubkey.is_none());
        assert!(trust_remove(url, &tmp).unwrap());
        assert!(!is_trusted(url, &tmp));
        std::fs::remove_dir_all(&tmp).ok();
    }

    // ── #150: authenticated arai:extends ─────────────────────────────────────

    #[test]
    fn bearer_env_round_trips_and_never_stores_the_token() {
        let tmp = std::env::temp_dir().join(format!("arai_bearer_rt_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let url = "https://example.com/org-rules.md";
        assert!(trust_add(url, &tmp, None, Some("ARAI_EXTENDS_TOKEN")).unwrap());
        // Round-trip: the entry carries the variable name.
        let entry = find_trust_entry(url, &tmp).unwrap();
        assert_eq!(entry.bearer_env.as_deref(), Some("ARAI_EXTENDS_TOKEN"));
        // Idempotent when re-added with the same config.
        assert!(!trust_add(url, &tmp, None, Some("ARAI_EXTENDS_TOKEN")).unwrap());
        // Secret hygiene at rest: the on-disk trust file contains only the
        // variable NAME — a token-shaped value must never appear, no matter
        // what the environment holds.
        let on_disk = std::fs::read_to_string(trust_path(&tmp)).unwrap();
        assert!(on_disk.contains("ARAI_EXTENDS_TOKEN"));
        assert!(!on_disk.to_lowercase().contains("secret"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn legacy_trust_entries_have_no_bearer() {
        let tf: TrustFile = toml::from_str("trusted = [\"https://example.com/rules.md\"]").unwrap();
        assert_eq!(tf.trusted.len(), 1);
        assert!(tf.trusted[0].bearer_env.is_none());
    }

    #[test]
    fn resolve_bearer_decision_table() {
        let entry = |bearer_env: Option<&str>| TrustEntry {
            url: "https://example.com/org-rules.md".to_string(),
            pubkey: None,
            bearer_env: bearer_env.map(String::from),
        };
        // No bearer_env configured → no header, even when the env var is set.
        assert_eq!(
            resolve_bearer_with(&entry(None), |_| Some("tok".into())),
            None
        );
        // Configured + set → token resolved.
        assert_eq!(
            resolve_bearer_with(&entry(Some("T")), |n| {
                assert_eq!(n, "T");
                Some("s3cr3t".into())
            }),
            Some("s3cr3t".to_string())
        );
        // Configured but unset → unauthenticated, not an error.
        assert_eq!(resolve_bearer_with(&entry(Some("T")), |_| None), None);
        // Configured but empty → treated as unset.
        assert_eq!(
            resolve_bearer_with(&entry(Some("T")), |_| Some(String::new())),
            None
        );
    }

    #[test]
    fn scrub_secret_redacts_every_occurrence() {
        let msg = "HTTP error: header Authorization: Bearer tok123 rejected (tok123)".to_string();
        let scrubbed = scrub_secret(msg, Some("tok123"));
        assert!(!scrubbed.contains("tok123"));
        assert_eq!(scrubbed.matches("[redacted]").count(), 2);
        // No secret / empty secret: message unchanged.
        assert_eq!(scrub_secret("plain".to_string(), None), "plain");
        assert_eq!(scrub_secret("plain".to_string(), Some("")), "plain");
    }

    #[test]
    fn bearer_env_name_validation_rejects_pasted_tokens() {
        let tmp = std::env::temp_dir().join(format!("arai_bearer_val_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let url = "https://example.com/org-rules.md";
        for bad in ["ghp_abc123XYZ-token", "Bearer xyz", "1TOKEN", "", "TOK EN"] {
            let err = trust_add(url, &tmp, None, Some(bad)).unwrap_err();
            // The error must not echo the rejected value — it may be a secret.
            assert!(
                !err.contains(bad) || bad.is_empty(),
                "error echoes value: {err}"
            );
        }
        for good in ["ARAI_EXTENDS_TOKEN", "_T", "a1"] {
            trust_add(url, &tmp, None, Some(good)).unwrap();
        }
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_resolve_inlines_untrusted_url_silently_noops() {
        // Untrusted URL: resolve returns original content, prints to stderr.
        let tmp = std::env::temp_dir().join(format!("arai_resolve_untrust_{}", std::process::id()));
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

    #[test]
    fn test_url_cache_sig_path_pairs_with_content() {
        let tmp = std::path::PathBuf::from("/tmp/arai_cache_sig_test");
        let content_path = url_cache_path("https://example.com/a.md", &tmp);
        let sig_path = url_cache_sig_path("https://example.com/a.md", &tmp);
        assert_eq!(
            sig_path,
            std::path::PathBuf::from(format!("{}.sha256", content_path.display()))
        );
    }

    #[test]
    fn test_read_cache_verified_accepts_matching_sidecar() {
        let tmp = std::env::temp_dir().join(format!(
            "arai_extends_sigok_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let url = "https://example.com/policy.md";
        let path = url_cache_path(url, &tmp);
        let sig = url_cache_sig_path(url, &tmp);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let body = "- never push --force\n";
        std::fs::write(&path, body).unwrap();
        std::fs::write(&sig, content_sha256_hex(body)).unwrap();
        let read = read_cache_verified(&path, &sig, url);
        assert_eq!(read.as_deref(), Some(body));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_read_cache_verified_rejects_mismatched_sidecar() {
        let tmp = std::env::temp_dir().join(format!(
            "arai_extends_sigbad_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let url = "https://example.com/policy.md";
        let path = url_cache_path(url, &tmp);
        let sig = url_cache_sig_path(url, &tmp);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Sidecar matches *original* content; tamper with the body after.
        let original = "- never push --force\n";
        std::fs::write(&sig, content_sha256_hex(original)).unwrap();
        std::fs::write(&path, "- tampered policy\n").unwrap();
        let read = read_cache_verified(&path, &sig, url);
        assert!(
            read.is_none(),
            "tampered cache must be refused even with present sidecar"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_read_cache_verified_rejects_missing_sidecar() {
        // Pre-signature cache files (or sidecar-deleted ones) must be
        // treated as a miss — the caller can then re-fetch and write a
        // fresh sidecar.
        let tmp = std::env::temp_dir().join(format!(
            "arai_extends_signone_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let url = "https://example.com/policy.md";
        let path = url_cache_path(url, &tmp);
        let sig = url_cache_sig_path(url, &tmp);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "- some rule\n").unwrap();
        // No sidecar written.
        let read = read_cache_verified(&path, &sig, url);
        assert!(read.is_none(), "missing sidecar → treat as cache miss");
        std::fs::remove_dir_all(&tmp).ok();
    }

    // ── directive-tokenisation unit tests ────────────────────────────────────

    const GOOD_PIN: &str = "a3f1e2b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2";

    // AC1 — bare directive with no trailing tokens produces ParsedDirective
    // with pin absent and tier absent.
    #[test]
    fn classify_bare_url_success() {
        let result = classify_directive("https://example.com/policy.md");
        assert_eq!(
            result,
            Ok(ParsedDirective {
                url: "https://example.com/policy.md".to_string(),
                pin: None,
                tier: None,
            })
        );
    }

    // AC1 — both surface forms produce identical output for a bare directive.
    #[test]
    fn classify_bare_url_html_comment_form() {
        let result = classify_directive("<!-- arai:extends https://example.com/policy.md -->");
        assert_eq!(
            result,
            Ok(ParsedDirective {
                url: "https://example.com/policy.md".to_string(),
                pin: None,
                tier: None,
            })
        );
    }

    // AC12g — valid 64-char hex pin accepted.
    #[test]
    fn classify_valid_pin_accepted() {
        let input = format!("https://example.com/policy.md @{GOOD_PIN}");
        let result = classify_directive(&input);
        assert_eq!(
            result,
            Ok(ParsedDirective {
                url: "https://example.com/policy.md".to_string(),
                pin: Some(GOOD_PIN.to_string()),
                tier: None,
            })
        );
    }

    // AC3 (tokeniser half) — malformed pin: too short.
    #[test]
    fn classify_short_pin_rejected() {
        let result = classify_directive("https://example.com/policy.md @abc123");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.offending_token, "@abc123");
    }

    // AC3 (tokeniser half) — malformed pin: lone @.
    #[test]
    fn classify_lone_at_rejected() {
        let result = classify_directive("https://example.com/policy.md @");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.offending_token, "@");
    }

    // AC3 (tokeniser half) — malformed pin: 63 hex chars (one short).
    #[test]
    fn classify_63_hex_pin_rejected() {
        let short_pin = "a".repeat(63);
        let input = format!("https://example.com/policy.md @{short_pin}");
        let result = classify_directive(&input);
        assert!(result.is_err());
    }

    // AC3 (tokeniser half) — malformed pin: uppercase chars are accepted by
    // is_valid_pin_hex and normalised to lowercase.
    #[test]
    fn classify_uppercase_pin_normalised() {
        let upper_pin = "A".repeat(64);
        let expected_lower = "a".repeat(64);
        let input = format!("https://example.com/policy.md @{upper_pin}");
        let result = classify_directive(&input);
        assert_eq!(
            result,
            Ok(ParsedDirective {
                url: "https://example.com/policy.md".to_string(),
                pin: Some(expected_lower),
                tier: None,
            })
        );
    }

    // AC12c — tier=strict accepted.
    #[test]
    fn classify_tier_strict_accepted() {
        let result = classify_directive("https://example.com/policy.md tier=strict");
        assert_eq!(
            result,
            Ok(ParsedDirective {
                url: "https://example.com/policy.md".to_string(),
                pin: None,
                tier: Some(Tier::Strict),
            })
        );
    }

    // AC12c — tier=advisory accepted.
    #[test]
    fn classify_tier_advisory_accepted() {
        let result = classify_directive("https://example.com/policy.md tier=advisory");
        assert_eq!(
            result,
            Ok(ParsedDirective {
                url: "https://example.com/policy.md".to_string(),
                pin: None,
                tier: Some(Tier::Advisory),
            })
        );
    }

    // AC12c — tier=override accepted (AC11_drop_syntax settled decision).
    #[test]
    fn classify_tier_override_accepted() {
        let result = classify_directive("https://example.com/policy.md tier=override");
        assert_eq!(
            result,
            Ok(ParsedDirective {
                url: "https://example.com/policy.md".to_string(),
                pin: None,
                tier: Some(Tier::Override),
            })
        );
    }

    // AC12b — unknown tier= value rejected.
    #[test]
    fn classify_unknown_tier_rejected() {
        let result = classify_directive("https://example.com/policy.md tier=unknown");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.offending_token, "tier=unknown");
    }

    // AC12b — tier=peer is NOT a writable value (vocabulary: "peer is NOT
    // a writable directive token").
    #[test]
    fn classify_tier_peer_keyword_rejected() {
        let result = classify_directive("https://example.com/policy.md tier=peer");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.offending_token, "tier=peer");
    }

    // AC12a — unknown trailing token rejected.
    #[test]
    fn classify_unknown_token_rejected() {
        let result = classify_directive("https://example.com/policy.md foo");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.offending_token, "foo");
    }

    // AC12a — bar=baz (key-value but not tier=) is unknown.
    #[test]
    fn classify_unknown_kv_token_rejected() {
        let result = classify_directive("https://example.com/policy.md bar=baz");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.offending_token, "bar=baz");
    }

    // AC12a — "strict" without "tier=" prefix is unknown.
    #[test]
    fn classify_bare_tier_value_rejected() {
        let result = classify_directive("https://example.com/policy.md strict");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.offending_token, "strict");
    }

    // AC12d (BINDING) — duplicate @pin token → fail-closed, second named.
    #[test]
    fn classify_duplicate_pin_rejected() {
        let pin2 = "b".repeat(64);
        let input = format!("https://example.com/policy.md @{GOOD_PIN} @{pin2}");
        let result = classify_directive(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        // The second pin token must be named as the offending token.
        assert!(
            err.offending_token.starts_with('@'),
            "offending token should be the second pin"
        );
        assert!(err.reason.contains("duplicate"));
    }

    // AC12e (BINDING) — duplicate tier= token → fail-closed, second named.
    #[test]
    fn classify_duplicate_tier_rejected() {
        let result = classify_directive("https://example.com/policy.md tier=strict tier=advisory");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.offending_token, "tier=advisory");
        assert!(err.reason.contains("duplicate"));
    }

    // AC12e (BINDING) — duplicate tier= with same value is still malformed.
    #[test]
    fn classify_duplicate_tier_same_value_rejected() {
        let result = classify_directive("https://example.com/policy.md tier=strict tier=strict");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.offending_token, "tier=strict");
        assert!(err.reason.contains("duplicate"));
    }

    // AC12f — pin then tier and tier then pin produce identical ParsedDirective.
    #[test]
    fn classify_order_independence() {
        let pin_then_tier = classify_directive(&format!(
            "https://example.com/policy.md @{GOOD_PIN} tier=strict"
        ));
        let tier_then_pin = classify_directive(&format!(
            "https://example.com/policy.md tier=strict @{GOOD_PIN}"
        ));
        assert!(pin_then_tier.is_ok());
        assert_eq!(pin_then_tier, tier_then_pin);
    }

    // AC12h — @-character inside the URL token is not misclassified as a pin.
    #[test]
    fn classify_in_url_at_not_a_pin() {
        let result = classify_directive("https://user@host.example.com/path.md");
        assert_eq!(
            result,
            Ok(ParsedDirective {
                url: "https://user@host.example.com/path.md".to_string(),
                pin: None,
                tier: None,
            })
        );
    }

    // AC12h — @-in-url combined with a valid trailing pin token.
    #[test]
    fn classify_in_url_at_plus_pin_token() {
        let input = format!("https://user@host.example.com/path.md @{GOOD_PIN}");
        let result = classify_directive(&input);
        assert_eq!(
            result,
            Ok(ParsedDirective {
                url: "https://user@host.example.com/path.md".to_string(),
                pin: Some(GOOD_PIN.to_string()),
                tier: None,
            })
        );
    }

    // Regression: extract_urls should still work correctly for directives
    // that carry trailing tokens (pin/tier in the line are now silently
    // consumed by classify_directive; the URL is still returned).
    #[test]
    fn extract_urls_with_pin_token() {
        let content =
            format!("# arai:extends https://example.com/a.md @{GOOD_PIN}\n\n# My rules\n");
        let urls = extract_urls(&content);
        assert_eq!(urls, vec!["https://example.com/a.md"]);
    }

    // Regression: extract_urls for a malformed directive returns nothing (no
    // partial URL leak).
    #[test]
    fn extract_urls_malformed_directive_skipped() {
        let content = "# arai:extends https://example.com/a.md foo\n\n# My rules\n";
        let urls = extract_urls(content);
        assert!(
            urls.is_empty(),
            "malformed directive must be skipped entirely"
        );
    }

    // ── fetch-verification unit tests ─────────────────────────────────────────

    /// Generate a deterministic keypair for use in tests.
    /// Uses a fixed 32-byte seed so tests are reproducible.
    fn test_keypair(seed_byte: u8) -> (ed25519_dalek::SigningKey, String) {
        use ed25519_dalek::SigningKey;
        let signing_key = SigningKey::from_bytes(&[seed_byte; 32]);
        let vk = signing_key.verifying_key();
        let vk_hex: String = vk.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
        (signing_key, vk_hex)
    }

    /// Sign content bytes and return the 128-char hex-encoded signature.
    fn sign_content(signing_key: &ed25519_dalek::SigningKey, content: &[u8]) -> String {
        use ed25519_dalek::Signer;
        let sig = signing_key.sign(content);
        sig.to_bytes().iter().map(|b| format!("{b:02x}")).collect()
    }

    // AC2 — Matching pin admits content (pubkey absent).
    #[test]
    fn ac2_matching_pin_admits() {
        let content = b"- never force-push\n";
        let pin = bytes_sha256_hex(content);
        let entry = TrustEntry {
            url: "https://example.com/policy.md".to_string(),
            pubkey: None,
            bearer_env: None,
        };
        // No sidecar fetch callback should be invoked.
        let result = verify_content(
            "https://example.com/policy.md",
            Some(&pin),
            content,
            &entry,
            |_| panic!("sidecar should not be fetched when pubkey is absent"),
        );
        assert!(result.is_ok(), "matching pin should admit: {:?}", result);
    }

    // AC3 — Pin mismatch rejects content.
    #[test]
    fn ac3_pin_mismatch_rejects() {
        let content = b"- never force-push\n";
        let wrong_pin = "a".repeat(64); // not the sha256 of content
        let entry = TrustEntry {
            url: "https://example.com/policy.md".to_string(),
            pubkey: None,
            bearer_env: None,
        };
        let result = verify_content(
            "https://example.com/policy.md",
            Some(&wrong_pin),
            content,
            &entry,
            |_| panic!("sidecar should not be fetched when pubkey is absent"),
        );
        assert!(result.is_err(), "pin mismatch should reject");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("pin check"),
            "warning should mention pin check: {msg}"
        );
        assert!(
            msg.contains("example.com"),
            "warning should name the URL: {msg}"
        );
    }

    // AC4 — Pin check runs on the stale-cache path too.
    // We test this by calling verify_content with stale-cache content (same
    // function, different source — the test ensures pin comparison is not
    // skipped regardless of content origin).
    #[test]
    fn ac4_pin_check_on_stale_cache_content() {
        // Seed cache with tampered content whose sha256 does not match the pin.
        let cache_content = b"- tampered content\n";
        let original_content = b"- original content\n";
        let pin_for_original = bytes_sha256_hex(original_content);

        let entry = TrustEntry {
            url: "https://example.com/policy.md".to_string(),
            pubkey: None,
            bearer_env: None,
        };
        // Simulate stale-cache path: verify_content called with cache content
        // and a pin from the directive.  The cache content ≠ original, so
        // the pin check must reject.
        let result = verify_content(
            "https://example.com/policy.md",
            Some(&pin_for_original),
            cache_content,
            &entry,
            |_| panic!("sidecar should not be fetched when pubkey is absent"),
        );
        assert!(
            result.is_err(),
            "stale-cache content with pin mismatch should reject"
        );
    }

    // AC5 — Configured pubkey + valid signature admits content.
    #[test]
    fn ac5_valid_signature_admits() {
        let (signing_key, vk_hex) = test_keypair(42);
        let content = b"- never force-push\n";
        let sig_hex = sign_content(&signing_key, content);

        let entry = TrustEntry {
            url: "https://example.com/policy.md".to_string(),
            pubkey: Some(vk_hex),
            bearer_env: None,
        };
        // Sidecar fetch callback returns our valid signature.
        let result = verify_content(
            "https://example.com/policy.md",
            None, // no pin
            content,
            &entry,
            |_url| Ok(sig_hex.as_bytes().to_vec()),
        );
        assert!(result.is_ok(), "valid signature should admit: {:?}", result);
    }

    // AC5 — Valid pin + valid signature admits content.
    #[test]
    fn ac5_valid_pin_and_signature_admits() {
        let (signing_key, vk_hex) = test_keypair(7);
        let content = b"- never force-push\n";
        let pin = bytes_sha256_hex(content);
        let sig_hex = sign_content(&signing_key, content);

        let entry = TrustEntry {
            url: "https://example.com/policy.md".to_string(),
            pubkey: Some(vk_hex),
            bearer_env: None,
        };
        let result = verify_content(
            "https://example.com/policy.md",
            Some(&pin),
            content,
            &entry,
            |_url| Ok(sig_hex.as_bytes().to_vec()),
        );
        assert!(
            result.is_ok(),
            "valid pin + valid signature should admit: {:?}",
            result
        );
    }

    // AC6a — Configured pubkey + sidecar fetch failure rejects.
    #[test]
    fn ac6a_sidecar_fetch_failure_rejects() {
        let (_, vk_hex) = test_keypair(1);
        let content = b"- never force-push\n";

        let entry = TrustEntry {
            url: "https://example.com/policy.md".to_string(),
            pubkey: Some(vk_hex),
            bearer_env: None,
        };
        let result = verify_content(
            "https://example.com/policy.md",
            None,
            content,
            &entry,
            |_url| Err("network error: connection refused".to_string()),
        );
        assert!(result.is_err(), "sidecar fetch failure should reject");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("missing or unreachable signature sidecar"),
            "warning should mention missing sidecar: {msg}"
        );
    }

    // AC6b — Configured pubkey + invalid signature rejects.
    #[test]
    fn ac6b_invalid_signature_rejects() {
        let (signing_key, vk_hex) = test_keypair(1);
        let content = b"- never force-push\n";
        // Sign different content — signature won't match.
        let wrong_content = b"- different content\n";
        let bad_sig_hex = sign_content(&signing_key, wrong_content);

        let entry = TrustEntry {
            url: "https://example.com/policy.md".to_string(),
            pubkey: Some(vk_hex),
            bearer_env: None,
        };
        let result = verify_content(
            "https://example.com/policy.md",
            None,
            content,
            &entry,
            |_url| Ok(bad_sig_hex.as_bytes().to_vec()),
        );
        assert!(result.is_err(), "invalid signature should reject");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("signature verification"),
            "warning should mention signature verification: {msg}"
        );
    }

    // AC7 — No configured pubkey means no sidecar fetch and no signature check.
    #[test]
    fn ac7_no_pubkey_no_sig_check() {
        let content = b"- never force-push\n";
        let entry = TrustEntry {
            url: "https://example.com/policy.md".to_string(),
            pubkey: None,
            bearer_env: None,
        };
        let sidecar_called = std::cell::Cell::new(false);
        let result = verify_content(
            "https://example.com/policy.md",
            None,
            content,
            &entry,
            |_url| {
                sidecar_called.set(true);
                Err("should not be called".to_string())
            },
        );
        assert!(
            !sidecar_called.get(),
            "sidecar callback must not be invoked when pubkey absent"
        );
        assert!(
            result.is_ok(),
            "no pin + no pubkey should always admit: {:?}",
            result
        );
    }

    // AC7 — No pubkey: only pin check runs (not sig check).
    #[test]
    fn ac7_no_pubkey_pin_check_only() {
        let content = b"- rule\n";
        let pin = bytes_sha256_hex(content);
        let entry = TrustEntry {
            url: "https://example.com/policy.md".to_string(),
            pubkey: None,
            bearer_env: None,
        };
        let sidecar_called = std::cell::Cell::new(false);
        let result = verify_content(
            "https://example.com/policy.md",
            Some(&pin),
            content,
            &entry,
            |_url| {
                sidecar_called.set(true);
                Err("should not be called".to_string())
            },
        );
        assert!(
            !sidecar_called.get(),
            "sidecar callback must not be invoked when pubkey absent"
        );
        assert!(
            result.is_ok(),
            "matching pin with no pubkey should admit: {:?}",
            result
        );
    }

    // AC8 — Legacy trust file (list of strings) parses correctly.
    #[test]
    fn ac8_legacy_trust_file_parses() {
        let toml_legacy = r#"trusted = ["https://example.com/policy.md"]"#;
        let tf: TrustFile = toml::from_str(toml_legacy).expect("legacy trust file must parse");
        assert_eq!(tf.trusted.len(), 1);
        assert_eq!(tf.trusted[0].url, "https://example.com/policy.md");
        assert!(
            tf.trusted[0].pubkey.is_none(),
            "legacy entry must have pubkey absent"
        );
    }

    // AC8 — New trust file (inline tables) also parses.
    #[test]
    fn ac8_new_trust_file_parses() {
        let toml_new = r#"trusted = [{url = "https://example.com/policy.md", pubkey = "aabbcc0000000000000000000000000000000000000000000000000000000000"}]"#;
        let tf: TrustFile = toml::from_str(toml_new).expect("new trust file must parse");
        assert_eq!(tf.trusted.len(), 1);
        assert_eq!(tf.trusted[0].url, "https://example.com/policy.md");
        assert!(
            tf.trusted[0].pubkey.is_some(),
            "new entry must have pubkey present"
        );
    }

    // AC8 — Mixed trust file (both forms) parses correctly.
    #[test]
    fn ac8_mixed_trust_file_parses() {
        let toml_mixed = r#"trusted = [
  "https://example.com/a.md",
  {url = "https://example.com/b.md", pubkey = "aabbcc0000000000000000000000000000000000000000000000000000000000"},
]"#;
        let tf: TrustFile = toml::from_str(toml_mixed).expect("mixed trust file must parse");
        assert_eq!(tf.trusted.len(), 2);
        assert_eq!(tf.trusted[0].url, "https://example.com/a.md");
        assert!(tf.trusted[0].pubkey.is_none());
        assert_eq!(tf.trusted[1].url, "https://example.com/b.md");
        assert!(tf.trusted[1].pubkey.is_some());
    }

    // AC8 — Legacy trust file: round-trip behaviour is preserved (no rewrite on read).
    // We test that reading a legacy file via is_trusted + trust_list works, and
    // the on-disk file is not modified.
    #[test]
    fn ac8_legacy_file_not_rewritten_on_read() {
        let tmp = std::env::temp_dir().join(format!(
            "arai_trust_legacy_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let trust_file = trust_path(&tmp);
        std::fs::write(&trust_file, r#"trusted = ["https://example.com/a.md"]"#).unwrap();

        // Read without modifying.
        assert!(is_trusted("https://example.com/a.md", &tmp));
        let entries = trust_list(&tmp);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].pubkey.is_none());

        // Confirm on-disk content was not rewritten.
        let after = std::fs::read_to_string(&trust_file).unwrap();
        assert!(
            after.contains(r#"trusted = ["https://example.com/a.md"]"#),
            "legacy file must not be rewritten on read: {after:?}"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    // AC13a — trust --add --pubkey writes keyed entry to trust file.
    #[test]
    fn ac13a_trust_add_with_pubkey_writes_entry() {
        let (_, vk_hex) = test_keypair(99);
        let tmp = std::env::temp_dir().join(format!(
            "arai_trust_key_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let url = "https://example.com/policy.md";

        let added = trust_add(url, &tmp, Some(&vk_hex), None).unwrap();
        assert!(added, "should return true for a new entry");

        let entries = trust_list(&tmp);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url, url);
        assert_eq!(
            entries[0].pubkey.as_deref(),
            Some(vk_hex.as_str()),
            "pubkey must be stored as supplied"
        );

        // verify_content for this URL must now perform a signature check.
        // Confirm: valid signature → admits; invalid signature → rejects.
        let content = b"- test rule\n";
        let (signing_key, _) = test_keypair(99); // same seed = same key
        let sig_hex = sign_content(&signing_key, content);
        let entry = &entries[0];
        let result = verify_content(url, None, content, entry, |_| {
            Ok(sig_hex.as_bytes().to_vec())
        });
        assert!(
            result.is_ok(),
            "valid sig after trust_add should admit: {:?}",
            result
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    // AC13b — trust listing distinguishes keyed from non-keyed entries.
    #[test]
    fn ac13b_listing_distinguishes_keyed_entries() {
        let (_, vk_hex) = test_keypair(5);
        let tmp = std::env::temp_dir().join(format!(
            "arai_trust_list_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).unwrap();

        trust_add("https://example.com/plain.md", &tmp, None, None).unwrap();
        trust_add("https://example.com/signed.md", &tmp, Some(&vk_hex), None).unwrap();

        let entries = trust_list(&tmp);
        assert_eq!(entries.len(), 2);

        let plain = entries.iter().find(|e| e.url.contains("plain")).unwrap();
        let signed_entry = entries.iter().find(|e| e.url.contains("signed")).unwrap();

        assert!(plain.pubkey.is_none(), "plain entry must have no pubkey");
        assert!(
            signed_entry.pubkey.is_some(),
            "signed entry must have a pubkey"
        );
        assert_eq!(signed_entry.pubkey.as_deref(), Some(vk_hex.as_str()));

        std::fs::remove_dir_all(&tmp).ok();
    }

    // Malformed configured key rejects (never silently downgrades).
    #[test]
    fn malformed_configured_key_rejects() {
        let content = b"- rule\n";
        let entry = TrustEntry {
            url: "https://example.com/policy.md".to_string(),
            pubkey: Some("not_a_valid_hex_key_at_all_and_too_short".to_string()),
            bearer_env: None,
        };
        let result = verify_content(
            "https://example.com/policy.md",
            None,
            content,
            &entry,
            |_| unreachable!("sidecar should not be fetched when key is malformed"),
        );
        assert!(
            result.is_err(),
            "malformed pubkey must reject (fail-closed)"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("malformed configured pubkey"),
            "warning must mention malformed key: {msg}"
        );
    }
}
