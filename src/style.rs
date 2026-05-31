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
}
