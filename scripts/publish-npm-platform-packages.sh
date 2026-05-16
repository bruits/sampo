#!/usr/bin/env bash
# Generate and publish the per-platform npm carrier packages for Sampo.
#
# These packages exist only to ship a pre-built CLI binary on each supported
# platform via the main `sampo` package's `optionalDependencies`. They have no
# source of truth in the repo: each release publishes a freshly templated
# package.json paired with the binary built by the release workflow.
#
# Usage: publish-npm-platform-packages.sh <artifacts-dir> [<version>]
#   artifacts-dir: directory containing sampo-<rust-target>.tar.gz tarballs
#   version:       shim version to publish at; defaults to the value in
#                  packages/sampo/package.json
#
# Environment:
#   NPM_PUBLISH_DRY_RUN=1  -> pass --dry-run to `npm publish`; never hit the
#                             registry. Use this for local smoke tests.
#
# Idempotent: a (name, version) tuple already on npm is skipped with a log
# line. This lets the release workflow re-run safely.
#
# Requires: npm and node in PATH, tar.

set -euo pipefail

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
  echo "usage: $0 <artifacts-dir> [<version>]" >&2
  exit 2
fi

artifacts_dir="$1"
script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
shim_manifest="$repo_root/packages/sampo/package.json"

if [ "$#" -eq 2 ]; then
  version="$2"
else
  version="$(node -p "require('$shim_manifest').version")"
fi

if [ -z "${version}" ] || [ "${version}" = "null" ] || [ "${version}" = "undefined" ]; then
  echo "error: could not determine shim version (got '${version}')" >&2
  exit 1
fi

# rust-target : npm-package-name : os : cpu : binary
mappings=(
  "x86_64-unknown-linux-gnu:sampo-linux-x64:linux:x64:sampo"
  "aarch64-unknown-linux-gnu:sampo-linux-arm64:linux:arm64:sampo"
  "x86_64-apple-darwin:sampo-darwin-x64:darwin:x64:sampo"
  "aarch64-apple-darwin:sampo-darwin-arm64:darwin:arm64:sampo"
  "x86_64-pc-windows-msvc:sampo-win32-x64:win32:x64:sampo.exe"
)

dry_run_flag=()
if [ "${NPM_PUBLISH_DRY_RUN:-}" = "1" ]; then
  dry_run_flag=("--dry-run")
  echo "[dry-run] NPM_PUBLISH_DRY_RUN=1 — nothing will be published to the registry"
fi

work_root="$(mktemp -d -t sampo-platform-publish.XXXXXX)"
trap 'rm -rf "$work_root"' EXIT

for entry in "${mappings[@]}"; do
  IFS=":" read -r target name os cpu binary <<<"$entry"
  archive="${artifacts_dir}/sampo-${target}.tar.gz"

  if [ ! -f "$archive" ]; then
    echo "missing artifact: $archive" >&2
    exit 1
  fi

  # Skip if this exact (name, version) is already on npm, so re-running the
  # release workflow after a partial failure does not error on the survivors.
  # Gate on stdout (not exit code) because npm <9 exits 0 even when the
  # queried version is missing.
  if [ -n "$(npm view "${name}@${version}" version 2>/dev/null || true)" ]; then
    echo "skip ${name}@${version}: already on npm"
    continue
  fi

  pkg_dir="${work_root}/${name}"
  mkdir -p "${pkg_dir}/bin"

  tar -xzf "$archive" -C "${pkg_dir}/bin" "$binary"

  if [ ! -s "${pkg_dir}/bin/${binary}" ]; then
    echo "extraction did not produce ${pkg_dir}/bin/${binary}" >&2
    exit 1
  fi

  if [ "$binary" = "sampo.exe" ]; then
    # `test -x` is unreliable for .exe files on Unix where this script runs.
    prepub_test="test -f bin/sampo.exe"
  else
    chmod +x "${pkg_dir}/bin/${binary}"
    prepub_test="test -x bin/sampo"
  fi

  cat > "${pkg_dir}/package.json" <<EOF
{
  "name": "${name}",
  "version": "${version}",
  "description": "Sampo CLI binary for ${os}-${cpu}",
  "repository": {
    "type": "git",
    "url": "git+https://github.com/bruits/sampo.git"
  },
  "license": "MIT",
  "bin": {
    "sampo": "bin/${binary}"
  },
  "files": [
    "bin/${binary}"
  ],
  "scripts": {
    "prepublishOnly": "${prepub_test}"
  },
  "os": [
    "${os}"
  ],
  "cpu": [
    "${cpu}"
  ],
  "publishConfig": {
    "access": "public"
  }
}
EOF

  echo "publishing ${name}@${version} (${target})"
  ( cd "$pkg_dir" && npm publish "${dry_run_flag[@]}" )
done

echo "platform package publish step complete (version ${version})"
