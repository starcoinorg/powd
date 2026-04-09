#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <output-dir>" >&2
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
package_dir="$repo_root/plugins/openclaw-powd"
output_dir="$1"

if ! command -v npm >/dev/null 2>&1; then
  echo "missing required command: npm" >&2
  exit 1
fi

mkdir -p "$output_dir"
archive_name="$(cd "$package_dir" && npm pack --silent)"
archive_path="$package_dir/$archive_name"
target_path="$output_dir/$archive_name"
sha_path="${target_path}.sha256"

cp "$archive_path" "$target_path"

if command -v sha256sum >/dev/null 2>&1; then
  digest="$(sha256sum "$target_path" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  digest="$(shasum -a 256 "$target_path" | awk '{print $1}')"
else
  echo "missing required command: sha256sum or shasum" >&2
  exit 1
fi

printf '%s  %s\n' "$digest" "$archive_name" >"$sha_path"
printf '%s\n' "$target_path"
