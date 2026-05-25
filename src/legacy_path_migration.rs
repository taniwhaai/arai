//! Legacy-path migration module.
//!
//! Decides whether a migration offer is warranted on a given `arai init`
//! invocation, conducts the prompt UX when warranted, performs the directory
//! move on accept, and records the decline state on decline.
//!
//! All world-touching operations are mediated by injected capabilities.
//! The module itself has no ambient I/O access.

use crate::config::{DeprecationNotice, ResolvedBaseDir};

// ─── Public types ───────────────────────────────────────────────────────────

/// The outcome of a single [`offer_migration`] call.
///
/// Every call returns exactly one variant.  `String` payloads are the raw
/// `Err` strings from the failing capability; they are not re-formatted.
#[derive(Debug, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// `resolved.notice` was absent.
    SkippedNoNotice,
    /// `resolved.notice` was `DeprecatedEnvVar`.
    SkippedEnvVarNotice,
    /// Decline marker already exists at the documented path.
    SkippedMarkerPresent,
    /// Trigger predicates 1 and 2 held but stdin was non-interactive.
    SkippedNonInteractive,
    /// `dir_stats` callable failed; the description is the `Err` string.
    SkippedSummaryFailed(String),
    /// User declined (any input other than `"y"`/`"Y"`); marker created
    /// successfully.
    PromptedDeclined,
    /// User declined; marker creation failed; the description is the `Err`
    /// string.
    PromptedDeclineMarkerFailed(String),
    /// User accepted (`"y"` or `"Y"`); move succeeded.
    PromptedAccepted {
        file_count: u64,
        total_bytes: u64,
    },
    /// User accepted; move signalled failure; the description is the `Err`
    /// string.
    PromptedAcceptFailed(String),
}

/// Directory statistics returned by the `dir_stats` capability.
#[derive(Debug, PartialEq, Eq)]
pub struct MigrationSummaryStats {
    /// Count of regular files (not directories, not symlinks) in the tree.
    pub file_count: u64,
    /// Sum of byte sizes of those files as reported by their metadata.
    pub total_bytes: u64,
}

/// All injected capabilities for [`offer_migration`].
///
/// Each field is a `Box<dyn Fn(...)>` so that test doubles (closures capturing
/// test state) can be substituted without requiring `'static` or `Copy`.
/// Each `offer_migration` call receives its own `MigrationCapabilities` by
/// value; no sharing wrapper is used.
#[allow(clippy::type_complexity)]
pub struct MigrationCapabilities {
    /// Probe whether a filesystem path exists (as any filesystem object).
    /// Returns `false` on any probe failure (cannot fail at the type level).
    pub path_exists: Box<dyn Fn(&str) -> bool>,
    /// Collect directory statistics for the source path.
    /// Returns `Ok(stats)` or `Err(description)`.
    pub dir_stats: Box<dyn Fn(&str) -> Result<MigrationSummaryStats, String>>,
    /// Move a directory from `source` to `destination`.
    /// Returns `Ok(())` on success or `Err(description)` on failure.
    pub move_dir: Box<dyn Fn(&str, &str) -> Result<(), String>>,
    /// Create the decline-marker file at the given path (idempotent).
    /// Returns `Ok(())` on success or `Err(description)` on failure.
    pub create_marker: Box<dyn Fn(&str) -> Result<(), String>>,
    /// Read one line from the user.
    /// Returns `Ok(line)` or `Err(description)` on read failure.
    pub read_line: Box<dyn Fn() -> Result<String, String>>,
    /// Write a user-facing string to the output channel.  Cannot fail.
    pub write_output: Box<dyn Fn(&str)>,
    /// Return `true` if the input channel is an interactive terminal.
    /// Cannot fail; returns `false` when TTY state cannot be determined.
    pub is_interactive: Box<dyn Fn() -> bool>,
}

// ─── Public entry point ─────────────────────────────────────────────────────

