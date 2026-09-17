#!/usr/bin/env node
"use strict";

const { execFileSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const os = require("os");

const REPO = "taniwhaai/arai";

function main() {
  const platform = detectPlatform();
  const version = getVersion();
  const binaryName = getBinaryDownloadName(platform);
  const url = `https://github.com/${REPO}/releases/download/v${version}/${binaryName}`;
  const checksumsUrl = `https://github.com/${REPO}/releases/download/v${version}/checksums.txt`;
  const dest = path.join(__dirname, "bin", getLocalBinaryName());
  const checksumsPath = path.join(__dirname, "bin", "checksums.txt");

  console.log(`  Downloading arai v${version} for ${platform}...`);

  fs.mkdirSync(path.dirname(dest), { recursive: true });

  // Replace only the native payload; bin/arai remains the npm launcher.
  if (fs.existsSync(dest)) {
    fs.unlinkSync(dest);
  }

  try {
    execFileSync("curl", ["-sL", "--fail", "-o", dest, url], { stdio: "pipe" });
  } catch (e) {
    console.error(`  Failed to download from ${url}`);
    console.error(`  You can install manually: https://github.com/${REPO}/releases`);
    process.exit(1);
  }

  // Verify file was actually downloaded
  const stats = fs.statSync(dest);
  if (stats.size < 10000) {
    console.error(`  Downloaded file is too small (${stats.size} bytes).`);
    console.error(`  Check that release v${version} exists at:`);
    console.error(`  https://github.com/${REPO}/releases`);
    fs.unlinkSync(dest);
    process.exit(1);
  }

  // Verify SHA-256 against checksums.txt published with the release.
  // ARAI_SKIP_CHECKSUM=1 escape hatch matches install.sh; intended only for
  // local dev against unsigned builds.
  if (process.env.ARAI_SKIP_CHECKSUM === "1") {
    console.warn("  \u26a0 Skipping checksum verification (ARAI_SKIP_CHECKSUM=1)");
  } else {
    try {
      execFileSync("curl", ["-sL", "--fail", "-o", checksumsPath, checksumsUrl], {
        stdio: "pipe",
      });
    } catch (e) {
      console.error(`  Failed to fetch checksums.txt from ${checksumsUrl}`);
      console.error("  This release is missing checksums.txt \u2014 refusing to install.");
      console.error("  Set ARAI_SKIP_CHECKSUM=1 to bypass (NOT recommended).");
      fs.unlinkSync(dest);
      process.exit(1);
    }

    const checksums = fs.readFileSync(checksumsPath, "utf8");
    const expected = parseChecksum(checksums, binaryName);
    if (!expected) {
      console.error(`  ${binaryName} not present in checksums.txt`);
      fs.unlinkSync(dest);
      fs.unlinkSync(checksumsPath);
      process.exit(1);
    }

    const crypto = require("crypto");
    const actual = crypto
      .createHash("sha256")
      .update(fs.readFileSync(dest))
      .digest("hex");

    if (actual !== expected) {
      console.error(`  Checksum mismatch for ${binaryName}`);
      console.error(`    expected: ${expected}`);
      console.error(`    actual:   ${actual}`);
      fs.unlinkSync(dest);
      fs.unlinkSync(checksumsPath);
      process.exit(1);
    }

    fs.unlinkSync(checksumsPath);
    console.log(`  \u2713 Checksum verified`);
  }

  // Make executable on Unix
  if (process.platform !== "win32") {
    fs.chmodSync(dest, 0o755);
  }

  console.log(`  \u2713 arai v${version} installed`);
}

// Parse a `sha256sum` output line for the given filename.  Format is
// `<64hex>  <filename>` per GNU coreutils.  Returns the hex digest or null.
function parseChecksum(content, filename) {
  for (const line of content.split(/\r?\n/)) {
    const match = line.match(/^([a-f0-9]{64})\s+(.+)$/i);
    if (match && match[2].trim() === filename) {
      return match[1].toLowerCase();
    }
  }
  return null;
}

function detectPlatform(platform = os.platform(), arch = os.arch()) {
  if (platform === "win32" && arch === "arm64") {
    throw new Error(
      "Windows ARM64 native binaries are not available. Use x64 Node.js under Windows x64 emulation, or build Arai from source with Cargo."
    );
  }

  let osName;
  switch (platform) {
    case "linux":
      osName = "linux";
      break;
    case "darwin":
      osName = "darwin";
      break;
    case "win32":
      osName = "windows";
      break;
    default:
      throw new Error(`Unsupported platform: ${platform}`);
  }

  let archName;
  switch (arch) {
    case "x64":
      archName = "x86_64";
      break;
    case "arm64":
      archName = "aarch64";
      break;
    default:
      throw new Error(`Unsupported architecture: ${arch}`);
  }

  return `${osName}-${archName}`;
}

function getBinaryDownloadName(platform) {
  if (platform.startsWith("windows")) {
    return `arai-${platform}.exe`;
  }
  return `arai-${platform}`;
}

function getLocalBinaryName() {
  return process.platform === "win32" ? "arai-native.exe" : "arai-native";
}

function getVersion() {
  const pkg = JSON.parse(
    fs.readFileSync(path.join(__dirname, "package.json"), "utf8")
  );
  return pkg.version;
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(`  ${error.message}`);
    process.exitCode = 1;
  }
}

module.exports = { detectPlatform, getBinaryDownloadName, parseChecksum };
