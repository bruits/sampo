#!/usr/bin/env node
const { spawnSync } = require("node:child_process");
const { createRequire } = require("node:module");
const path = require("node:path");

const PLATFORM_PACKAGES = {
  "linux x64": "sampo-linux-x64",
  "linux arm64": "sampo-linux-arm64",
  "darwin x64": "sampo-darwin-x64",
  "darwin arm64": "sampo-darwin-arm64",
  "win32 x64": "sampo-win32-x64",
};

const key = `${process.platform} ${process.arch}`;
const pkg = PLATFORM_PACKAGES[key];
if (!pkg) {
  console.error(
    `sampo: unsupported platform ${key}. Supported: ${Object.keys(PLATFORM_PACKAGES).join(", ")}.`,
  );
  process.exit(1);
}

// Resolve via package.json then join: pnpm's strict layouts don't expose bin/ files directly.
const exe = process.platform === "win32" ? "sampo.exe" : "sampo";
let binary;
try {
  const pkgJson = createRequire(__filename).resolve(`${pkg}/package.json`);
  binary = path.join(path.dirname(pkgJson), "bin", exe);
} catch {
  console.error(
    `sampo: failed to resolve ${pkg}. The optional dependency was not installed — ` +
      `check your install logs (npm/pnpm/yarn may have skipped it due to --no-optional or a platform mismatch).`,
  );
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`sampo: failed to execute binary: ${result.error.message}`);
  process.exit(1);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
} else {
  process.exit(result.status ?? 1);
}
