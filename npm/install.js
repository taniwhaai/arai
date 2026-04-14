#!/usr/bin/env node
"use strict";

const { execSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const os = require("os");
const https = require("https");

const REPO = "taniwhaai/arai";

function main() {
  const platform = detectPlatform();
  const version = getVersion();
  const binaryName = `arai-${platform}`;
  const url = `https://github.com/${REPO}/releases/download/v${version}/${binaryName}`;
  const dest = path.join(__dirname, "bin", getPlatformBinaryName());

  console.log(`  Downloading arai ${version} for ${platform}...`);

  fs.mkdirSync(path.dirname(dest), { recursive: true });

  // Use curl for download (available on all platforms with Node)
  try {
    execSync(`curl -sL -o "${dest}" "${url}"`, { stdio: "pipe" });
  } catch (e) {
    console.error(`  Failed to download from ${url}`);
    console.error(
      "  You can install manually: https://github.com/taniwhaai/arai/releases"
    );
    process.exit(1);
  }

  // Verify file size
  const stats = fs.statSync(dest);
  if (stats.size < 10000) {
    console.error(`  Downloaded file is too small (${stats.size} bytes).`);
    console.error(`  Check that release v${version} exists at:`);
    console.error(`  https://github.com/${REPO}/releases`);
    fs.unlinkSync(dest);
    process.exit(1);
  }

  // Make executable on Unix
  if (process.platform !== "win32") {
    fs.chmodSync(dest, 0o755);
  }

  console.log(`  ✓ arai installed`);
}

function detectPlatform() {
  const platform = os.platform();
  const arch = os.arch();

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
      console.error(`Unsupported platform: ${platform}`);
      process.exit(1);
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
      console.error(`Unsupported architecture: ${arch}`);
      process.exit(1);
  }

  return `${osName}-${archName}`;
}

function getPlatformBinaryName() {
  return process.platform === "win32" ? "arai.exe" : "arai";
}

function getVersion() {
  // Read version from package.json
  const pkg = JSON.parse(
    fs.readFileSync(path.join(__dirname, "package.json"), "utf8")
  );
  return pkg.version;
}

main();
