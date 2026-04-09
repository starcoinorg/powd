#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

export POWD_REPO_ROOT="${POWD_REPO_ROOT:-$repo_root}"
export POWD_OPENCLAW_ROOT="${POWD_OPENCLAW_ROOT:-$repo_root/.tmp/openclaw-plugin}"
export OPENCLAW_HOME="${OPENCLAW_HOME:-$POWD_OPENCLAW_ROOT/home}"
export XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$POWD_OPENCLAW_ROOT/xdg-config}"
export XDG_STATE_HOME="${XDG_STATE_HOME:-$POWD_OPENCLAW_ROOT/xdg-state}"

rm -rf "$POWD_OPENCLAW_ROOT"
mkdir -p "$POWD_OPENCLAW_ROOT" "$OPENCLAW_HOME" "$XDG_CONFIG_HOME" "$XDG_STATE_HOME"

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required; run this script inside 'nix develop .#openclaw'" >&2
  exit 1
fi

if ! command -v openclaw >/dev/null 2>&1; then
  echo "openclaw is required; run this script inside 'nix develop .#openclaw'" >&2
  exit 1
fi

if ! command -v npm >/dev/null 2>&1; then
  echo "npm is required; run this script inside 'nix develop .#openclaw'" >&2
  exit 1
fi

capture_json() {
  local output
  output="$("$@" 2>&1)"
  printf '%s\n' "$output" | awk 'BEGIN { emit = 0 } /^[[:space:]]*{/ { emit = 1 } emit { print }'
}

cargo build --quiet --bin powd

powd_bin="$repo_root/target/debug/powd"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo_root/Cargo.toml" | head -n 1)"
if [ -z "$version" ]; then
  echo "failed to resolve powd version from Cargo.toml" >&2
  exit 1
fi
case "$(uname -s):$(uname -m)" in
  Linux:x86_64)
    asset_suffix="linux-x86_64"
    ;;
  Darwin:arm64|Darwin:aarch64)
    asset_suffix="darwin-arm64"
    ;;
  *)
    echo "powd plugin smoke requires a supported host platform" >&2
    exit 1
    ;;
esac
release_root="$POWD_OPENCLAW_ROOT/release-fixture/releases/download/v$version"
"$repo_root/scripts/pack-release.sh" "$powd_bin" "$version" "$asset_suffix" "$release_root" >/dev/null
latest_api_path="$POWD_OPENCLAW_ROOT/release-fixture/api/releases/latest"
mkdir -p "$(dirname "$latest_api_path")"
printf '{"tag_name":"v%s"}\n' "$version" >"$latest_api_path"

server_port=39123
node "$repo_root/scripts/httpd.mjs" "$POWD_OPENCLAW_ROOT/release-fixture" "$server_port" >/tmp/powd-plugin-smoke-http.log 2>&1 &
server_pid=$!
trap 'kill "$server_pid" >/dev/null 2>&1 || true' EXIT
sleep 1

plugin_tgz="$(cd "$repo_root/plugins/openclaw-powd" && npm pack --silent)"
plugin_path="$repo_root/plugins/openclaw-powd/$plugin_tgz"

openclaw plugins install "$plugin_path" >/dev/null
inspect_json="$(capture_json openclaw plugins inspect powd --json)"
printf '%s\n' "$inspect_json" | jq -e '.plugin.id == "powd"' >/dev/null

status_before="$(capture_json openclaw powd status --json)"
printf '%s\n' "$status_before" | jq -e '.installed == false and .registered == false' >/dev/null

openclaw config set plugins.entries.powd.config.releaseBaseUrl "\"http://127.0.0.1:${server_port}/releases/download\"" >/dev/null
openclaw config set plugins.entries.powd.config.releaseApiBaseUrl "\"http://127.0.0.1:${server_port}/api/releases\"" >/dev/null
install_json="$(capture_json openclaw powd install --json)"
printf '%s\n' "$install_json" | jq -e --arg version "$version" '
  .installed == true and
  .registered == true and
  .version == $version and
  .mcpCommandMatchesInstall == true
' >/dev/null

saved_json="$(capture_json openclaw mcp show powd --json)"
binary_path="$(printf '%s\n' "$install_json" | jq -r '.binaryPath')"
printf '%s\n' "$saved_json" | jq -e --arg powd "$binary_path" '
  .command == $powd and
  .args == ["mcp", "serve"] and
  .env == {}
' >/dev/null

workspace="$(openclaw-bootstrap)"
materialize_workspace="$POWD_OPENCLAW_ROOT/materialize-workspace"
saved_json_compact="$(printf '%s\n' "$saved_json" | jq -c .)"
materialize_test="$POWD_OPENCLAW_ROOT/powd-plugin-materialize.test.ts"
cat >"$materialize_test" <<EOF
import fs from "node:fs/promises";
import { afterAll, expect, it } from "vitest";

let runtime;

afterAll(async () => {
  await runtime?.dispose();
});

it("materializes powd MCP tools from the plugin-managed registration", async () => {
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
  const names = runtime.tools.map((tool) => tool.name).toSorted();
  expect(names).toContain("powd__wallet_set");
  expect(names).toContain("powd__wallet_show");
  expect(names).toContain("powd__wallet_reward");
  expect(names).toContain("powd__miner_status");
  expect(names).toContain("powd__miner_set_mode");
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

printf 'OpenClaw plugin smoke passed\n'
