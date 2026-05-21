//! `arai migrate` — move data from the legacy `~/.arai` layout to the
//! current `~/.taniwha/arai` layout.
//!
//! The path resolver in `config::resolve_base_dir` already keeps an existing
//! `~/.arai` working (branch 4, with a deprecation notice).  This module
//! handles the actual *move* — opt-in, prompted, idempotent.
//!
//! Flow:
//!   1. Detect: is there a legacy `~/.arai`?  Is the new `~/.taniwha/arai`
//!      already populated?  Is there a marker saying the user declined a
//!      previous prompt?
//!   2. Summarise: count files + total size so the prompt has substance.
//!   3. Prompt (default "no"): explicit y/N, no surprises.
//!   4. Move: `fs::rename` if possible (same filesystem), otherwise
//!      copy-then-delete; on success, leave a marker file at the old
//!      location pointing at the new one for the deprecation window.
//!   5. Decline: drop a `.migrate_declined` marker under `~/.taniwha/arai`
//!      so re-running `arai migrate` (or any auto-trigger from init) won't
//!      re-prompt.
//!
//! Repo-local `<repo>/.arai` is detected as a second candidate but its
//! migration target is also `~/.taniwha/arai` — Arai never wrote project-
//! scoped state into a repo-local directory in practice, so encountering
//! one means a manual artefact; we warn and skip rather than silently
//! merging.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Marker file written under the *new* base after a confirmed decline.
/// Path: `{new_base}/.migrate_declined`.
const DECLINE_MARKER: &str = ".migrate_declined";

/// Marker file left at the *old* base after a successful migration, so a
/// user revisiting `~/.arai` understands where their data went and that the
/// move was intentional.  Path: `{old_base}/MOVED-TO-TANIWHA.txt`.
const MOVED_MARKER: &str = "MOVED-TO-TANIWHA.txt";

/// What `detect` found.  Drives the rest of the flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detected {
    /// Nothing to do — no legacy `~/.arai`, or the user already declined.
    NothingToMigrate { reason: String },
    /// New base is already populated and would be overwritten.  Abort with
    /// a message instead of merging.
    NewBaseInUse { new_base: PathBuf },
    /// Legacy path exists and looks safe to move.
    Found {
        old_base: PathBuf,
        new_base: PathBuf,
        file_count: u64,
        total_bytes: u64,
        repo_local_path: Option<PathBuf>,
    },
}

/// Run the migration end-to-end.  Pure for the prompt/IO seam — the
/// `confirm` closure decides whether to proceed.  Returning closures lets
/// tests skip the interactive prompt without monkey-patching stdin.
///
/// Exits with `Ok(())` on every documented outcome (nothing-to-migrate,
/// decline, successful move).  Returns `Err` only for genuine IO failures
/// that prevent any safe progression.
pub fn run(home_dir: &Path, repo_root: Option<&Path>) -> Result<(), String> {
    run_with_confirm(home_dir, repo_root, prompt_default_no)
}

/// Variant of [`run`] that accepts a confirmation closure — used by tests
/// to drive accept/decline deterministically.
pub fn run_with_confirm<F>(
    home_dir: &Path,
    repo_root: Option<&Path>,
    confirm: F,
) -> Result<(), String>
where
    F: FnOnce(&Detected) -> bool,
{
    let detected = detect(home_dir, repo_root);

    match &detected {
        Detected::NothingToMigrate { reason } => {
            println!("arai migrate: {reason}");
            Ok(())
        }
        Detected::NewBaseInUse { new_base } => {
            println!(
                "arai migrate: {} already exists and is non-empty.\n\
                 Refusing to overwrite.  Remove or back up the new directory \
                 first if you want to re-migrate from the legacy layout.",
                new_base.display(),
            );
            Ok(())
        }
        Detected::Found {
            old_base,
            new_base,
            file_count,
            total_bytes,
            repo_local_path,
        } => {
            print_summary(
                old_base,
                new_base,
                *file_count,
                *total_bytes,
                repo_local_path.as_deref(),
            );
            if !confirm(&detected) {
                write_decline_marker(new_base)?;
                println!(
                    "arai migrate: declined.  Marker written to {} — \
                     re-run `arai migrate` to revisit.",
                    new_base.join(DECLINE_MARKER).display(),
                );
                return Ok(());
            }
            perform_move(old_base, new_base)?;
            println!(
                "arai migrate: moved {} → {} ({} file(s), {} bytes).",
                old_base.display(),
                new_base.display(),
                file_count,
                total_bytes,
            );
            if let Some(repo) = repo_local_path {
                println!(
                    "arai migrate: also detected repo-local {} — not moved \
                     automatically (manual artefact; inspect and delete by hand).",
                    repo.display(),
                );
            }
            Ok(())
        }
    }
}

