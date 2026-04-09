#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
raw_version="${1:-}"

usage() {
  cat <<'EOF'
usage: scripts/bump.sh <version>

Update the release version in:
  - Cargo.toml
  - plugins/openclaw-powd/package.json

Examples:
  scripts/bump.sh 1.0.0
  scripts/bump.sh v1.0.0
EOF
}

if [ -z "$raw_version" ]; then
  usage >&2
  exit 1
fi

if [ "$raw_version" = "-h" ] || [ "$raw_version" = "--help" ]; then
  usage
  exit 0
fi

version="${raw_version#v}"

if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]]; then
  echo "invalid version: $raw_version" >&2
  echo "expected semver like 1.0.0 or 1.0.0-rc.1" >&2
  exit 1
fi

cargo_toml="$repo_root/Cargo.toml"
plugin_package_json="$repo_root/plugins/openclaw-powd/package.json"

if [ ! -f "$cargo_toml" ]; then
  echo "missing file: $cargo_toml" >&2
  exit 1
fi

if [ ! -f "$plugin_package_json" ]; then
  echo "missing file: $plugin_package_json" >&2
  exit 1
fi

sed -i '0,/^version = ".*"$/s//version = "'"$version"'"/' "$cargo_toml"
sed -i '0,/^  "version": ".*",$/{s//  "version": "'"$version"'",/;}' "$plugin_package_json"

echo "updated version to $version"
echo "  - $cargo_toml"
echo "  - $plugin_package_json"
