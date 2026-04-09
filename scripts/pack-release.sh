#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 3 ] && [ "$#" -ne 4 ]; then
  echo "usage: $0 <powd-binary> <version> [asset-suffix] <output-dir>" >&2
  exit 1
fi

binary="$1"
version="$2"
if [ "$#" -eq 3 ]; then
  asset_suffix="linux-x86_64"
  output_dir="$3"
else
  asset_suffix="$3"
  output_dir="$4"
fi

asset_base="powd-v${version}-${asset_suffix}"
archive_path="$output_dir/${asset_base}.tar.gz"
sha_path="${archive_path}.sha256"

if [ ! -x "$binary" ]; then
  echo "powd binary is missing or not executable: $binary" >&2
  exit 1
fi

mkdir -p "$output_dir"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

cp "$binary" "$tmp_dir/powd"
chmod 755 "$tmp_dir/powd"

tar -C "$tmp_dir" -czf "$archive_path" powd
if command -v sha256sum >/dev/null 2>&1; then
  digest="$(sha256sum "$archive_path" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  digest="$(shasum -a 256 "$archive_path" | awk '{print $1}')"
else
  echo "missing required command: sha256sum or shasum" >&2
  exit 1
fi
printf '%s  %s\n' "$digest" "${asset_base}.tar.gz" >"$sha_path"

printf '%s\n' "$archive_path"
