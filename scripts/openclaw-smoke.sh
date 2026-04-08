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

cargo build --quiet --bin powd

powd_bin="$repo_root/target/debug/powd"
wallet_address="${POWD_OPENCLAW_WALLET:-0x11111111111111111111111111111111}"
state_path="$POWD_OPENCLAW_ROOT/powd-state.json"
socket_path="$POWD_OPENCLAW_ROOT/powd.sock"
export POWD_STATE_PATH="$state_path"

openclaw --version >/dev/null

"$powd_bin" --socket "$socket_path" --json wallet set --wallet-address "$wallet_address" >/dev/null

doctor_json="$("$powd_bin" --socket "$socket_path" --json doctor)"
printf '%s\n' "$doctor_json" | jq -e --arg wallet "$wallet_address" '
  .wallet_configured == true and
  .wallet_address == $wallet and
  .requested_mode == "auto"
' >/dev/null

server_json="$("$powd_bin" --socket "$socket_path" --json mcp config --server-only)"
printf '%s\n' "$server_json" | jq -e --arg powd "$powd_bin" '
  .command == $powd and
  .args == ["mcp", "serve"] and
  .env == {}
' >/dev/null

openclaw mcp unset powd >/dev/null 2>&1 || true
openclaw mcp set powd "$server_json" >/dev/null

saved_json="$(openclaw mcp show powd --json)"
printf '%s\n' "$saved_json" | jq -e --arg powd "$powd_bin" '
  .command == $powd and
  .args == ["mcp", "serve"] and
  .env == {}
' >/dev/null

list_json="$(openclaw mcp list --json)"
printf '%s\n' "$list_json" | jq -e 'has("powd")' >/dev/null

workspace="$(openclaw-bootstrap)"
materialize_workspace="$POWD_OPENCLAW_ROOT/materialize-workspace"
saved_json_compact="$(printf '%s\n' "$saved_json" | jq -c .)"
materialize_test="$POWD_OPENCLAW_ROOT/powd-materialize.test.ts"
cat >"$materialize_test" <<EOF
import fs from "node:fs/promises";
import { afterAll, expect, it } from "vitest";

let runtime;

afterAll(async () => {
  await runtime?.dispose();
});

it("materializes powd MCP tools with routing metadata intact", async () => {
  const { createBundleMcpToolRuntime } = await import(
    \`\${process.env.OPENCLAW_WORKSPACE_ROOT}/src/agents/pi-bundle-mcp-tools.ts\`,
  );
  const workspaceDir = process.env.OPENCLAW_MATERIALIZE_WORKSPACE;
  const server = JSON.parse(process.env.SERVER_JSON);
  await fs.mkdir(workspaceDir, { recursive: true });

  runtime = await createBundleMcpToolRuntime({
    workspaceDir,
    cfg: {
      mcp: {
        servers: {
          powd: server,
        },
      },
    },
  });

  expect(runtime.tools).toHaveLength(9);

  const toolMap = new Map(runtime.tools.map((tool) => [tool.name, tool]));
  const walletSet = toolMap.get("powd__wallet_set");
  const walletReward = toolMap.get("powd__wallet_reward");
  const minerStop = toolMap.get("powd__miner_stop");
  const minerSetMode = toolMap.get("powd__miner_set_mode");

  expect(walletSet).toBeDefined();
  expect(walletReward).toBeDefined();
  expect(minerStop).toBeDefined();
  expect(minerSetMode).toBeDefined();

  expect(walletSet.label === "wallet_set" || walletSet.label === "Set Wallet").toBe(true);
  expect(walletSet.description).toContain("Prefer wallet_show");
  expect(walletSet.description).toContain("换钱包");
  expect(walletSet.parameters.properties.wallet_address.description).toContain(
    "not the worker name or login string",
  );

  expect(walletReward.description).toContain("earnings");
  expect(walletReward.description).toContain("收益");
  expect(walletReward.description).toContain("Prefer miner_status");

  expect(minerStop.description).toContain("Prefer miner_pause");
  expect(minerStop.description).toContain("turn mining off");

  expect(minerSetMode.description).toContain("Prefer miner_pause or miner_stop");
  expect(minerSetMode.parameters.properties.mode.description).toContain("auto =");
  expect(minerSetMode.parameters.properties.mode.description).toContain("aggressive =");
});
EOF

(
  cd "$workspace"
  SERVER_JSON="$saved_json_compact" \
  OPENCLAW_WORKSPACE_ROOT="$workspace" \
  OPENCLAW_MATERIALIZE_WORKSPACE="$materialize_workspace" \
  node scripts/run-vitest.mjs run --config vitest.unit.config.ts "$materialize_test" >/dev/null
)
rm -f "$materialize_test"

printf 'OpenClaw MCP smoke passed\n'