/// Offer the user a migration from the legacy `~/.arai` path to the new
/// canonical `~/.taniwha/arai` path.
///
/// # Parameters
///
/// - `resolved` — the `ResolvedBaseDir` value produced by
///   `base-directory-resolution` and threaded through `Config::load`.
/// - `dest_path` — the new canonical base-directory path string
///   (typically `~/.taniwha/arai`).
/// - `capabilities` — all world-touching operations, injected so tests can
///   substitute doubles without touching the real filesystem or terminal.
///
/// # Evaluation order (fixed, short-circuit)
///
/// 1. Check notice variant — no capability invoked if not `DeprecatedDefaultPath`.
/// 2. Probe decline-marker existence via `path_exists`.
/// 3. Probe TTY via `is_interactive`.
/// 4. Collect directory statistics via `dir_stats`.
/// 5. Write prompt text via `write_output`.
/// 6. Read user input via `read_line`.
/// 7. Act on input: `move_dir` (accept) or `create_marker` (decline).
pub fn offer_migration(
    resolved: &ResolvedBaseDir,
    dest_path: &str,
    capabilities: MigrationCapabilities,
) -> MigrationOutcome {
    // ── Step 1: check notice variant ────────────────────────────────────────
    match &resolved.notice {
        None => return MigrationOutcome::SkippedNoNotice,
        Some(DeprecationNotice::DeprecatedEnvVar(_)) => {
            return MigrationOutcome::SkippedEnvVarNotice;
        }
        Some(DeprecationNotice::DeprecatedDefaultPath(_)) => {
            // Continue to step 2.
        }
    }

    // The source path is `resolved.path` when the notice is
    // `DeprecatedDefaultPath` (the legacy `~/.arai` directory).
    let source_path = &resolved.path;

    // ── Step 2: probe decline-marker existence ───────────────────────────
    let marker_path = format!("{source_path}/.migration_declined");
    if (capabilities.path_exists)(&marker_path) {
        return MigrationOutcome::SkippedMarkerPresent;
    }

    // ── Step 3: probe TTY ────────────────────────────────────────────────
    if !(capabilities.is_interactive)() {
        return MigrationOutcome::SkippedNonInteractive;
    }

    // ── Step 4: collect directory statistics ─────────────────────────────
    let stats = match (capabilities.dir_stats)(source_path) {
        Ok(s) => s,
        Err(desc) => return MigrationOutcome::SkippedSummaryFailed(desc),
    };

    // ── Step 5: write prompt text ─────────────────────────────────────────
    let prompt = format!(
        "Arai detected a legacy data directory that can be moved to the new location.\n\
         \n\
         Source:      {source_path}\n\
         Destination: {dest_path}\n\
         Files:       {} file(s)\n\
         Size:        {} byte(s)\n\
         \n\
         Move now? [y/N] ",
        stats.file_count, stats.total_bytes,
    );
    (capabilities.write_output)(&prompt);

    // ── Step 6: read user input ───────────────────────────────────────────
    // On read failure, treat as empty input (default-decline; AC2).
    let raw = (capabilities.read_line)().unwrap_or_default();

    // Strip a single trailing '\n' and then a single trailing '\r' before
    // comparison.  Only exact "y" or "Y" after stripping are accepted.
    let trimmed = raw.trim_end_matches('\n').trim_end_matches('\r');
    let accepted = trimmed == "y" || trimmed == "Y";

    // ── Step 7: act on input ─────────────────────────────────────────────
    if accepted {
        // Accept branch: attempt the move.
        match (capabilities.move_dir)(source_path, dest_path) {
            Ok(()) => {
                let confirmation = format!(
                    "Migration complete. {} file(s) moved to {dest_path}. \
                     Run 'arai init' again to finish initialisation.",
                    stats.file_count,
                );
                (capabilities.write_output)(&confirmation);
                MigrationOutcome::PromptedAccepted {
                    file_count: stats.file_count,
                    total_bytes: stats.total_bytes,
                }
            }
            Err(desc) => MigrationOutcome::PromptedAcceptFailed(desc),
        }
    } else {
        // Decline branch: write the marker.
        match (capabilities.create_marker)(&marker_path) {
            Ok(()) => MigrationOutcome::PromptedDeclined,
            Err(desc) => MigrationOutcome::PromptedDeclineMarkerFailed(desc),
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    // ── Helpers ─────────────────────────────────────────────────────────────

    const SRC: &str = "/home/user/.arai";
    const DST: &str = "/home/user/.taniwha/arai";

    /// Build a `ResolvedBaseDir` with `DeprecatedDefaultPath` notice.
    fn resolved_deprecated_default() -> ResolvedBaseDir {
        ResolvedBaseDir {
            path: SRC.to_string(),
            notice: Some(DeprecationNotice::DeprecatedDefaultPath(
                "deprecated".to_string(),
            )),
        }
    }

    /// Build a `ResolvedBaseDir` with `DeprecatedEnvVar` notice.
    fn resolved_deprecated_env() -> ResolvedBaseDir {
        ResolvedBaseDir {
            path: SRC.to_string(),
            notice: Some(DeprecationNotice::DeprecatedEnvVar(
                "env deprecated".to_string(),
            )),
        }
    }

    /// Build a `ResolvedBaseDir` with no notice.
    fn resolved_no_notice() -> ResolvedBaseDir {
        ResolvedBaseDir {
            path: SRC.to_string(),
            notice: None,
        }
    }

    // ── AC1 — Prompt fires on DeprecatedDefaultPath + interactive + no marker ──

    #[test]
    fn ac1_prompt_fires_and_contains_required_info() {
        let output = Rc::new(RefCell::new(Vec::<String>::new()));
        let marker_calls = Rc::new(RefCell::new(Vec::<String>::new()));
        let read_called = Rc::new(RefCell::new(false));
        let output_clone = Rc::clone(&output);
        let read_called_clone = Rc::clone(&read_called);

        let caps = MigrationCapabilities {
            path_exists: Box::new(|_| false),
            dir_stats: Box::new(|_| {
                Ok(MigrationSummaryStats {
                    file_count: 7,
                    total_bytes: 2048,
                })
            }),
            move_dir: Box::new(|_, _| panic!("move_dir must not be called")),
            create_marker: Box::new({
                let mc = Rc::clone(&marker_calls);
                move |path| {
                    mc.borrow_mut().push(path.to_string());
                    Ok(())
                }
            }),
            read_line: Box::new(move || {
                *read_called_clone.borrow_mut() = true;
                Ok("N".to_string())
            }),
            write_output: Box::new(move |s| {
                output_clone.borrow_mut().push(s.to_string());
            }),
            is_interactive: Box::new(|| true),
        };

        let outcome = offer_migration(&resolved_deprecated_default(), DST, caps);

        assert_eq!(outcome, MigrationOutcome::PromptedDeclined);

        // write_output was called before read_line.
        assert!(!output.borrow().is_empty(), "write_output must be invoked");
        assert!(*read_called.borrow(), "read_line must be invoked");

        // Prompt text contains source path, dest path, file count, and byte size.
        let full_output = output.borrow().join(" ");
        assert!(
            full_output.contains(SRC),
            "prompt must contain source path; got: {full_output}"
        );
        assert!(
            full_output.contains(DST),
            "prompt must contain destination path; got: {full_output}"
        );
        assert!(
            full_output.contains("7"),
            "prompt must contain file count; got: {full_output}"
        );
        assert!(
            full_output.contains("2048"),
            "prompt must contain total bytes; got: {full_output}"
        );
    }

    // ── AC2 — Default-no: any non-y/Y input is decline ───────────────────────

    fn run_with_input(input: Result<String, String>) -> (MigrationOutcome, Vec<String>) {
        let marker_calls = Rc::new(RefCell::new(Vec::<String>::new()));
        let move_calls = Rc::new(RefCell::new(0u32));
        let mc = Rc::clone(&marker_calls);
        let move_clone = Rc::clone(&move_calls);
        let caps = MigrationCapabilities {
            path_exists: Box::new(|_| false),
            dir_stats: Box::new(|_| {
                Ok(MigrationSummaryStats {
                    file_count: 1,
                    total_bytes: 100,
                })
            }),
            move_dir: Box::new(move |_, _| {
                *move_clone.borrow_mut() += 1;
                Ok(())
            }),
            create_marker: Box::new(move |path| {
                mc.borrow_mut().push(path.to_string());
                Ok(())
            }),
            read_line: Box::new(move || input.clone()),
            write_output: Box::new(|_| {}),
            is_interactive: Box::new(|| true),
        };
        let outcome = offer_migration(&resolved_deprecated_default(), DST, caps);
        let marker_snapshot = marker_calls.borrow().clone();
        (outcome, marker_snapshot)
    }

    #[test]
    fn ac2_default_no_inputs_all_decline() {
        let decline_inputs: Vec<Result<String, String>> = vec![
            Ok("".to_string()),
            Ok("N".to_string()),
            Ok("n".to_string()),
            Ok("no".to_string()),
            Ok("garbage".to_string()),
            Ok(" y".to_string()),
            Ok("yes".to_string()),
            Ok("\n".to_string()),
            Err("read error".to_string()),
        ];

        for input in decline_inputs {
            let label = format!("{input:?}");
            let (outcome, marker_calls) = run_with_input(input);
            assert_eq!(
                outcome,
                MigrationOutcome::PromptedDeclined,
                "expected PromptedDeclined for input {label}"
            );
            assert_eq!(
                marker_calls.len(),
                1,
                "create_marker must be called exactly once for input {label}"
            );
        }
    }

    // ── AC2-accept — Accept paths: only y and Y ───────────────────────────────

    fn run_accept_input(input: &str) -> (MigrationOutcome, u32, u32) {
        let move_calls = Rc::new(RefCell::new(0u32));
        let marker_calls = Rc::new(RefCell::new(0u32));
        let write_after_move = Rc::new(RefCell::new(0u32));
        let move_done = Rc::new(RefCell::new(false));

        let mc = Rc::clone(&move_calls);
        let mkr = Rc::clone(&marker_calls);
        let wam = Rc::clone(&write_after_move);
        let md = Rc::clone(&move_done);
        let md2 = Rc::clone(&move_done);

        let input_str = input.to_string();
        let caps = MigrationCapabilities {
            path_exists: Box::new(|_| false),
            dir_stats: Box::new(|_| {
                Ok(MigrationSummaryStats {
                    file_count: 3,
                    total_bytes: 300,
                })
            }),
            move_dir: Box::new(move |_, _| {
                *mc.borrow_mut() += 1;
                *md.borrow_mut() = true;
                Ok(())
            }),
            create_marker: Box::new(move |_| {
                *mkr.borrow_mut() += 1;
                Ok(())
            }),
            read_line: Box::new(move || Ok(input_str.clone())),
            write_output: Box::new(move |_| {
                if *md2.borrow() {
                    *wam.borrow_mut() += 1;
                }
            }),
            is_interactive: Box::new(|| true),
        };
        let outcome = offer_migration(&resolved_deprecated_default(), DST, caps);
        let move_count = *move_calls.borrow();
        let marker_count = *marker_calls.borrow();
        (outcome, move_count, marker_count)
    }

    #[test]
    fn ac2_accept_only_y_and_y_uppercase() {
        let accept_inputs = ["y", "Y", "y\n", "Y\n"];

        for input in accept_inputs {
            let (outcome, move_count, marker_count) = run_accept_input(input);
            assert!(
                matches!(
                    outcome,
                    MigrationOutcome::PromptedAccepted {
                        file_count: 3,
                        total_bytes: 300,
                    }
                ),
                "expected PromptedAccepted for input {:?}, got {:?}",
                input,
                outcome
            );
            assert_eq!(
                move_count, 1,
                "move_dir must be called exactly once for input {:?}",
                input
            );
            assert_eq!(
                marker_count, 0,
                "create_marker must not be called for accept input {:?}",
                input
            );
        }
    }

    // ── AC3 — Move failure produces accept-failed; no marker written ──────────

    #[test]
    fn ac3_move_failure_produces_accept_failed_no_marker() {
        let marker_calls = Rc::new(RefCell::new(0u32));
        let write_after_move = Rc::new(RefCell::new(0u32));
        let move_done = Rc::new(RefCell::new(false));
        let md = Rc::clone(&move_done);
        let md2 = Rc::clone(&move_done);
        let mkr = Rc::clone(&marker_calls);
        let wam = Rc::clone(&write_after_move);

        let caps = MigrationCapabilities {
            path_exists: Box::new(|_| false),
            dir_stats: Box::new(|_| {
                Ok(MigrationSummaryStats {
                    file_count: 2,
                    total_bytes: 200,
                })
            }),
            move_dir: Box::new(move |_, _| {
                *md.borrow_mut() = true;
                Err("disk full".to_string())
            }),
            create_marker: Box::new(move |_| {
                *mkr.borrow_mut() += 1;
                Ok(())
            }),
            read_line: Box::new(|| Ok("y".to_string())),
            write_output: Box::new(move |_| {
                if *md2.borrow() {
                    *wam.borrow_mut() += 1;
                }
            }),
            is_interactive: Box::new(|| true),
        };

        let outcome = offer_migration(&resolved_deprecated_default(), DST, caps);

        assert_eq!(
            outcome,
            MigrationOutcome::PromptedAcceptFailed("disk full".to_string())
        );
        assert_eq!(
            *marker_calls.borrow(),
            0,
            "create_marker must not be called after move failure"
        );
        assert_eq!(
            *write_after_move.borrow(),
            0,
            "write_output must not be called after move failure"
        );
    }

    // ── AC4 — Decline writes marker at exact path ─────────────────────────────

    #[test]
    fn ac4_decline_writes_marker_at_exact_path() {
        let marker_paths = Rc::new(RefCell::new(Vec::<String>::new()));
        let mp = Rc::clone(&marker_paths);

        let caps = MigrationCapabilities {
            path_exists: Box::new(|_| false),
            dir_stats: Box::new(|_| {
                Ok(MigrationSummaryStats {
                    file_count: 1,
                    total_bytes: 50,
                })
            }),
            move_dir: Box::new(|_, _| panic!("move_dir must not be called on decline")),
            create_marker: Box::new(move |path| {
                mp.borrow_mut().push(path.to_string());
                Ok(())
            }),
            read_line: Box::new(|| Ok("N".to_string())),
            write_output: Box::new(|_| {}),
            is_interactive: Box::new(|| true),
        };

        let outcome = offer_migration(&resolved_deprecated_default(), DST, caps);

        assert_eq!(outcome, MigrationOutcome::PromptedDeclined);
        let paths = marker_paths.borrow();
        assert_eq!(paths.len(), 1, "create_marker must be called exactly once");
        let expected = format!("{SRC}/.migration_declined");
        assert_eq!(
            paths[0], expected,
            "marker path must be <resolved.path>/.migration_declined"
        );
    }

    // ── AC5 — Marker presence short-circuits all other capabilities ───────────

    #[test]
    fn ac5_marker_present_short_circuits_all_other_capabilities() {
        let dir_stats_calls = Rc::new(RefCell::new(0u32));
        let is_interactive_calls = Rc::new(RefCell::new(0u32));
        let read_line_calls = Rc::new(RefCell::new(0u32));
        let write_output_calls = Rc::new(RefCell::new(0u32));
        let create_marker_calls = Rc::new(RefCell::new(0u32));
        let move_dir_calls = Rc::new(RefCell::new(0u32));
        let path_exists_calls = Rc::new(RefCell::new(0u32));

        let ds = Rc::clone(&dir_stats_calls);
        let ii = Rc::clone(&is_interactive_calls);
        let rl = Rc::clone(&read_line_calls);
        let wo = Rc::clone(&write_output_calls);
        let cm = Rc::clone(&create_marker_calls);
        let md = Rc::clone(&move_dir_calls);
        let pe = Rc::clone(&path_exists_calls);

        let caps = MigrationCapabilities {
            path_exists: Box::new(move |_| {
                *pe.borrow_mut() += 1;
                true // marker exists
            }),
            dir_stats: Box::new(move |_| {
                *ds.borrow_mut() += 1;
                Ok(MigrationSummaryStats {
                    file_count: 0,
                    total_bytes: 0,
                })
            }),
            move_dir: Box::new(move |_, _| {
                *md.borrow_mut() += 1;
                Ok(())
            }),
            create_marker: Box::new(move |_| {
                *cm.borrow_mut() += 1;
                Ok(())
            }),
            read_line: Box::new(move || {
                *rl.borrow_mut() += 1;
                Ok(String::new())
            }),
            write_output: Box::new(move |_| {
                *wo.borrow_mut() += 1;
            }),
            is_interactive: Box::new(move || {
                *ii.borrow_mut() += 1;
                true
            }),
        };

        let outcome = offer_migration(&resolved_deprecated_default(), DST, caps);

        assert_eq!(outcome, MigrationOutcome::SkippedMarkerPresent);
        assert_eq!(*path_exists_calls.borrow(), 1, "path_exists must be called exactly once");
        assert_eq!(*dir_stats_calls.borrow(), 0, "dir_stats must not be called");
        assert_eq!(*is_interactive_calls.borrow(), 0, "is_interactive must not be called");
        assert_eq!(*read_line_calls.borrow(), 0, "read_line must not be called");
        assert_eq!(*write_output_calls.borrow(), 0, "write_output must not be called");
        assert_eq!(*create_marker_calls.borrow(), 0, "create_marker must not be called");
        assert_eq!(*move_dir_calls.borrow(), 0, "move_dir must not be called");
    }

    // ── AC6 — No notice / DeprecatedEnvVar notice: no capabilities invoked ────

    fn caps_all_instrumented() -> (
        MigrationCapabilities,
        Rc<RefCell<u32>>, // total call counter
    ) {
        let calls = Rc::new(RefCell::new(0u32));
        let c1 = Rc::clone(&calls);
        let c2 = Rc::clone(&calls);
        let c3 = Rc::clone(&calls);
        let c4 = Rc::clone(&calls);
        let c5 = Rc::clone(&calls);
        let c6 = Rc::clone(&calls);
        let c7 = Rc::clone(&calls);
        let caps = MigrationCapabilities {
            path_exists: Box::new(move |_| {
                *c1.borrow_mut() += 1;
                false
            }),
            dir_stats: Box::new(move |_| {
                *c2.borrow_mut() += 1;
                Ok(MigrationSummaryStats {
                    file_count: 0,
                    total_bytes: 0,
                })
            }),
            move_dir: Box::new(move |_, _| {
                *c3.borrow_mut() += 1;
                Ok(())
            }),
            create_marker: Box::new(move |_| {
                *c4.borrow_mut() += 1;
                Ok(())
            }),
            read_line: Box::new(move || {
                *c5.borrow_mut() += 1;
                Ok(String::new())
            }),
            write_output: Box::new(move |_| {
                *c6.borrow_mut() += 1;
            }),
            is_interactive: Box::new(move || {
                *c7.borrow_mut() += 1;
                true
            }),
        };
        (caps, calls)
    }

    #[test]
    fn ac6a_no_notice_no_capabilities_invoked() {
        let (caps, calls) = caps_all_instrumented();
        let outcome = offer_migration(&resolved_no_notice(), DST, caps);
        assert_eq!(outcome, MigrationOutcome::SkippedNoNotice);
        assert_eq!(*calls.borrow(), 0, "no capability must be invoked when notice is absent");
    }

    #[test]
    fn ac6b_deprecated_env_var_notice_no_capabilities_invoked() {
        let (caps, calls) = caps_all_instrumented();
        let outcome = offer_migration(&resolved_deprecated_env(), DST, caps);
        assert_eq!(outcome, MigrationOutcome::SkippedEnvVarNotice);
        assert_eq!(
            *calls.borrow(),
            0,
            "no capability must be invoked when notice is DeprecatedEnvVar"
        );
    }

    // ── AC-noninteractive — Non-interactive stdin: short-circuit without marker ─

    #[test]
    fn ac_noninteractive_skips_without_marker() {
        let path_exists_calls = Rc::new(RefCell::new(0u32));
        let is_interactive_calls = Rc::new(RefCell::new(0u32));
        let other_calls = Rc::new(RefCell::new(0u32));

        let pe = Rc::clone(&path_exists_calls);
        let ii = Rc::clone(&is_interactive_calls);
        let oc1 = Rc::clone(&other_calls);
        let oc2 = Rc::clone(&other_calls);
        let oc3 = Rc::clone(&other_calls);
        let oc4 = Rc::clone(&other_calls);
        let oc5 = Rc::clone(&other_calls);

        let caps = MigrationCapabilities {
            path_exists: Box::new(move |_| {
                *pe.borrow_mut() += 1;
                false // marker does not exist
            }),
            dir_stats: Box::new(move |_| {
                *oc1.borrow_mut() += 1;
                Ok(MigrationSummaryStats {
                    file_count: 0,
                    total_bytes: 0,
                })
            }),
            move_dir: Box::new(move |_, _| {
                *oc2.borrow_mut() += 1;
                Ok(())
            }),
            create_marker: Box::new(move |_| {
                *oc3.borrow_mut() += 1;
                Ok(())
            }),
            read_line: Box::new(move || {
                *oc4.borrow_mut() += 1;
                Ok(String::new())
            }),
            write_output: Box::new(move |_| {
                *oc5.borrow_mut() += 1;
            }),
            is_interactive: Box::new(move || {
                *ii.borrow_mut() += 1;
                false // not interactive
            }),
        };

        let outcome = offer_migration(&resolved_deprecated_default(), DST, caps);

        assert_eq!(outcome, MigrationOutcome::SkippedNonInteractive);
        assert_eq!(*path_exists_calls.borrow(), 1, "path_exists must be called exactly once");
        assert_eq!(*is_interactive_calls.borrow(), 1, "is_interactive must be called exactly once");
        assert_eq!(
            *other_calls.borrow(),
            0,
            "dir_stats/move_dir/create_marker/read_line/write_output must not be called"
        );
    }

    // ── AC-statsfail — Statistics failure prevents prompt ─────────────────────

    #[test]
    fn ac_statsfail_prevents_prompt() {
        let other_calls = Rc::new(RefCell::new(0u32));
        let oc1 = Rc::clone(&other_calls);
        let oc2 = Rc::clone(&other_calls);
        let oc3 = Rc::clone(&other_calls);
        let oc4 = Rc::clone(&other_calls);

        let caps = MigrationCapabilities {
            path_exists: Box::new(|_| false),
            dir_stats: Box::new(|_| Err("permission denied".to_string())),
            move_dir: Box::new(move |_, _| {
                *oc1.borrow_mut() += 1;
                Ok(())
            }),
            create_marker: Box::new(move |_| {
                *oc2.borrow_mut() += 1;
                Ok(())
            }),
            read_line: Box::new(move || {
                *oc3.borrow_mut() += 1;
                Ok(String::new())
            }),
            write_output: Box::new(move |_| {
                *oc4.borrow_mut() += 1;
            }),
            is_interactive: Box::new(|| true),
        };

        let outcome = offer_migration(&resolved_deprecated_default(), DST, caps);

        assert_eq!(
            outcome,
            MigrationOutcome::SkippedSummaryFailed("permission denied".to_string())
        );
        assert_eq!(
            *other_calls.borrow(),
            0,
            "read_line/write_output/create_marker/move_dir must not be called after dir_stats failure"
        );
    }

    // ── AC-determinism — Repeated invocations with identical inputs yield
    //    identical results ────────────────────────────────────────────────────

    #[test]
    fn ac_determinism_repeated_calls_identical() {
        let make_caps = || MigrationCapabilities {
            path_exists: Box::new(|_| false),
            dir_stats: Box::new(|_| {
                Ok(MigrationSummaryStats {
                    file_count: 4,
                    total_bytes: 400,
                })
            }),
            move_dir: Box::new(|_, _| panic!("move_dir must not be called")),
            create_marker: Box::new(|_| Ok(())),
            read_line: Box::new(|| Ok("N".to_string())),
            write_output: Box::new(|_| {}),
            is_interactive: Box::new(|| true),
        };

        let r1 = offer_migration(&resolved_deprecated_default(), DST, make_caps());
        let r2 = offer_migration(&resolved_deprecated_default(), DST, make_caps());

        assert_eq!(
            r1, r2,
            "identical inputs must produce identical outcomes"
        );
    }

    // ── Decline-marker failure ────────────────────────────────────────────────

    #[test]
    fn decline_marker_failure_returns_marker_failed() {
        let caps = MigrationCapabilities {
            path_exists: Box::new(|_| false),
            dir_stats: Box::new(|_| {
                Ok(MigrationSummaryStats {
                    file_count: 1,
                    total_bytes: 100,
                })
            }),
            move_dir: Box::new(|_, _| panic!("move_dir must not be called")),
            create_marker: Box::new(|_| Err("permission denied".to_string())),
            read_line: Box::new(|| Ok("N".to_string())),
            write_output: Box::new(|_| {}),
            is_interactive: Box::new(|| true),
        };

        let outcome = offer_migration(&resolved_deprecated_default(), DST, caps);

        assert_eq!(
            outcome,
            MigrationOutcome::PromptedDeclineMarkerFailed("permission denied".to_string())
        );
    }

    // ── read_line failure treated as decline ─────────────────────────────────

    #[test]
    fn read_line_failure_treated_as_decline() {
        let marker_calls = Rc::new(RefCell::new(0u32));
        let mc = Rc::clone(&marker_calls);

        let caps = MigrationCapabilities {
            path_exists: Box::new(|_| false),
            dir_stats: Box::new(|_| {
                Ok(MigrationSummaryStats {
                    file_count: 1,
                    total_bytes: 100,
                })
            }),
            move_dir: Box::new(|_, _| panic!("move_dir must not be called")),
            create_marker: Box::new(move |_| {
                *mc.borrow_mut() += 1;
                Ok(())
            }),
            read_line: Box::new(|| Err("unexpected EOF".to_string())),
            write_output: Box::new(|_| {}),
            is_interactive: Box::new(|| true),
        };

        let outcome = offer_migration(&resolved_deprecated_default(), DST, caps);

        assert_eq!(outcome, MigrationOutcome::PromptedDeclined);
        assert_eq!(
            *marker_calls.borrow(),
            1,
            "create_marker must be called exactly once on read_line failure"
        );
    }

    // ── Confirmation line contains required phrase ────────────────────────────

    #[test]
    fn confirmation_line_contains_run_arai_init_again() {
        let confirmations = Rc::new(RefCell::new(Vec::<String>::new()));
        let move_done = Rc::new(RefCell::new(false));
        let conf = Rc::clone(&confirmations);
        let md = Rc::clone(&move_done);
        let md2 = Rc::clone(&move_done);

        let caps = MigrationCapabilities {
            path_exists: Box::new(|_| false),
            dir_stats: Box::new(|_| {
                Ok(MigrationSummaryStats {
                    file_count: 2,
                    total_bytes: 200,
                })
            }),
            move_dir: Box::new(move |_, _| {
                *md.borrow_mut() = true;
                Ok(())
            }),
            create_marker: Box::new(|_| panic!("create_marker must not be called on accept")),
            read_line: Box::new(|| Ok("y".to_string())),
            write_output: Box::new(move |s| {
                if *md2.borrow() {
                    conf.borrow_mut().push(s.to_string());
                }
            }),
            is_interactive: Box::new(|| true),
        };

        let outcome = offer_migration(&resolved_deprecated_default(), DST, caps);

        assert!(
            matches!(outcome, MigrationOutcome::PromptedAccepted { .. }),
            "expected PromptedAccepted"
        );
        let confs = confirmations.borrow();
        assert!(!confs.is_empty(), "confirmation write_output must be called after move");
        let full = confs.join(" ");
        assert!(
            full.contains("arai init"),
            "confirmation must instruct user to run 'arai init' again; got: {full}"
        );
    }

    // ── move_dir invoked with correct (source, dest) arguments ───────────────

    #[test]
    fn move_dir_invoked_with_correct_args() {
        let move_args = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
        let ma = Rc::clone(&move_args);

        let caps = MigrationCapabilities {
            path_exists: Box::new(|_| false),
            dir_stats: Box::new(|_| {
                Ok(MigrationSummaryStats {
                    file_count: 1,
                    total_bytes: 50,
                })
            }),
            move_dir: Box::new(move |src, dst| {
                ma.borrow_mut().push((src.to_string(), dst.to_string()));
                Ok(())
            }),
            create_marker: Box::new(|_| panic!("must not be called")),
            read_line: Box::new(|| Ok("y".to_string())),
            write_output: Box::new(|_| {}),
            is_interactive: Box::new(|| true),
        };

        offer_migration(&resolved_deprecated_default(), DST, caps);

        let args = move_args.borrow();
        assert_eq!(args.len(), 1, "move_dir must be called exactly once");
        assert_eq!(args[0].0, SRC, "source path must be resolved.path");
        assert_eq!(args[0].1, DST, "destination path must be dest_path");
    }
}
