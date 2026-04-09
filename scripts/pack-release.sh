#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: $0 <powd-binary> <version> <output-dir>" >&2
  exit 1
fi

binary="$1"
version="$2"
output_dir="$3"
asset_base="powd-v${version}-linux-x86_64"
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
sha256sum "$archive_path" | awk '{print $1 "  " "'"${asset_base}.tar.gz"'"}' >"$sha_path"

printf '%s\n' "$archive_path"
