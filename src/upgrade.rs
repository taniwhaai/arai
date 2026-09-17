use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

const REPO: &str = "taniwhaai/arai";

/// Whether this binary was built with the enrich feature.
pub fn is_full_binary() -> bool {
    cfg!(feature = "enrich")
}

/// Get the current binary's variant name.
pub fn current_variant() -> &'static str {
    if is_full_binary() {
        "full"
    } else {
        "lean"
    }
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

    let binary_name = release_asset_name(
        std::env::consts::OS,
        std::env::consts::ARCH,
        target_variant == "full",
    )?;
    let version = fetch_latest_version()?;

    println!("  Upgrading to {version} ({target_variant})...");
    println!("  Downloading {binary_name}...");

    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Could not determine current binary path: {e}"))?;

    let release_url = format!("https://github.com/{REPO}/releases/download/{version}");
    install_verified_binary(&current_exe, &binary_name, |asset, destination| {
        download_file(&format!("{release_url}/{asset}"), destination)
    })?;
    println!("  \u{2713} Upgraded to {version} ({target_variant})");
    Ok(())
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
        .map_err(|e| format!("Could not read input: {e}"))?;

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

fn release_asset_name(os: &str, arch: &str, full: bool) -> Result<String, String> {
    let platform = match (os, arch) {
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("macos", "x86_64") => "darwin-x86_64",
        ("macos", "aarch64") => "darwin-aarch64",
        ("windows", "x86_64") => "windows-x86_64.exe",
        _ => return Err(format!("No released binary for {os}-{arch}")),
    };
    let variant = if full { "arai-full" } else { "arai" };
    Ok(format!("{variant}-{platform}"))
}

fn fetch_latest_version() -> Result<String, String> {
    let output = std::process::Command::new("curl")
        .args([
            "-sSf",
            &format!("https://api.github.com/repos/{REPO}/releases/latest"),
        ])
        .output()
        .map_err(|e| format!("Could not fetch version: {e}"))?;

    if !output.status.success() {
        return Err("Could not fetch latest version from GitHub.".to_string());
    }

    let body = String::from_utf8_lossy(&output.stdout);

    // Parse with serde_json for reliability
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Could not parse GitHub API response: {e}"))?;

    json.get("tag_name")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "No tag_name in GitHub API response".to_string())
}

fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    let output = std::process::Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--output",
        ])
        .arg(dest)
        .arg(url)
        .output()
        .map_err(|e| format!("Could not download: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Download failed: {stderr}"));
    }

    Ok(())
}

fn expected_checksum(checksums: &str, asset: &str) -> Result<String, String> {
    let mut expected = None;
    for line in checksums.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields
            .get(1)
            .map(|name| name.strip_prefix('*').unwrap_or(name))
            != Some(asset)
        {
            continue;
        }
        if fields.len() != 2
            || fields[0].len() != 64
            || !fields[0].bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!("Invalid checksum for {asset} in checksums.txt"));
        }
        if expected.replace(fields[0].to_ascii_lowercase()).is_some() {
            return Err(format!("Duplicate checksum for {asset} in checksums.txt"));
        }
    }
    expected.ok_or_else(|| format!("No checksum for {asset} in checksums.txt"))
}

