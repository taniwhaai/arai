"use strict";

const assert = require("node:assert/strict");
const { spawn, spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { after, before, test } = require("node:test");
const { detectPlatform, getBinaryDownloadName } = require("../install.js");

const packageRoot = path.resolve(__dirname, "..");
const windows = process.platform === "win32";
const nativeName = windows ? "arai-native.exe" : "arai-native";
let fixture;
let consumer;
let fixturePackage;

before(() => {
  fixture = fs.mkdtempSync(path.join(os.tmpdir(), "arai npm launcher "));
  fixturePackage = path.join(fixture, "package with spaces");
  consumer = path.join(fixture, "consumer");
  fs.mkdirSync(path.join(fixturePackage, "bin"), { recursive: true });
  fs.mkdirSync(consumer);
  for (const file of ["package.json", "install.js", "bin/arai"]) {
    fs.copyFileSync(path.join(packageRoot, file), path.join(fixturePackage, file));
  }
  // A real native executable lets the launcher tests control arguments, standard
  // streams, exit status and signals without a downloaded Arai release.
  fs.copyFileSync(process.execPath, path.join(fixturePackage, "bin", nativeName));
  fs.chmodSync(path.join(fixturePackage, "bin", nativeName), 0o755);
  const npmCli = process.env.npm_execpath;
  assert.ok(npmCli, "Run these tests through npm test so its CLI is available");
  const installed = spawnSync(process.execPath, [npmCli, "install", fixturePackage,
    "--prefix", consumer, "--ignore-scripts", "--offline", "--no-audit",
    "--no-fund", "--no-package-lock"], { encoding: "utf8" });
  assert.equal(installed.status, 0, installed.stdout + installed.stderr);
});

after(() => {
  if (fixture) fs.rmSync(fixture, { recursive: true, force: true });
});

function invoke(args, options = {}) {
  const command = path.join(consumer, "node_modules", ".bin", windows ? "arai.cmd" : "arai");
  if (windows) {
    // Keep program text out of cmd.exe: node reads the probe from stdin. The
    // command path exercises npm's actual Windows shim in a path with spaces.
    return spawnSync(process.env.ComSpec || "cmd.exe", ["/d", "/s", "/c",
      `""${command}" ${args.join(" ")}"`], {
      encoding: "utf8", windowsVerbatimArguments: true, ...options,
    });
  }
  return spawnSync(command, args, { encoding: "utf8", ...options });
}

test("npm-created launcher runs its native payload with arguments and stdio", () => {
  const result = invoke(["-", "--probe=ok"], {
    input: 'console.log(JSON.stringify(process.argv.slice(2))); console.error("native stderr");',
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout.trim(), '["--probe=ok"]');
  assert.equal(result.stderr.trim(), "native stderr");
});

test("npm-created launcher preserves a native nonzero exit code", () => {
  const result = invoke(["-"], { input: "process.exit(37);" });
  assert.equal(result.status, 37, result.stderr);
});

test("missing native payload gives actionable installation guidance", () => {
  const empty = path.join(fixture, "missing payload");
  fs.mkdirSync(empty);
  fs.copyFileSync(path.join(packageRoot, "bin", "arai"), path.join(empty, "arai"));
  const result = spawnSync(process.execPath, [path.join(empty, "arai"), "--version"], { encoding: "utf8" });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /native binary not installed/);
  assert.match(result.stderr, /lifecycle scripts enabled/);
  assert.match(result.stderr, /install\.js/);
});

function runInstaller(name, corrupt = false) {
  const directory = path.join(fixture, name);
  fs.mkdirSync(path.join(directory, "bin"), { recursive: true });
  for (const file of ["package.json", "install.js", "bin/arai"]) {
    fs.copyFileSync(path.join(packageRoot, file), path.join(directory, file));
  }
  // Substitute the network boundary only. Run the actual installer including
  // checksum validation, destination selection and executable permissions.
  const preload = path.join(directory, "fake-download.cjs");
  fs.writeFileSync(preload, `
    const fs = require("node:fs");
    const crypto = require("node:crypto");
    const payload = fs.readFileSync(process.execPath);
    const digest = crypto.createHash("sha256").update(payload).digest("hex");
    const asset = ${JSON.stringify(getBinaryDownloadName(detectPlatform()))};
    require("node:child_process").execFileSync = (program, args) => {
      if (program !== "curl") throw new Error("Unexpected download program");
      const destination = args[args.indexOf("-o") + 1];
      const url = args[args.length - 1];
      if (url.endsWith("/checksums.txt")) {
        fs.writeFileSync(destination, digest + "  " + asset + "\\n");
      } else if (url.endsWith("/" + asset)) {
        fs.writeFileSync(destination, payload);
        if (${corrupt}) fs.appendFileSync(destination, "corrupt");
      } else {
        throw new Error("Unexpected asset URL: " + url);
      }
    };
  `);
  const env = { ...process.env };
  delete env.ARAI_SKIP_CHECKSUM;
  const result = spawnSync(process.execPath, ["--require", preload, path.join(directory, "install.js")], {
    encoding: "utf8", env,
  });
  return { directory, result };
}

test("postinstall verifies and installs the payload without replacing the launcher", () => {
  const { directory, result } = runInstaller("install with spaces & 'quotes'");
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Checksum verified/);
  assert.deepEqual(fs.readFileSync(path.join(directory, "bin", "arai")), fs.readFileSync(path.join(packageRoot, "bin", "arai")));
  assert.equal(fs.existsSync(path.join(directory, "bin", "checksums.txt")), false);
  const launched = spawnSync(process.execPath, [path.join(directory, "bin", "arai"), "--version"], { encoding: "utf8" });
  assert.equal(launched.status, 0, launched.stderr);
  assert.equal(launched.stdout.trim(), process.version);
});