/// Pure detection — no IO beyond `stat`/`read_dir`.
pub fn detect(home_dir: &Path, repo_root: Option<&Path>) -> Detected {
    let old_base = home_dir.join(".arai");
    let new_base = home_dir.join(".taniwha").join("arai");

    let repo_local_path = repo_root
        .map(|r| r.join(".arai"))
        .filter(|p| p.exists() && p != &old_base);

    if !old_base.exists() {
        return Detected::NothingToMigrate {
            reason: format!("no legacy directory at {}", old_base.display()),
        };
    }

    // Honour a previous decline.
    if new_base.join(DECLINE_MARKER).exists() {
        return Detected::NothingToMigrate {
            reason: format!(
                "previous decline recorded at {} — delete it to re-prompt",
                new_base.join(DECLINE_MARKER).display(),
            ),
        };
    }

    // If the new base already has content beyond the decline marker, we
    // refuse to merge — moving on top would lose either side's state.
    if new_base.exists() && has_real_content(&new_base) {
        return Detected::NewBaseInUse { new_base };
    }

    let (file_count, total_bytes) = walk_size(&old_base);
    Detected::Found {
        old_base,
        new_base,
        file_count,
        total_bytes,
        repo_local_path,
    }
}

fn has_real_content(dir: &Path) -> bool {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == DECLINE_MARKER {
            continue;
        }
        return true;
    }
    false
}

fn walk_size(dir: &Path) -> (u64, u64) {
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                files += 1;
                if let Ok(meta) = entry.metadata() {
                    bytes += meta.len();
                }
            }
        }
    }
    (files, bytes)
}

fn print_summary(
    old_base: &Path,
    new_base: &Path,
    file_count: u64,
    total_bytes: u64,
    repo_local_path: Option<&Path>,
) {
    println!("arai migrate — legacy layout detected.");
    println!("  from: {}", old_base.display());
    println!("    to: {}", new_base.display());
    println!(
        "    {} file(s), {} ({} bytes)",
        file_count,
        human_size(total_bytes),
        total_bytes,
    );
    if let Some(repo) = repo_local_path {
        println!(
            "  also: {} (repo-local; will NOT be moved automatically)",
            repo.display(),
        );
    }
}

fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.2} GiB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MiB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KiB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

