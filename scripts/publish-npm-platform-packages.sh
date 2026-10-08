#!/usr/bin/env bash
# Publish the per-platform npm carrier packages for the Sampo CLI. Each
# carries a pre-built binary that the main `sampo` package pulls via
# `optionalDependencies`. Sampo versions their package.json with the shim.
#
# Usage: publish-npm-platform-packages.sh <artifacts-dir>
#   artifacts-dir: directory containing sampo-<rust-target>.tar.gz tarballs
#
# Set NPM_PUBLISH_DRY_RUN=1 to pass --dry-run and never hit the registry.
#
# Idempotent: an existing (name, version) on npm is skipped, so the release
# workflow can be safely re-run.
#
# Requires: npm and node in PATH, tar.

set -euo pipefail

if [ $# -ne 1 ]; then
  echo "usage: $0 <artifacts-dir>" >&2
  exit 2
fi

artifacts_dir="$1"
script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
version="$(node -p "require('$repo_root/packages/sampo/package.json').version")"

mappings=(
  "x86_64-unknown-linux-gnu:sampo-linux-x64"
  "aarch64-unknown-linux-gnu:sampo-linux-arm64"
  "x86_64-apple-darwin:sampo-darwin-x64"
  "aarch64-apple-darwin:sampo-darwin-arm64"
  "x86_64-pc-windows-msvc:sampo-win32-x64"
)

# `bin/sampo.js` refuses to run against a carrier at another version, so check
# them all before publishing any.
for entry in "${mappings[@]}"; do
  IFS=":" read -r _ dir <<<"$entry"
  carrier_version="$(node -p "require('$repo_root/packages/$dir/package.json').version")"
  if [ "$carrier_version" != "$version" ]; then
    echo "error: packages/$dir is at $carrier_version but the shim is at $version" >&2
    exit 1
  fi
done

dry_run_flag=()
if [ "${NPM_PUBLISH_DRY_RUN:-}" = "1" ]; then
  dry_run_flag=("--dry-run")
  echo "[dry-run] NPM_PUBLISH_DRY_RUN=1 — nothing will be published to the registry"
fi

work_root="$(mktemp -d -t sampo-platform-publish.XXXXXX)"
trap 'rm -rf "$work_root"' EXIT

for entry in "${mappings[@]}"; do
  IFS=":" read -r target dir <<<"$entry"
  manifest="$repo_root/packages/$dir/package.json"
  name="$(node -p "require('$manifest').name")"
  binary_path="$(node -p "require('$manifest').bin.sampo")"
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

  pkg_dir="${work_root}/${dir}"
  mkdir -p "${pkg_dir}/bin"
  cp "$manifest" "${pkg_dir}/package.json"

  tar -xzf "$archive" -C "${pkg_dir}/bin" "${binary_path#bin/}"

  if [ ! -s "${pkg_dir}/${binary_path}" ]; then
    echo "extraction did not produce ${pkg_dir}/${binary_path}" >&2
    exit 1
  fi
  chmod +x "${pkg_dir}/${binary_path}"

  echo "publishing ${name}@${version} (${target})"
  # `[@]+` avoids aborting on an empty array under `set -u` (bash < 4.4, e.g. macOS).
  if ! ( cd "$pkg_dir" && npm publish ${dry_run_flag[@]+"${dry_run_flag[@]}"} ); then
    echo "error: failed to publish ${name}@${version} (see npm error above)" >&2
    echo "CI publishes through npm trusted publishing: ${name} must already exist on npm with a trusted publisher (workflow release.yml, environment sampo) that allows npm publish." >&2
    exit 1
  fi
done

echo "platform package publish step complete (version ${version})"
