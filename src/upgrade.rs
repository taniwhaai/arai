use std::path::PathBuf;

const REPO: &str = "taniwhaai/arai";

/// Whether this binary was built with the enrich feature.
pub fn is_full_binary() -> bool {
    cfg!(feature = "enrich")
}

/// Get the current binary's variant name.
pub fn current_variant() -> &'static str {
    if is_full_binary() { "full" } else { "lean" }
}

/// Run the upgrade flow.
pub fn run(full: bool, lean: bool) -> Result<(), String> {
    let target_variant = if full {
        "full"
    } else if lean {
        "lean"
    } else {
        current_variant() // Same variant, just update version
    };

    let platform = detect_platform()?;
    let version = fetch_latest_version()?;

    let binary_name = if target_variant == "full" {
        format!("arai-full-{platform}")
    } else {
        format!("arai-{platform}")
    };

    println!("  Upgrading to {version} ({target_variant})...");
    println!("  Downloading {binary_name}...");

    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Could not determine current binary path: {e}"))?;

    let url = format!(
        "https://github.com/{REPO}/releases/download/{version}/{binary_name}"
    );

    let tmp_path = current_exe.with_extension("tmp");

    download_file(&url, &tmp_path)?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to set permissions: {e}"))?;
    }

    // Atomic replace: rename tmp over current binary
    // On some systems we need to remove the old binary first
    let backup_path = current_exe.with_extension("bak");
    if backup_path.exists() {
        std::fs::remove_file(&backup_path).ok();
    }

    // Move current → backup, tmp → current
    std::fs::rename(&current_exe, &backup_path)
        .map_err(|e| format!("Failed to backup current binary: {e}"))?;

    match std::fs::rename(&tmp_path, &current_exe) {
        Ok(_) => {
            // Clean up backup
            std::fs::remove_file(&backup_path).ok();
            println!("  \u{2713} Upgraded to {version} ({target_variant})");
            Ok(())
        }
        Err(e) => {
            // Restore backup
            std::fs::rename(&backup_path, &current_exe).ok();
            std::fs::remove_file(&tmp_path).ok();
            Err(format!("Failed to replace binary: {e}"))
        }
    }
}

/// Offer to upgrade to full binary when --enrich is used on lean.
/// Returns true if the user accepted and upgrade succeeded.
#[allow(dead_code)]
pub fn offer_upgrade_to_full() -> Result<bool, String> {
    if is_full_binary() {
        return Ok(false); // Already full
    }

    eprintln!("  Enrichment requires the full binary (with ONNX runtime).");
    eprintln!("  Current binary: lean (~9MB)");
    eprintln!("  Full binary: ~32MB");
    eprintln!();
    eprint!("  Download full binary now? [Y/n] ");

    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| format!("Failed to read input: {e}"))?;

    let input = input.trim().to_lowercase();
    if input.is_empty() || input == "y" || input == "yes" {
        run(true, false)?;
        println!();
        println!("  Re-run `arai scan --enrich` to continue.");
        Ok(true)
    } else {
        println!("  Skipped. You can upgrade later with: arai upgrade --full");
        Ok(false)
    }
}

fn detect_platform() -> Result<String, String> {
    let os = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        return Err("Unsupported OS".to_string());
    };

    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        return Err("Unsupported architecture".to_string());
    };

    Ok(format!("{os}-{arch}"))
}

fn fetch_latest_version() -> Result<String, String> {
    let output = std::process::Command::new("curl")
        .args(["-sSf", &format!("https://api.github.com/repos/{REPO}/releases/latest")])
        .output()
        .map_err(|e| format!("Failed to fetch version: {e}"))?;

    if !output.status.success() {
        return Err("Failed to fetch latest version from GitHub".to_string());
    }

    let body = String::from_utf8_lossy(&output.stdout);

    // Parse with serde_json for reliability
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse GitHub API response: {e}"))?;

    json.get("tag_name")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "No tag_name in GitHub API response".to_string())
}

fn download_file(url: &str, dest: &PathBuf) -> Result<(), String> {
    let output = std::process::Command::new("curl")
        .args(["-sL", "-o", &dest.to_string_lossy(), url])
        .output()
        .map_err(|e| format!("Failed to download: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Download failed: {stderr}"));
    }

    let meta = std::fs::metadata(dest)
        .map_err(|e| format!("Downloaded file not found: {e}"))?;

    if meta.len() < 10000 {
        return Err(format!(
            "Downloaded file is suspiciously small ({} bytes). Check the release URL.",
            meta.len()
        ));
    }

    Ok(())
}