fn prompt_default_no(_d: &Detected) -> bool {
    print!("\nProceed with migration? [y/N]: ");
    let _ = io::stdout().flush();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn write_decline_marker(new_base: &Path) -> Result<(), String> {
    fs::create_dir_all(new_base).map_err(|e| format!("create {}: {}", new_base.display(), e))?;
    let marker = new_base.join(DECLINE_MARKER);
    fs::write(
        &marker,
        b"User declined `arai migrate`. Delete this file to re-prompt.\n",
    )
    .map_err(|e| format!("write {}: {}", marker.display(), e))
}

fn perform_move(old_base: &Path, new_base: &Path) -> Result<(), String> {
    if let Some(parent) = new_base.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create {}: {}", parent.display(), e))?;
    }

    // If the new base exists but is empty (or only holds the decline
    // marker), remove it so `rename` has a clear target on platforms that
    // require it.
    if new_base.exists() {
        let marker = new_base.join(DECLINE_MARKER);
        if marker.exists() {
            let _ = fs::remove_file(&marker);
        }
        // Best-effort empty-dir removal; if anything else is present
        // detect() should already have flagged NewBaseInUse.
        let _ = fs::remove_dir(new_base);
    }

    // Try the cheap rename first; fall back to recursive copy + delete
    // when it fails (cross-device on Linux, ACL quirks on Windows).
    if fs::rename(old_base, new_base).is_err() {
        copy_dir_recursive(old_base, new_base)?;
        fs::remove_dir_all(old_base)
            .map_err(|e| format!("remove {}: {}", old_base.display(), e))?;
    }

    // Leave a forwarding marker at the old location.  Best-effort —
    // creating it inside a directory we just removed means we have to
    // recreate the dir, which would be misleading; only write if the
    // parent still exists (i.e. the user has other ~/.arai siblings).
    if let Some(parent) = old_base.parent() {
        if parent.exists() {
            let marker = parent.join(MOVED_MARKER);
            let _ = fs::write(
                &marker,
                format!(
                    "Arai data was moved to {} on `arai migrate`.\n\
                     The legacy ~/.arai directory is no longer used.\n",
                    new_base.display(),
                ),
            );
        }
    }

    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("create {}: {}", dst.display(), e))?;
    let entries = fs::read_dir(src).map_err(|e| format!("read {}: {}", src.display(), e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let dest = dst.join(entry.file_name());
        let ft = entry
            .file_type()
            .map_err(|e| format!("stat {}: {}", path.display(), e))?;
        if ft.is_dir() {
            copy_dir_recursive(&path, &dest)?;
        } else if ft.is_file() {
            fs::copy(&path, &dest)
                .map_err(|e| format!("copy {} → {}: {}", path.display(), dest.display(), e))?;
        }
        // Symlinks and other special files are skipped — Arai doesn't
        // write them, and silently copying them risks loops / surprises.
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_home(label: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "arai_migrate_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn seed_legacy(home: &Path) {
        let old = home.join(".arai");
        fs::create_dir_all(old.join("audit").join("my-project")).unwrap();
        fs::write(
            old.join("audit").join("my-project").join("20260101.jsonl"),
            b"{\"x\":1}\n",
        )
        .unwrap();
        fs::write(old.join("config.toml"), b"# legacy\n").unwrap();
    }

    /// State A — no legacy directory.  `detect` reports nothing to do; `run`
    /// completes silently.
    #[test]
    fn state_a_no_legacy_directory() {
        let home = fresh_home("noop");
        let detected = detect(&home, None);
        assert!(matches!(detected, Detected::NothingToMigrate { .. }));
        // Run must succeed without prompting (closure never invoked).
        run_with_confirm(&home, None, |_| panic!("confirm should not be called")).unwrap();
        fs::remove_dir_all(&home).ok();
    }

    /// State B — legacy present, user accepts.  Files move; old dir gone;
    /// new dir contains the originals.
    #[test]
    fn state_b_accept_moves_files() {
        let home = fresh_home("accept");
        seed_legacy(&home);
        let old = home.join(".arai");
        let new = home.join(".taniwha").join("arai");
        assert!(old.exists());
        assert!(!new.exists());

        run_with_confirm(&home, None, |_| true).unwrap();

        assert!(!old.exists(), "legacy ~/.arai should be gone after accept");
        assert!(new.exists(), "new ~/.taniwha/arai should exist");
        assert!(
            new.join("audit")
                .join("my-project")
                .join("20260101.jsonl")
                .exists(),
            "audit file should have moved to new location",
        );
        assert!(new.join("config.toml").exists());
        fs::remove_dir_all(&home).ok();
    }

    /// State C — legacy present, user declines.  Old dir untouched;
    /// decline marker written under new base; second run sees nothing to do.
    #[test]
    fn state_c_decline_writes_marker_and_does_not_reprompt() {
        let home = fresh_home("decline");
        seed_legacy(&home);
        let old = home.join(".arai");
        let new = home.join(".taniwha").join("arai");

        run_with_confirm(&home, None, |_| false).unwrap();

        assert!(
            old.exists(),
            "legacy ~/.arai must NOT be touched on decline"
        );
        assert!(
            new.join(DECLINE_MARKER).exists(),
            "decline marker should be written under {}",
            new.display(),
        );

        // Second run — confirm closure must NOT be invoked because detect()
        // sees the marker and reports NothingToMigrate.
        run_with_confirm(&home, None, |_| {
            panic!("should not re-prompt after decline")
        })
        .unwrap();
        fs::remove_dir_all(&home).ok();
    }

    /// New base already populated → refuse, don't merge.
    #[test]
    fn refuses_to_overwrite_populated_new_base() {
        let home = fresh_home("populated");
        seed_legacy(&home);
        let new = home.join(".taniwha").join("arai");
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join("important.db"), b"keepme").unwrap();

        let detected = detect(&home, None);
        assert!(matches!(detected, Detected::NewBaseInUse { .. }));

        // Run completes (returns Ok) but does not touch either dir.
        run_with_confirm(&home, None, |_| {
            panic!("should not prompt when new base is in use")
        })
        .unwrap();
        assert!(home.join(".arai").exists());
        assert!(new.join("important.db").exists());
        fs::remove_dir_all(&home).ok();
    }

    /// Repo-local `<repo>/.arai` is detected but reported separately.
    #[test]
    fn repo_local_arai_surfaces_in_detected() {
        let home = fresh_home("repolocal");
        let repo = home.join("some-repo");
        fs::create_dir_all(repo.join(".arai")).unwrap();
        fs::write(repo.join(".arai").join("misc.txt"), b"x").unwrap();
        seed_legacy(&home);

        let detected = detect(&home, Some(&repo));
        match detected {
            Detected::Found {
                repo_local_path: Some(p),
                ..
            } => assert_eq!(p, repo.join(".arai")),
            other => panic!("expected Found with repo_local_path, got {other:?}"),
        }
        fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn human_size_formats_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.00 KiB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.00 MiB");
    }
}
