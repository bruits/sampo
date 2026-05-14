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
const ourVersion = require("../package.json").version;
let binary;
try {
  const pkgJsonPath = createRequire(__filename).resolve(`${pkg}/package.json`);
  const platformVersion = require(pkgJsonPath).version;
  if (platformVersion !== ourVersion) {
    // Guards against lockfile drift leaving an older binary alongside a newer shim.
    console.error(
      `sampo: version mismatch — shim is ${ourVersion} but ${pkg} is ${platformVersion}. ` +
        `Reinstall sampo to realign the platform binary.`,
    );
    process.exit(1);
  }
  binary = path.join(path.dirname(pkgJsonPath), "bin", exe);
} catch (err) {
  const detail = err && (err.code || err.name) ? ` (${err.code || err.name})` : "";
  const message = err && err.message ? `: ${err.message}` : "";
  console.error(
    `sampo: failed to load ${pkg}${detail}${message}. ` +
      `If the optional dependency was skipped at install time, check your install logs ` +
      `(npm/pnpm/yarn may have skipped it due to --no-optional or a platform mismatch); ` +
      `otherwise inspect node_modules/${pkg}/ for a corrupt install.`,
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