fn verify_checksum(binary: &Path, checksums: &Path, asset: &str) -> Result<(), String> {
    let checksums = std::fs::read_to_string(checksums)
        .map_err(|error| format!("Could not read checksums.txt: {error}"))?;
    let expected = expected_checksum(&checksums, asset)?;
    let mut file = std::fs::File::open(binary)
        .map_err(|error| format!("Could not read downloaded binary: {error}"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("Could not hash downloaded binary: {error}"))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    let actual: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual != expected {
        return Err(format!(
            "Checksum mismatch for {asset}; current binary was not changed"
        ));
    }
    Ok(())
}

/// A uniquely owned directory beside the executable keeps renames on the same
/// filesystem and avoids overwriting another upgrade's files or an existing .bak.
struct UpgradeFiles {
    directory: PathBuf,
    preserve_backup: bool,
}

impl UpgradeFiles {
    fn new(current: &Path) -> Result<Self, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = current
            .parent()
            .ok_or("Executable has no parent directory")?;
        for _ in 0..100 {
            let directory = parent.join(format!(
                ".arai-upgrade-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&directory) {
                Ok(()) => {
                    return Ok(Self {
                        directory,
                        preserve_backup: false,
                    })
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("Could not stage upgrade: {error}")),
            }
        }
        Err("Could not allocate an unused upgrade directory".into())
    }
}

impl Drop for UpgradeFiles {
    fn drop(&mut self) {
        // Delete only the files owned by this attempt. If rollback failed,
        // preserve the original executable for recovery instead of deleting it.
        for name in ["binary", "checksums.txt"] {
            std::fs::remove_file(self.directory.join(name)).ok();
        }
        if !self.preserve_backup {
            std::fs::remove_file(self.directory.join("previous")).ok();
        }
        std::fs::remove_dir(&self.directory).ok();
    }
}

fn install_verified_binary(
    current: &Path,
    asset: &str,
    mut download: impl FnMut(&str, &Path) -> Result<(), String>,
) -> Result<(), String> {
    let mut files = UpgradeFiles::new(current)?;
    let binary = files.directory.join("binary");
    let checksums = files.directory.join("checksums.txt");
    let backup = files.directory.join("previous");
    download(asset, &binary)?;
    download("checksums.txt", &checksums)?;
    verify_checksum(&binary, &checksums, asset)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("Could not set permissions: {error}"))?;
    }

    std::fs::rename(current, &backup)
        .map_err(|error| format!("Could not back up current binary: {error}"))?;
    if let Err(error) = std::fs::rename(&binary, current) {
        if let Err(restore) = std::fs::rename(&backup, current) {
            files.preserve_backup = true;
            return Err(format!(
                "Could not replace binary: {error}; restore failed: {restore}. Original binary retained at {}",
                backup.display()
            ));
        }
        return Err(format!(
            "Could not replace binary: {error}; original binary restored"
        ));
    }
    // Windows can retain a running executable until this process exits. Leave
    // that backup discoverable if deletion is denied; downloaded files still
    // get cleaned up by UpgradeFiles.
    if let Err(error) = std::fs::remove_file(&backup) {
        eprintln!(
            "  Previous binary retained at {}: {error}",
            backup.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASSET: &str = "arai-windows-x86_64.exe";
    const NEW: &[u8] = b"verified replacement binary";

    fn manifest(bytes: &[u8], asset: &str) -> String {
        let hash: String = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        format!("{hash}  {asset}\n")
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir()
                .join(format!("arai_upgrade_test_{}_{stamp}", std::process::id()));
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("arai.exe"), b"original binary").unwrap();
            std::fs::write(root.join("arai.bak"), b"unrelated backup").unwrap();
            Self(root)
        }
        fn current(&self) -> PathBuf {
            self.0.join("arai.exe")
        }
        fn assert_clean(&self) {
            assert_eq!(
                std::fs::read(self.0.join("arai.bak")).unwrap(),
                b"unrelated backup"
            );
            assert_eq!(std::fs::read_dir(&self.0).unwrap().count(), 2);
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn released_asset_names_cover_both_variants_of_exactly_five_platforms() {
        for (os, arch, suffix) in [
            ("linux", "x86_64", "linux-x86_64"),
            ("linux", "aarch64", "linux-aarch64"),
            ("macos", "x86_64", "darwin-x86_64"),
            ("macos", "aarch64", "darwin-aarch64"),
            ("windows", "x86_64", "windows-x86_64.exe"),
        ] {
            assert_eq!(
                release_asset_name(os, arch, false).unwrap(),
                format!("arai-{suffix}")
            );
            assert_eq!(
                release_asset_name(os, arch, true).unwrap(),
                format!("arai-full-{suffix}")
            );
        }
        for (os, arch) in [
            ("windows", "aarch64"),
            ("linux", "x86"),
            ("freebsd", "x86_64"),
        ] {
            assert!(release_asset_name(os, arch, false).is_err());
            assert!(release_asset_name(os, arch, true).is_err());
        }
    }

    #[test]
    fn checksum_failures_never_replace_current_or_leave_temporary_files() {
        let valid = manifest(NEW, ASSET);
        for checksums in [
            String::new(),
            manifest(NEW, "arai-full-windows-x86_64.exe"),
            manifest(b"different binary", ASSET),
            format!("invalid-hash  {ASSET}\n"),
            format!("{valid}{valid}"),
            format!("{}  {ASSET} extra-field\n", "0".repeat(64)),
        ] {
            let fixture = Fixture::new();
            let result = install_verified_binary(&fixture.current(), ASSET, |asset, dest| {
                let bytes = if asset == ASSET {
                    NEW
                } else {
                    checksums.as_bytes()
                };
                std::fs::write(dest, bytes).map_err(|error| error.to_string())
            });
            assert!(result.is_err(), "accepted {checksums:?}");
            assert_eq!(
                std::fs::read(fixture.current()).unwrap(),
                b"original binary"
            );
            fixture.assert_clean();
        }
    }

    #[test]
    fn partial_download_failures_leave_current_and_cleanup_both_assets() {
        for failed_asset in [ASSET, "checksums.txt"] {
            let fixture = Fixture::new();
            let result = install_verified_binary(&fixture.current(), ASSET, |asset, dest| {
                std::fs::write(dest, NEW).unwrap();
                if asset == failed_asset {
                    Err("HTTP download failed".into())
                } else {
                    Ok(())
                }
            });
            assert!(result.is_err());
            assert_eq!(
                std::fs::read(fixture.current()).unwrap(),
                b"original binary"
            );
            fixture.assert_clean();
        }
    }

    #[test]
    fn verified_download_replaces_current_and_cleans_staging() {
        let fixture = Fixture::new();
        let checksums = manifest(NEW, ASSET);
        install_verified_binary(&fixture.current(), ASSET, |asset, dest| {
            std::fs::write(
                dest,
                if asset == ASSET {
                    NEW
                } else {
                    checksums.as_bytes()
                },
            )
            .map_err(|error| error.to_string())
        })
        .unwrap();
        assert_eq!(std::fs::read(fixture.current()).unwrap(), NEW);
        fixture.assert_clean();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(fixture.current())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
        }
    }

    #[test]
    fn checksum_parser_accepts_standard_binary_mode_and_uppercase_hashes() {
        let hash = manifest(NEW, ASSET)
            .split_whitespace()
            .next()
            .unwrap()
            .to_string();
        let checksum = format!("{} *{ASSET}\r\n", hash.to_uppercase());
        assert_eq!(expected_checksum(&checksum, ASSET).unwrap(), hash);
    }

    #[test]
    fn http_error_response_is_rejected_even_when_larger_than_a_binary_size_heuristic() {
        use std::io::Write;
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        use std::time::{Duration, Instant};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/missing-release", listener.local_addr().unwrap());
        let download_finished = Arc::new(AtomicBool::new(false));
        let server_finished = Arc::clone(&download_finished);
        let server = std::thread::spawn(move || -> Result<(), String> {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if server_finished.load(Ordering::Acquire) {
                            return Err("curl exited without reaching the loopback fixture; check curl availability and localhost proxy settings".into());
                        }
                        if Instant::now() >= deadline {
                            return Err("curl did not reach the loopback fixture within 5 seconds; check localhost proxy settings".into());
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => return Err(format!("Loopback accept failed: {error}")),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .map_err(|error| format!("Could not set fixture read timeout: {error}"))?;
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .map_err(|error| format!("Could not set fixture write timeout: {error}"))?;
            let mut request = [0u8; 4096];
            let count = stream.read(&mut request).map_err(|error| {
                format!("Loopback request was not received within 2 seconds: {error}")
            })?;
            if count == 0 {
                return Err("curl closed the loopback connection without a request".into());
            }
            let body = "x".repeat(20_000);
            // Curl may close as soon as it reads the failing status.
            write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .ok();
            Ok(())
        });
        let fixture = Fixture::new();
        let result = install_verified_binary(&fixture.current(), ASSET, |_, destination| {
            download_file(&url, destination)
        });
        // Missing curl and proxy failures can return without connecting. Wake
        // the accept loop promptly instead of waiting for its deadline.
        download_finished.store(true, Ordering::Release);
        server
            .join()
            .expect("loopback fixture thread panicked")
            .unwrap_or_else(|error| panic!("{error}; downloader result: {result:?}"));
        assert!(result.unwrap_err().contains("Download failed"));
        assert_eq!(
            std::fs::read(fixture.current()).unwrap(),
            b"original binary"
        );
        fixture.assert_clean();
    }
}
