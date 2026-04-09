#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo="starcoinorg/powd"
remote="origin"
output_dir=""
tag=""
title=""
notes=""
notes_file=""
generate_notes=0
dry_run=0
prerelease=0

usage() {
  cat <<'EOF'
usage: scripts/release.sh [--repo owner/name] [--remote origin] [--tag vX.Y.Z] [--title "title"] [--notes "text"] [--notes-file path] [--generate-notes] [--output-dir path] [--dry-run]

Build the local powd release assets, create or update the GitHub Release, and upload the assets
that can be produced from the current host.

On Linux today this uploads:
  - powd-v<version>-linux-x86_64.tar.gz
  - powd-v<version>-linux-x86_64.tar.gz.sha256

The tag push also triggers GitHub Actions to build and upload additional cross-platform assets,
including:
  - powd-v<version>-darwin-arm64.tar.gz
  - powd-v<version>-darwin-arm64.tar.gz.sha256
  - powd-v<version>-windows-x86_64.tar.gz
  - powd-v<version>-windows-x86_64.tar.gz.sha256

Default behavior:
  - read the version from Cargo.toml
  - ensure tag v<version> exists on the current commit and push it to origin
  - build target/release/powd
  - generate:
      powd-v<version>-linux-x86_64.tar.gz
      powd-v<version>-linux-x86_64.tar.gz.sha256
  - create the GitHub Release if it does not already exist
  - generate GitHub-style release title and notes by default
  - mark versions with a prerelease suffix such as -rc.1 or -beta.1 as GitHub prereleases
  - let --notes / --notes-file override the default notes body
  - upload all assets with --clobber

Examples:
  scripts/release.sh --dry-run
  scripts/release.sh --tag v0.1.0
  scripts/release.sh --notes-file release-notes/v0.1.0.md
  scripts/release.sh --generate-notes
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --repo)
      repo="$2"
      shift 2
      ;;
    --remote)
      remote="$2"
      shift 2
      ;;
    --tag)
      tag="$2"
      shift 2
      ;;
    --title)
      title="$2"
      shift 2
      ;;
    --notes)
      notes="$2"
      shift 2
      ;;
    --notes-file)
      notes_file="$2"
      shift 2
      ;;
    --generate-notes)
      generate_notes=1
      shift
      ;;
    --output-dir)
      output_dir="$2"
      shift 2
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

require_cmd cargo
require_cmd gh
require_cmd git
require_cmd tar

if ! git -C "$repo_root" diff --quiet || ! git -C "$repo_root" diff --cached --quiet; then
  echo "working tree is dirty; commit or stash changes before running scripts/release.sh" >&2
  exit 1
fi

cargo_version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo_root/Cargo.toml" | head -n 1)"

if [ -z "$cargo_version" ]; then
  echo "failed to resolve Cargo version from $repo_root/Cargo.toml" >&2
  exit 1
fi

if [ -n "$notes" ] && [ -n "$notes_file" ]; then
  echo "use either --notes or --notes-file, not both" >&2
  exit 1
fi

if [ -n "$notes_file" ] && [ ! -f "$notes_file" ]; then
  echo "release notes file not found: $notes_file" >&2
  exit 1
fi

if [ -z "$notes" ] && [ -z "$notes_file" ]; then
  generate_notes=1
fi

if [ "$generate_notes" -eq 1 ]; then
  require_cmd jq
fi

version="$cargo_version"
if [ -z "$tag" ]; then
  tag="v$version"
fi

if [[ "$version" == *-* ]]; then
  prerelease=1
fi

if [ -z "$output_dir" ]; then
  output_dir="$repo_root/.tmp/release-assets/$tag"
fi

mkdir -p "$output_dir"

notes_path=""
cleanup_notes_path=0
if [ "$generate_notes" -eq 1 ]; then
  if [ -n "$notes_file" ]; then
    notes_path="$notes_file"
  elif [ -n "$notes" ]; then
    notes_path="$(mktemp)"
    cleanup_notes_path=1
    printf '%s\n' "$notes" >"$notes_path"
  fi
elif [ -n "$notes_file" ]; then
  notes_path="$notes_file"
elif [ -n "$notes" ]; then
  notes_path="$(mktemp)"
  cleanup_notes_path=1
  printf '%s\n' "$notes" >"$notes_path"
fi

cleanup() {
  if [ "$cleanup_notes_path" -eq 1 ] && [ -n "$notes_path" ]; then
    rm -f "$notes_path"
  fi
}
trap cleanup EXIT

powd_binary="$repo_root/target/release/powd"
head_commit="$(git -C "$repo_root" rev-parse HEAD)"
local_tag_commit="$(git -C "$repo_root" rev-parse -q --verify "refs/tags/$tag^{commit}" 2>/dev/null || true)"
remote_tag_commit=""
if [ "$dry_run" -eq 0 ]; then
  remote_tag_commit="$(git -C "$repo_root" ls-remote --tags "$remote" "refs/tags/$tag^{}" | awk '{print $1}' | head -n 1)"
  if [ -z "$remote_tag_commit" ]; then
    remote_tag_commit="$(git -C "$repo_root" ls-remote --tags "$remote" "refs/tags/$tag" | awk '{print $1}' | head -n 1)"
  fi
