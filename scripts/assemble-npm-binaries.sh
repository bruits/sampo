#!/usr/bin/env bash
# Extract Sampo platform binaries from CI artifacts and place them into each
# packages/sampo-<platform>/bin/ directory so `sampo publish` (or `npm publish`)
# ships them inside the corresponding npm tarball.
#
# Usage: assemble-npm-binaries.sh <artifacts-dir> <packages-dir>
#   artifacts-dir: directory containing sampo-<rust-target>.tar.gz tarballs
#   packages-dir:  directory containing the npm package skeletons

set -euo pipefail

if [ $# -ne 2 ]; then
  echo "usage: $0 <artifacts-dir> <packages-dir>" >&2
  exit 2
fi

artifacts_dir="$1"
packages_dir="$2"

# rust-target  →  npm-platform-dir  →  binary-name
mappings=(
  "x86_64-unknown-linux-gnu:sampo-linux-x64:sampo"
  "aarch64-unknown-linux-gnu:sampo-linux-arm64:sampo"
  "x86_64-apple-darwin:sampo-darwin-x64:sampo"
  "aarch64-apple-darwin:sampo-darwin-arm64:sampo"
  "x86_64-pc-windows-msvc:sampo-win32-x64:sampo.exe"
)

for entry in "${mappings[@]}"; do
  IFS=":" read -r target platform_dir binary <<<"$entry"
  archive="${artifacts_dir}/sampo-${target}.tar.gz"
  dest_dir="${packages_dir}/${platform_dir}/bin"

  if [ ! -f "$archive" ]; then
    echo "missing artifact: $archive" >&2
    exit 1
  fi

  mkdir -p "$dest_dir"
  tar -xzf "$archive" -C "$dest_dir" "$binary"

  if [ ! -s "${dest_dir}/${binary}" ]; then
    echo "extraction did not produce ${dest_dir}/${binary}" >&2
    exit 1
  fi

  if [ "$binary" != "sampo.exe" ]; then
    chmod +x "${dest_dir}/${binary}"
  fi

  echo "placed ${binary} -> ${dest_dir}/"
done
