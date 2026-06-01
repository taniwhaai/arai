//! Brand palette and semantic styling helpers.
//!
//! Holds the two fixed foreground colours — pounamu and ochre — decides per
//! stream whether colour may be emitted (the `should_colorize` gate), and
//! exposes a closed set of five semantic helpers: `structural`, `passage`,
//! `dim`, `warn`, `error`.  Each helper returns its input text either wrapped
//! in the appropriate 24-bit truecolor ANSI escape + reset, or returned
//! byte-identical, based on the gate's verdict.
//!
//! Machine-consumed paths (hook-protocol JSON, every `--json` rendering) must
//! never be routed through these helpers.  The module itself is total: every
//! function returns a value for every input, with no failure modes.

use std::io::IsTerminal;

// ── Palette ──────────────────────────────────────────────────────────────────

/// Pounamu: structural and informational text.  RGB(61, 130, 104).
const POUNAMU_R: u8 = 61;
const POUNAMU_G: u8 = 130;
const POUNAMU_B: u8 = 104;

/// Ochre: decision / passage moments; warn and error styling.  RGB(184, 118, 58).
const OCHRE_R: u8 = 184;
const OCHRE_G: u8 = 118;
const OCHRE_B: u8 = 58;

// ── ANSI primitives ───────────────────────────────────────────────────────────

/// Build a 24-bit foreground truecolor escape: `ESC[38;2;R;G;Bm`.
#[inline]
fn fg(r: u8, g: u8, b: u8) -> String {
    format!("\x1b[38;2;{r};{g};{b}m")
}

/// SGR reset: `ESC[0m`.
const RESET: &str = "\x1b[0m";

/// SGR bold: `ESC[1m`.
const BOLD: &str = "\x1b[1m";

/// SGR faint/dim: `ESC[2m`.
#[allow(dead_code)]
const DIM_ATTR: &str = "\x1b[2m";

// ── Gate ─────────────────────────────────────────────────────────────────────

