#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

export POWD_REPO_ROOT="${POWD_REPO_ROOT:-$repo_root}"
export POWD_OPENCLAW_ROOT="${POWD_OPENCLAW_ROOT:-$repo_root/.tmp/openclaw}"
export OPENCLAW_HOME="${OPENCLAW_HOME:-$POWD_OPENCLAW_ROOT/home}"
export XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$POWD_OPENCLAW_ROOT/xdg-config}"
export XDG_STATE_HOME="${XDG_STATE_HOME:-$POWD_OPENCLAW_ROOT/xdg-state}"
mkdir -p "$POWD_OPENCLAW_ROOT" "$XDG_CONFIG_HOME" "$XDG_STATE_HOME"

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required; run this script inside 'nix develop .#openclaw'" >&2
  exit 1
fi

if ! command -v openclaw >/dev/null 2>&1; then
  echo "openclaw is required; run this script inside 'nix develop .#openclaw'" >&2
  exit 1
fi

cargo build --quiet --bin powctl --bin powd

powctl_bin="$repo_root/target/debug/powctl"
wallet_address="${POWD_OPENCLAW_WALLET:-0x11111111111111111111111111111111}"
state_path="$POWD_OPENCLAW_ROOT/powd-state.json"
socket_path="$POWD_OPENCLAW_ROOT/powd.sock"
export POWD_STATE_PATH="$state_path"

openclaw --version >/dev/null

"$powctl_bin" --socket "$socket_path" --json wallet set --wallet-address "$wallet_address" >/dev/null

doctor_json="$("$powctl_bin" --socket "$socket_path" --json doctor)"
printf '%s\n' "$doctor_json" | jq -e --arg wallet "$wallet_address" '
  .wallet_configured == true and
  .wallet_address == $wallet and
  .requested_mode == "auto"
' >/dev/null

server_json="$("$powctl_bin" --socket "$socket_path" --json mcp config --server-only)"
printf '%s\n' "$server_json" | jq -e --arg powctl "$powctl_bin" '
  .command == $powctl and
  .args == ["mcp", "serve"] and
  .env == {}
' >/dev/null

openclaw mcp unset powd >/dev/null 2>&1 || true
openclaw mcp set powd "$server_json" >/dev/null

saved_json="$(openclaw mcp show powd --json)"
printf '%s\n' "$saved_json" | jq -e --arg powctl "$powctl_bin" '
  .command == $powctl and
  .args == ["mcp", "serve"] and
  .env == {}
' >/dev/null

list_json="$(openclaw mcp list --json)"
printf '%s\n' "$list_json" | jq -e 'has("powd")' >/dev/null

printf 'OpenClaw MCP smoke passed\n'
