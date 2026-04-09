#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_path="$repo_root/plugins/openclaw-powd"
source_repo=""
source_ref=""
source_commit=""
dry_run=0
json=0

usage() {
  cat <<'EOF'
usage: scripts/clawhub-release.sh [--source path] [--source-repo owner/name] [--source-ref ref] [--source-commit sha] [--dry-run] [--json]

Publish the OpenClaw powd plugin to ClawHub.

Default behavior:
  - publish ./plugins/openclaw-powd
  - derive the plugin version from plugins/openclaw-powd/package.json
  - derive source repo from git remote origin
  - prefer refs/tags/v<version> when that tag exists
  - otherwise fall back to the current branch ref
  - use the matching commit for the chosen ref

Notes:
  - ClawHub package publish records source repo / source ref / source commit.
  - ClawHub package publish does not accept a changelog flag; changelog is not part of this script.

Examples:
  scripts/clawhub-release.sh --dry-run
  scripts/clawhub-release.sh
  scripts/clawhub-release.sh --source-repo starcoinorg/powd --source-ref refs/tags/v1.0.0
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --source)
      source_path="$2"
      shift 2
      ;;
    --source-repo|--repo)
      source_repo="$2"
      shift 2
      ;;
    --source-ref)
      source_ref="$2"
      shift 2
      ;;
    --source-commit)
      source_commit="$2"
      shift 2
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    --json)
      json=1
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

normalize_repo() {
  local raw="$1"
  raw="${raw%.git}"
  raw="${raw#git+https://github.com/}"
  raw="${raw#https://github.com/}"
  raw="${raw#ssh://git@github.com/}"
  raw="${raw#git@github.com:}"
  printf '%s\n' "$raw"
}

require_cmd clawhub
require_cmd git

plugin_version="$(sed -n 's/^  "version": "\(.*\)",$/\1/p' "$source_path/package.json" | head -n 1)"
if [ -z "$plugin_version" ]; then
  echo "failed to resolve plugin version from $source_path/package.json" >&2
  exit 1
fi

if [ -z "$source_repo" ]; then
  source_repo="$(normalize_repo "$(git -C "$repo_root" remote get-url origin)")"
fi

head_commit="$(git -C "$repo_root" rev-parse HEAD)"
branch_name="$(git -C "$repo_root" rev-parse --abbrev-ref HEAD)"
default_tag="v$plugin_version"
tag_commit="$(git -C "$repo_root" rev-parse -q --verify "refs/tags/$default_tag^{commit}" 2>/dev/null || true)"

if [ -z "$source_ref" ]; then
  if [ -n "$tag_commit" ]; then
    source_ref="refs/tags/$default_tag"
  elif [ "$branch_name" != "HEAD" ]; then
    source_ref="refs/heads/$branch_name"
  fi
fi

if [ -z "$source_commit" ]; then
  if [ -n "$tag_commit" ]; then
    source_commit="$tag_commit"
  else
    source_commit="$head_commit"
  fi
fi

cmd=(clawhub package publish "$source_path" --source-repo "$source_repo" --source-commit "$source_commit")
if [ -n "$source_ref" ]; then
  cmd+=(--source-ref "$source_ref")
fi
if [ "$dry_run" -eq 1 ]; then
  cmd+=(--dry-run)
fi
if [ "$json" -eq 1 ]; then
  cmd+=(--json)
fi

echo "==> publishing OpenClaw plugin to ClawHub"
echo "  source: $source_path"
echo "  repo:   $source_repo"
if [ -n "$source_ref" ]; then
  echo "  ref:    $source_ref"
fi
echo "  commit: $source_commit"
echo "  version: $plugin_version"
echo "==> command"
printf '  %q' "${cmd[@]}"
printf '\n'

"${cmd[@]}"