/// Which stream to query terminal status for.
#[derive(Clone, Copy, Debug)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Decide whether the given stream may receive ANSI colour escapes.
///
/// Precedence (fixed, non-configurable):
/// 1. `NO_COLOR` present in the environment (any value) → **off**.
/// 2. `CLICOLOR_FORCE` present in the environment (any value) → **on**.
/// 3. Stream is attached to a terminal → **on**; otherwise **off**.
pub fn should_colorize(stream: Stream) -> bool {
    // Rule 1: NO_COLOR present → always off.
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    // Rule 2: CLICOLOR_FORCE present → always on.
    if std::env::var_os("CLICOLOR_FORCE").is_some() {
        return true;
    }
    // Rule 3: terminal detection.  Undetermined = off (safe direction).
    match stream {
        Stream::Stdout => std::io::stdout().is_terminal(),
        Stream::Stderr => std::io::stderr().is_terminal(),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Structural / informational text — pounamu foreground, no intensity change.
///
/// Escape sequence when on: `ESC[38;2;61;130;104m` + text + `ESC[0m`.
pub fn structural(text: &str, colorize: bool) -> String {
    if colorize {
        format!("{}{text}{RESET}", fg(POUNAMU_R, POUNAMU_G, POUNAMU_B))
    } else {
        text.to_string()
    }
}

/// Decision / passage moments — ochre foreground, no intensity change.
///
/// Escape sequence when on: `ESC[38;2;184;118;58m` + text + `ESC[0m`.
pub fn passage(text: &str, colorize: bool) -> String {
    if colorize {
        format!("{}{text}{RESET}", fg(OCHRE_R, OCHRE_G, OCHRE_B))
    } else {
        text.to_string()
    }
}

/// Dim / faint text — faint SGR attribute, no new RGB colour.
///
/// Escape sequence when on: `ESC[2m` + text + `ESC[0m`.
#[allow(dead_code)]
pub fn dim(text: &str, colorize: bool) -> String {
    if colorize {
        format!("{DIM_ATTR}{text}{RESET}")
    } else {
        text.to_string()
    }
}

/// Warning text — ochre foreground + bold.
///
/// Escape sequence when on: `ESC[38;2;184;118;58m` + `ESC[1m` + text + `ESC[0m`.
pub fn warn(text: &str, colorize: bool) -> String {
    if colorize {
        format!("{}{BOLD}{text}{RESET}", fg(OCHRE_R, OCHRE_G, OCHRE_B))
    } else {
        text.to_string()
    }
}

/// Error text — ochre foreground + bold (identical output to `warn`; semantic
/// distinction is meaningful to callers choosing which to call).
///
/// Escape sequence when on: `ESC[38;2;184;118;58m` + `ESC[1m` + text + `ESC[0m`.
pub fn error(text: &str, colorize: bool) -> String {
    // Deliberately identical output to `warn` per the Semantic Role Set.
    warn(text, colorize)
}

// ── Gateway outcome glyphs ────────────────────────────────────────────────────

/// The four possible outcomes for a gateway decision — used to select the
/// appropriate glyph from the binding table.
///
/// Block and Allow map to the deny/pass sides of the gateway.  Warn and Inform
/// both map to the `warned` glyph (dot adjacent, pre-passage).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Block,
    Warn,
    Inform,
    Allow,
}

/// Decide whether Unicode glyph forms should be used.
///
/// Precedence (fixed):
/// 1. `ARAI_ASCII` present and non-empty → ASCII (false).
/// 2. `NO_UNICODE` present and non-empty → ASCII (false).
/// 3. First of `LC_ALL`, `LC_CTYPE`, `LANG` that is set and non-empty:
///    if it contains `utf-8` or `utf8` (case-insensitive) → Unicode (true).
/// 4. No locale variable provides a UTF-8 signal → ASCII (false).
///
/// TTY-independent: terminal status is never consulted.
pub fn should_use_unicode() -> bool {
    // Rule 1: ARAI_ASCII present → always ASCII.
    if let Ok(v) = std::env::var("ARAI_ASCII") {
        if !v.is_empty() {
            return false;
        }
    }
    // Rule 2: NO_UNICODE present → always ASCII.
    if let Ok(v) = std::env::var("NO_UNICODE") {
        if !v.is_empty() {
            return false;
        }
    }
    // Rules 3–4: locale priority LC_ALL > LC_CTYPE > LANG.
    for var in &["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(locale) = std::env::var(var) {
            if !locale.is_empty() {
                let lower = locale.to_lowercase();
                if lower.contains("utf-8") || lower.contains("utf8") {
                    return true;
                }
                // This locale var is set but not UTF-8 — stop here (don't
                // fall through to a lower-priority variable).
                return false;
            }
        }
    }
    // No locale variable provided a signal.
    false
}

