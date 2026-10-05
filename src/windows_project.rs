//! Windows project identity and one-time relocation of legacy path-hashed state.
use super::slug_for_path;
use fs2::FileExt;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

pub(super) fn canonical_root(root: &Path) -> Result<PathBuf, String> {
    let path = fs::canonicalize(root).map_err(|e| {
        format!(
            "Could not resolve project directory {}: {e}",
            root.display()
        )
    })?;
    Ok(without_verbatim_prefix(&path))
}

fn without_verbatim_prefix(path: &Path) -> PathBuf {
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    let prefix: Vec<u16> = r"\\?\".encode_utf16().collect();
    let unc: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
    if wide.starts_with(&unc) {
        let mut ordinary = vec![b'\\' as u16; 2];
        ordinary.extend_from_slice(&wide[unc.len()..]);
        PathBuf::from(OsString::from_wide(&ordinary))
    } else if wide.starts_with(&prefix) {
        PathBuf::from(OsString::from_wide(&wide[prefix.len()..]))
    } else {
        path.to_path_buf()
    }
}

fn short_root(root: &Path) -> Option<PathBuf> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetShortPathNameW(long: *const u16, short: *mut u16, size: u32) -> u32;
    }
    let input: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: input is NUL-terminated; the first call only requests capacity.
    let size = unsafe { GetShortPathNameW(input.as_ptr(), std::ptr::null_mut(), 0) };
    if size == 0 {
        return None; // 8.3 aliases may be disabled on this volume.
    }
    let mut output = vec![0; size as usize];
    // SAFETY: output has exactly the capacity passed to Windows.
    let written = unsafe { GetShortPathNameW(input.as_ptr(), output.as_mut_ptr(), size) };
    if written == 0 || written >= size {
        return None;
    }
    Some(PathBuf::from(OsString::from_wide(
        &output[..written as usize],
    )))
}

pub(super) fn migrate_store(base: &Path, raw: &Path, canonical: &Path) -> Result<(), String> {
    let projects = base.join("projects");
    if !projects.exists() {
        return Ok(());
    }
    let slug = slug_for_path(canonical);
    let destination = projects.join(&slug);
    if destination.join("arai.db").is_file() {
        return Ok(());
    }
    // Canonical identity is shared by every spelling. Serialize first-open
    // migration; never copy a live SQLite database without its WAL/SHM files.
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(projects.join(format!(".{slug}.migration.lock")))
        .map_err(|e| format!("Could not open project migration lock: {e}"))?;
    FileExt::lock_exclusive(&lock).map_err(|e| format!("Could not lock project migration: {e}"))?;
    if destination.join("arai.db").is_file() {
        return Ok(());
    }
    let mut candidates = vec![raw.to_path_buf()];
    // %TEMP% often abbreviates an ancestor (e.g. RUNNER~1) while the
    // checkout name itself stays long. Probe those prefix spellings as
    // well as the fully shortened root, independent of the caller's cwd.
    for ancestor in canonical.ancestors() {
        if let Some(short) = short_root(ancestor) {
            let suffix = canonical.strip_prefix(ancestor).expect("ancestor prefix");
            candidates.push(if suffix.as_os_str().is_empty() {
                short
            } else {
                short.join(suffix)
            });
        }
    }
    let mut legacy = Vec::new();
    for root in candidates {
        let old_slug = slug_for_path(&root);
        if old_slug != slug
            && projects.join(&old_slug).join("arai.db").is_file()
            && !legacy.iter().any(|(_, s)| s == &old_slug)
        {
            legacy.push((root, old_slug));
        }
    }
    if legacy.len() > 1 {
        return Err(format!(
            "Multiple legacy Arai stores match {}: {}. Preserve and reconcile these stores before retrying.",
            canonical.display(),
            legacy.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }
    let Some((old_root, old_slug)) = legacy.pop() else {
        return Ok(());
    };
    let source = projects.join(&old_slug);
    let old_audit = base.join("audit").join(&old_slug);
    let new_audit = base.join("audit").join(&slug);
    if old_audit.exists() && new_audit.exists() {
        return Err(format!(
            "Both legacy and canonical audit directories exist: {} and {}. Reconcile them before migrating; audit chains cannot be merged automatically.",
            old_audit.display(), new_audit.display()
        ));
    }
    // Only remove an empty destination. Never replace unrelated state.
    if destination.exists() {
        fs::remove_dir(&destination).map_err(|e| {
            format!(
                "Cannot migrate into {} (expected an empty directory): {e}",
                destination.display()
            )
        })?;
    }
    {
        let store = crate::store::Store::open(&source.join("arai.db"))?;
        store.relocate_project_sources(&old_root, canonical)?;
    }
    // Audit first: if interrupted between renames, the old DB remains a
    // migration candidate and the next invocation completes the second move.
    if old_audit.exists() {
        fs::rename(&old_audit, &new_audit)
            .map_err(|e| format!("Could not migrate audit directory: {e}"))?;
    }
    fs::rename(&source, &destination)
        .map_err(|e| format!("Could not migrate project store {}: {e}", source.display()))?;
    eprintln!("Migrated Arai project state from {old_slug} to {slug}.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_disk_and_unc_prefixes_without_losing_unc_root() {
        assert_eq!(
            without_verbatim_prefix(Path::new(r"\\?\C:\Repo")),
            Path::new(r"C:\Repo")
        );
        assert_eq!(
            without_verbatim_prefix(Path::new(r"\\?\UNC\server\share\Repo")),
            Path::new(r"\\server\share\Repo")
        );
    }
}