test("postinstall rejects corrupt payloads and preserves the launcher", () => {
  const { directory, result } = runInstaller("corrupt download", true);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Checksum mismatch/);
  assert.equal(fs.existsSync(path.join(directory, "bin", nativeName)), false);
  assert.deepEqual(fs.readFileSync(path.join(directory, "bin", "arai")), fs.readFileSync(path.join(packageRoot, "bin", "arai")));
});

test("native termination remains a signal on Unix", { skip: windows }, () => {
  const result = invoke(["-"], { input: 'process.kill(process.pid, "SIGTERM");' });
  assert.equal(result.status, null);
  assert.equal(result.signal, "SIGTERM");
});

test("termination of launcher reaches its native child on Unix", { skip: windows }, async () => {
  const command = path.join(consumer, "node_modules", ".bin", "arai");
  const child = spawn(command, ["-"], { stdio: ["pipe", "pipe", "pipe"] });
  child.stdin.end('process.on("SIGTERM", () => process.exit(42)); console.log("ready"); setInterval(() => {}, 1000);');
  const timer = setTimeout(() => child.kill("SIGKILL"), 10000);
  try {
    const status = await new Promise((resolve, reject) => {
      child.on("error", reject);
      child.stdout.once("data", () => child.kill("SIGTERM"));
      child.on("exit", (code, signal) => resolve({ code, signal }));
    });
    assert.deepEqual(status, { code: 42, signal: null });
  } finally {
    clearTimeout(timer);
  }
});

test("installer maps only platforms with required release assets", () => {
  const platforms = [
    ["linux", "x64", "arai-linux-x86_64"],
    ["linux", "arm64", "arai-linux-aarch64"],
    ["darwin", "x64", "arai-darwin-x86_64"],
    ["darwin", "arm64", "arai-darwin-aarch64"],
    ["win32", "x64", "arai-windows-x86_64.exe"],
  ];
  for (const [osName, arch, asset] of platforms) {
    assert.equal(getBinaryDownloadName(detectPlatform(osName, arch)), asset);
  }
  assert.throws(() => detectPlatform("win32", "arm64"), /Windows ARM64 native binaries are not available/);
  assert.throws(() => detectPlatform("linux", "ia32"), /Unsupported architecture/);
  assert.throws(() => detectPlatform("freebsd", "x64"), /Unsupported platform/);
});