/// Return the gateway glyph string for the given outcome.
///
/// | outcome        | Unicode   | ASCII  |
/// |----------------|-----------|--------|
/// | Block          | `●·│✕`    | `o.|x` |
/// | Allow          | `│●│`     | `\|o\|` |
/// | Warn / Inform  | `●·│`     | `o.\|` |
///
/// The `✕` in the Unicode blocked form (and the `x` in ASCII blocked form) is
/// wrapped in the ochre/error treatment when `colorize = true` **and**
/// `unicode = true` **and** `outcome = Block`.  In every other case the
/// returned string contains no ANSI bytes.
///
/// ASCII forms are 7-bit clean: every byte ≤ 0x7F.
pub fn outcome_glyph(outcome: Outcome, unicode: bool, colorize: bool) -> String {
    match (outcome, unicode) {
        // ── Unicode forms ────────────────────────────────────────────────────
        (Outcome::Block, true) => {
            // Only the ✕ carries colour; the rest is always bare.
            let cross = if colorize {
                passage("\u{2715}", true) // ochre via the existing passage helper
            } else {
                "\u{2715}".to_string()
            };
            // ●·│ + (optionally coloured) ✕
            format!("\u{25CF}\u{00B7}\u{2502}{cross}")
        }
        (Outcome::Allow, true) => "\u{2502}\u{25CF}\u{2502}".to_string(),
        (Outcome::Warn, true) | (Outcome::Inform, true) => "\u{25CF}\u{00B7}\u{2502}".to_string(),
        // ── ASCII forms ──────────────────────────────────────────────────────
        (Outcome::Block, false) => "o.|x".to_string(),
        (Outcome::Allow, false) => "|o|".to_string(),
        (Outcome::Warn, false) | (Outcome::Inform, false) => "o.|".to_string(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    // Helper: clear both env vars, run a closure, then restore.
    fn with_env<F: FnOnce() -> R, R>(no_color: bool, clicolor_force: bool, f: F) -> R {
        // Remove first to avoid interference between iterations.
        env::remove_var("NO_COLOR");
        env::remove_var("CLICOLOR_FORCE");
        if no_color {
            env::set_var("NO_COLOR", "1");
        }
        if clicolor_force {
            env::set_var("CLICOLOR_FORCE", "1");
        }
        let result = f();
        env::remove_var("NO_COLOR");
        env::remove_var("CLICOLOR_FORCE");
        result
    }

    // ── AC2: NO_COLOR → gate returns false, helpers return plain ─────────────

    #[test]
    fn ac2_no_color_gate_is_off() {
        with_env(true, false, || {
            // Both streams must return false.
            assert!(!should_colorize(Stream::Stdout));
            assert!(!should_colorize(Stream::Stderr));
        });
    }

    #[test]
    fn ac2_no_color_dominates_clicolor_force() {
        // NO_COLOR beats CLICOLOR_FORCE (rule 1 > rule 2).
        with_env(true, true, || {
            assert!(!should_colorize(Stream::Stdout));
            assert!(!should_colorize(Stream::Stderr));
        });
    }

    #[test]
    fn ac2_no_color_helpers_return_plain() {
        with_env(true, false, || {
            let off = false; // simulate gate verdict
            let input = "test text";
            for s in [
                structural(input, off),
                passage(input, off),
                dim(input, off),
                warn(input, off),
                error(input, off),
            ] {
                assert_eq!(s, input, "helper must return input byte-identical when off");
                assert!(!s.contains('\x1b'), "no ANSI escape bytes when off: {s:?}");
            }
        });
    }

    // ── AC3: non-TTY (plain colorize=false) → plain output ───────────────────

    #[test]
    fn ac3_non_tty_helpers_return_plain() {
        // We simulate non-TTY by calling helpers with colorize=false.
        let input = "hello world";
        for s in [
            structural(input, false),
            passage(input, false),
            dim(input, false),
            warn(input, false),
            error(input, false),
        ] {
            assert_eq!(s, input, "non-TTY helper must return input unchanged");
            assert!(!s.contains('\x1b'));
        }
    }

    // ── CLICOLOR_FORCE → gate returns true (even without a TTY) ──────────────

    #[test]
    fn clicolor_force_gate_is_on() {
        with_env(false, true, || {
            // CLICOLOR_FORCE: must be on regardless of stream status.
            assert!(should_colorize(Stream::Stdout));
            assert!(should_colorize(Stream::Stderr));
        });
    }

    // ── Styled output has expected truecolor ANSI sequences ──────────────────

    #[test]
    fn structural_emits_pounamu_escape() {
        let s = structural("x", true);
        assert!(
            s.contains("\x1b[38;2;61;130;104m"),
            "structural must contain pounamu escape: {s:?}"
        );
        assert!(s.ends_with("\x1b[0m"), "must end with reset: {s:?}");
        assert!(s.contains("x"), "must contain the input text");
    }

    #[test]
    fn passage_emits_ochre_escape() {
        let s = passage("x", true);
        assert!(
            s.contains("\x1b[38;2;184;118;58m"),
            "passage must contain ochre escape: {s:?}"
        );
        assert!(s.ends_with("\x1b[0m"), "must end with reset: {s:?}");
    }

    #[test]
    fn dim_emits_faint_escape() {
        let s = dim("x", true);
        assert!(
            s.contains("\x1b[2m"),
            "dim must contain faint escape: {s:?}"
        );
        assert!(s.ends_with("\x1b[0m"), "must end with reset: {s:?}");
    }

    #[test]
    fn warn_emits_ochre_bold() {
        let s = warn("x", true);
        assert!(
            s.contains("\x1b[38;2;184;118;58m"),
            "warn must contain ochre: {s:?}"
        );
        assert!(s.contains("\x1b[1m"), "warn must contain bold: {s:?}");
        assert!(s.ends_with("\x1b[0m"), "must end with reset: {s:?}");
    }

    #[test]
    fn error_emits_ochre_bold_same_as_warn() {
        let w = warn("x", true);
        let e = error("x", true);
        assert_eq!(
            w, e,
            "warn and error must produce identical output per spec"
        );
        // Confirm ochre — not red.
        assert!(
            e.contains("\x1b[38;2;184;118;58m"),
            "error must use ochre, not red: {e:?}"
        );
        // Confirm no red escape (no pure 16-colour red \x1b[31m or similar).
        assert!(
            !e.contains("\x1b[31m"),
            "error must not contain 16-colour red: {e:?}"
        );
    }

    // ── AC6: no stoplight — confirm ochre in warn/error, no green/red ────────

    #[test]
    fn ac6_no_stoplight_warn_and_error_use_ochre() {
        let w = warn("text", true);
        let e = error("text", true);

        // Ochre present.
        assert!(w.contains("\x1b[38;2;184;118;58m"));
        assert!(e.contains("\x1b[38;2;184;118;58m"));

        // No red escape (basic 16-colour \x1b[31m).
        assert!(!w.contains("\x1b[31m") && !e.contains("\x1b[31m"));
        // No green escape (basic 16-colour \x1b[32m).
        assert!(!w.contains("\x1b[32m") && !e.contains("\x1b[32m"));
    }

    // ── AC7: foreground-only — no background escape in any helper ────────────

    #[test]
    fn ac7_no_background_escape_in_any_helper() {
        let helpers: Vec<(&str, String)> = vec![
            ("structural", structural("x", true)),
            ("passage", passage("x", true)),
            ("dim", dim("x", true)),
            ("warn", warn("x", true)),
            ("error", error("x", true)),
        ];
        for (name, s) in helpers {
            // Background-colour patterns to reject:
            //   \x1b[4...m  (40-47, basic bg colours)
            //   \x1b[48;...m (256-colour or truecolor bg)
            //   \x1b[10...m (bright bg colours 100-107)
            let has_bg40 = s.contains("\x1b[40m")
                || s.contains("\x1b[41m")
                || s.contains("\x1b[42m")
                || s.contains("\x1b[43m")
                || s.contains("\x1b[44m")
                || s.contains("\x1b[45m")
                || s.contains("\x1b[46m")
                || s.contains("\x1b[47m");
            let has_bg48 = s.contains("\x1b[48;");
            let has_bg100 = (100u8..=107).any(|n| s.contains(&format!("\x1b[{n}m")));
            assert!(
                !has_bg40 && !has_bg48 && !has_bg100,
                "helper `{name}` must not emit any background escape: {s:?}"
            );
        }
    }

    // ── Reset discipline — every styled span is self-contained ───────────────

    #[test]
    fn reset_discipline_every_span_ends_with_reset() {
        for (name, s) in [
            ("structural", structural("hello", true)),
            ("passage", passage("hello", true)),
            ("dim", dim("hello", true)),
            ("warn", warn("hello", true)),
            ("error", error("hello", true)),
        ] {
            assert!(
                s.ends_with("\x1b[0m"),
                "helper `{name}` output must end with reset ESC[0m: {s:?}"
            );
        }
    }

    // ── Plain output is byte-identical ───────────────────────────────────────

    #[test]
    fn plain_output_is_byte_identical_to_input() {
        let inputs = ["", "hello", "  spaces  ", "line1\nline2", "emoji 🎉"];
        for input in inputs {
            for s in [
                structural(input, false),
                passage(input, false),
                dim(input, false),
                warn(input, false),
                error(input, false),
            ] {
                assert_eq!(
                    s.as_str(),
                    input,
                    "colorize=false must return input byte-identical"
                );
            }
        }
    }

    // ── Gateway-outcome-glyphs unit tests ────────────────────────────────────

    // Serialise env-var-touching tests against each other (mirrors the pattern
    // used in the existing hook tests).
    static GLYPH_ENV_MUTEX: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

    fn glyph_env_lock() -> std::sync::MutexGuard<'static, ()> {
        let m = GLYPH_ENV_MUTEX.get_or_init(|| std::sync::Mutex::new(()));
        m.lock().unwrap_or_else(|p| p.into_inner())
    }

    // AC1: outcome-to-glyph mapping, Unicode + colorize=false.
    #[test]
    fn ac1_unicode_glyph_mapping_colorize_false() {
        let cases: &[(Outcome, &str)] = &[
            (Outcome::Block, "\u{25CF}\u{00B7}\u{2502}\u{2715}"),
            (Outcome::Warn, "\u{25CF}\u{00B7}\u{2502}"),
            (Outcome::Inform, "\u{25CF}\u{00B7}\u{2502}"),
            (Outcome::Allow, "\u{2502}\u{25CF}\u{2502}"),
        ];
        for &(outcome, expected) in cases {
            let g = outcome_glyph(outcome, true, false);
            assert_eq!(g, expected, "Unicode glyph mismatch for {outcome:?}: {g:?}");
            assert!(!g.is_empty(), "glyph must not be empty for {outcome:?}");
        }
    }

    // AC1: outcome-to-glyph mapping, ASCII + colorize=false.
    #[test]
    fn ac1_ascii_glyph_mapping_colorize_false() {
        let cases: &[(Outcome, &str)] = &[
            (Outcome::Block, "o.|x"),
            (Outcome::Warn, "o.|"),
            (Outcome::Inform, "o.|"),
            (Outcome::Allow, "|o|"),
        ];
        for &(outcome, expected) in cases {
            let g = outcome_glyph(outcome, false, false);
            assert_eq!(g, expected, "ASCII glyph mismatch for {outcome:?}: {g:?}");
            // Every byte must be 7-bit clean.
            for (i, &b) in g.as_bytes().iter().enumerate() {
                assert!(
                    b <= 0x7F,
                    "ASCII glyph byte[{i}]={b:#04x} > 0x7F for {outcome:?}: {g:?}"
                );
            }
        }
    }

    // AC8: ochre colour appears ONLY on unicode blocked cross when colorize=true.
    #[test]
    fn ac8_ochre_only_on_unicode_blocked_cross_when_colorize_true() {
        // Block + unicode + colorize=true → ANSI bytes present (ochre).
        let with_color = outcome_glyph(Outcome::Block, true, true);
        assert!(
            with_color.contains('\x1b'),
            "Block+unicode+colorize=true must contain ANSI escape: {with_color:?}"
        );
        // The ochre sequence wraps ✕ specifically.
        assert!(
            with_color.contains("\u{2715}"),
            "must still contain ✕ character: {with_color:?}"
        );

        // Block + unicode + colorize=false → no ANSI.
        let no_color = outcome_glyph(Outcome::Block, true, false);
        assert!(
            !no_color.contains('\x1b'),
            "Block+unicode+colorize=false must have no ANSI: {no_color:?}"
        );
        assert_eq!(
            no_color, "\u{25CF}\u{00B7}\u{2502}\u{2715}",
            "bare unicode block glyph: {no_color:?}"
        );

        // Block + ASCII (any colorize) → no ANSI.
        let ascii_block_true = outcome_glyph(Outcome::Block, false, true);
        let ascii_block_false = outcome_glyph(Outcome::Block, false, false);
        assert!(
            !ascii_block_true.contains('\x1b'),
            "Block+ascii+colorize=true must have no ANSI: {ascii_block_true:?}"
        );
        assert!(
            !ascii_block_false.contains('\x1b'),
            "Block+ascii+colorize=false must have no ANSI: {ascii_block_false:?}"
        );

        // Non-blocked outcomes never carry colour regardless of colorize.
        for outcome in [Outcome::Warn, Outcome::Inform, Outcome::Allow] {
            for unicode in [true, false] {
                for colorize in [true, false] {
                    let g = outcome_glyph(outcome, unicode, colorize);
                    assert!(
                        !g.contains('\x1b'),
                        "non-block glyph must never carry ANSI: \
                         outcome={outcome:?} unicode={unicode} colorize={colorize}: {g:?}"
                    );
                }
            }
        }
    }

    // AC2: should_use_unicode precedence — ARAI_ASCII override.
    #[test]
    fn ac2_arai_ascii_forces_ascii() {
        let _guard = glyph_env_lock();
        env::remove_var("ARAI_ASCII");
        env::remove_var("NO_UNICODE");
        env::remove_var("LC_ALL");
        env::remove_var("LC_CTYPE");
        env::remove_var("LANG");

        env::set_var("ARAI_ASCII", "1");
        env::set_var("LC_ALL", "en_US.UTF-8");
        assert!(
            !should_use_unicode(),
            "ARAI_ASCII=1 must override UTF-8 locale"
        );
        env::remove_var("ARAI_ASCII");
        env::remove_var("LC_ALL");
    }

    // AC2: should_use_unicode — NO_UNICODE override.
    #[test]
    fn ac2_no_unicode_forces_ascii() {
        let _guard = glyph_env_lock();
        env::remove_var("ARAI_ASCII");
        env::remove_var("NO_UNICODE");
        env::remove_var("LC_ALL");
        env::remove_var("LC_CTYPE");
        env::remove_var("LANG");

        env::set_var("NO_UNICODE", "yes");
        env::set_var("LC_ALL", "en_US.UTF-8");
        assert!(
            !should_use_unicode(),
            "NO_UNICODE set must override UTF-8 locale"
        );
        env::remove_var("NO_UNICODE");
        env::remove_var("LC_ALL");
    }

    // AC2: no locale → false.
    #[test]
    fn ac2_no_locale_returns_false() {
        let _guard = glyph_env_lock();
        env::remove_var("ARAI_ASCII");
        env::remove_var("NO_UNICODE");
        env::remove_var("LC_ALL");
        env::remove_var("LC_CTYPE");
        env::remove_var("LANG");

        assert!(!should_use_unicode(), "no locale → false");
    }

    // AC2: LC_ALL containing utf-8 → true.
    #[test]
    fn ac2_lc_all_utf8_returns_true() {
        let _guard = glyph_env_lock();
        env::remove_var("ARAI_ASCII");
        env::remove_var("NO_UNICODE");
        env::set_var("LC_ALL", "en_US.UTF-8");
        env::remove_var("LC_CTYPE");
        env::remove_var("LANG");

        assert!(should_use_unicode(), "LC_ALL=en_US.UTF-8 → true");
        env::remove_var("LC_ALL");
    }

    // AC2: LC_ALL absent, LC_CTYPE utf-8 → true.
    #[test]
    fn ac2_lc_ctype_utf8_without_lc_all_returns_true() {
        let _guard = glyph_env_lock();
        env::remove_var("ARAI_ASCII");
        env::remove_var("NO_UNICODE");
        env::remove_var("LC_ALL");
        env::set_var("LC_CTYPE", "C.UTF-8");
        env::remove_var("LANG");

        assert!(should_use_unicode(), "LC_CTYPE=C.UTF-8 (no LC_ALL) → true");
        env::remove_var("LC_CTYPE");
    }

    // AC2: LC_ALL and LC_CTYPE absent, LANG utf-8 → true.
    #[test]
    fn ac2_lang_utf8_without_lc_all_or_lc_ctype_returns_true() {
        let _guard = glyph_env_lock();
        env::remove_var("ARAI_ASCII");
        env::remove_var("NO_UNICODE");
        env::remove_var("LC_ALL");
        env::remove_var("LC_CTYPE");
        env::set_var("LANG", "en_US.utf8");

        assert!(
            should_use_unicode(),
            "LANG=en_US.utf8 (no LC_ALL/LC_CTYPE) → true"
        );
        env::remove_var("LANG");
    }

    // AC2: LC_ALL set to non-UTF-8 blocks LANG from winning.
    #[test]
    fn ac2_lc_all_non_utf8_blocks_lang() {
        let _guard = glyph_env_lock();
        env::remove_var("ARAI_ASCII");
        env::remove_var("NO_UNICODE");
        env::set_var("LC_ALL", "C");
        env::remove_var("LC_CTYPE");
        env::set_var("LANG", "en_US.UTF-8");

        assert!(
            !should_use_unicode(),
            "LC_ALL=C (non-UTF-8) blocks LANG despite UTF-8 → false"
        );
        env::remove_var("LC_ALL");
        env::remove_var("LANG");
    }
}