fi

if [ -n "$local_tag_commit" ] && [ "$local_tag_commit" != "$head_commit" ] && [ "$dry_run" -eq 0 ]; then
  echo "local tag $tag exists but does not point to HEAD ($local_tag_commit != $head_commit)" >&2
  exit 1
fi

if [ -n "$remote_tag_commit" ] && [ "$remote_tag_commit" != "$head_commit" ]; then
  echo "remote tag $tag exists on $remote but does not point to HEAD ($remote_tag_commit != $head_commit)" >&2
  exit 1
fi

echo "==> building powd $version"
cargo build --release --bin powd --manifest-path "$repo_root/Cargo.toml"

echo "==> packaging powd release archive"
powd_archive_path="$("$repo_root/scripts/pack-release.sh" "$powd_binary" "$version" "$output_dir")"
powd_sha_path="${powd_archive_path}.sha256"

assets=(
  "$powd_archive_path"
  "$powd_sha_path"
)

echo "==> assets ready"
for asset in "${assets[@]}"; do
  echo "  - $asset"
done

if [ "$dry_run" -eq 1 ] && [ -n "$local_tag_commit" ] && [ "$local_tag_commit" != "$head_commit" ]; then
  echo "==> dry-run: local tag $tag exists on a different commit; skipping local tag mutation checks"
elif [ -z "$local_tag_commit" ]; then
  echo "==> creating local tag $tag on $head_commit"
  if [ "$dry_run" -eq 0 ]; then
    git -C "$repo_root" tag -a "$tag" -m "$tag"
  fi
else
  echo "==> local tag $tag already points to HEAD"
fi

if [ -z "$remote_tag_commit" ]; then
  echo "==> pushing tag $tag to $remote"
  if [ "$dry_run" -eq 0 ]; then
    git -C "$repo_root" push "$remote" "refs/tags/$tag"
  fi
else
  echo "==> remote tag $tag already exists on $remote"
fi

if [ "$dry_run" -eq 1 ]; then
  echo "dry-run: not contacting GitHub"
  exit 0
fi

echo "==> checking GitHub auth"
gh auth status -h github.com >/dev/null

release_exists=0
if gh release view "$tag" --repo "$repo" >/dev/null 2>&1; then
  release_exists=1
fi

release_title="${title:-$tag}"
release_notes_path="$notes_path"
cleanup_release_notes_path=0
generate_notes_json=""

if [ "$generate_notes" -eq 1 ]; then
  echo "==> generating release notes via GitHub Release Notes API"
  generate_notes_json="$(gh api -X POST "repos/$repo/releases/generate-notes" -f tag_name="$tag" -f target_commitish="$head_commit")"
  generated_title="$(printf '%s\n' "$generate_notes_json" | jq -r '.name // empty')"
  generated_body="$(printf '%s\n' "$generate_notes_json" | jq -r '.body // empty')"
  release_title="${title:-${generated_title:-$tag}}"
  release_notes_path="$(mktemp)"
  cleanup_release_notes_path=1
  if [ -n "$notes_path" ]; then
    cat "$notes_path" >"$release_notes_path"
    if [ -n "$generated_body" ]; then
      printf '\n\n%s\n' "$generated_body" >>"$release_notes_path"
    fi
  else
    printf '%s\n' "$generated_body" >"$release_notes_path"
  fi
fi

cleanup_release_notes() {
  if [ "$cleanup_release_notes_path" -eq 1 ] && [ -n "$release_notes_path" ]; then
    rm -f "$release_notes_path"
  fi
}
trap 'cleanup; cleanup_release_notes' EXIT

if [ "$release_exists" -eq 0 ]; then
  echo "==> creating release $tag in $repo"
  create_args=()
  if [ "$prerelease" -eq 1 ]; then
    create_args+=(--prerelease)
  fi
  gh release create "$tag" \
    --repo "$repo" \
    --verify-tag \
    --title "$release_title" \
    --notes-file "$release_notes_path" \
    "${create_args[@]}"
else
  echo "==> updating release notes for $tag in $repo"
  edit_args=()
  if [ "$prerelease" -eq 1 ]; then
    edit_args+=(--prerelease)
  fi
  gh release edit "$tag" \
    --repo "$repo" \
    --title "$release_title" \
    --notes-file "$release_notes_path" \
    "${edit_args[@]}"
fi

echo "==> uploading assets to $repo@$tag"
gh release upload "$tag" --repo "$repo" --clobber "${assets[@]}"

echo "==> release URL"
gh release view "$tag" --repo "$repo" --json url --jq .url
