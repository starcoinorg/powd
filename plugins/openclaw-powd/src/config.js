import path from "node:path";
import { MCP_SERVER_NAME, POWD_PLUGIN_CLAWHUB_SPEC } from "./constants.js";

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function normalizeCommand(command) {
  return typeof command === "string" && command.trim() ? path.resolve(command) : null;
}

export function buildPowdServer(binaryPath) {
  return {
    command: binaryPath,
    args: ["mcp", "serve"],
    env: {},
  };
}

export function getPowdServer(config) {
  if (!isRecord(config?.mcp) || !isRecord(config.mcp.servers)) {
    return null;
  }
  const candidate = config.mcp.servers[MCP_SERVER_NAME];
  return isRecord(candidate) ? candidate : null;
}

export function isManagedPowdServer(server, binaryPath) {
  if (!server) {
    return false;
  }

  const command = normalizeCommand(server.command);
  const expected = normalizeCommand(binaryPath);
  if (!command || !expected || command !== expected) {
    return false;
  }

  return Array.isArray(server.args) && server.args.length === 2 && server.args[0] === "mcp" && server.args[1] === "serve";
}

export function upsertPowdServer(config, binaryPath) {
  return {
    ...config,
    mcp: {
      ...config?.mcp,
      servers: {
        ...(config?.mcp?.servers ?? {}),
        [MCP_SERVER_NAME]: buildPowdServer(binaryPath),
      },
    },
  };
}

export function upsertPowdPluginAllow(config) {
  const currentAllow = Array.isArray(config?.plugins?.allow) ? config.plugins.allow : [];
  const allow = currentAllow.includes(MCP_SERVER_NAME) ? currentAllow : [...currentAllow, MCP_SERVER_NAME];

  return {
    ...config,
    plugins: {
      ...config?.plugins,
      allow,
    },
  };
}

export function normalizePowdPluginInstallSpec(config) {
  const install = config?.plugins?.installs?.[MCP_SERVER_NAME];
  if (!isRecord(install) || install.source !== "clawhub") {
    return config;
  }

  const spec = typeof install.spec === "string" ? install.spec.trim() : "";
  if (!spec || spec === POWD_PLUGIN_CLAWHUB_SPEC || !spec.startsWith(`${POWD_PLUGIN_CLAWHUB_SPEC}@`)) {
    return config;
  }

  return {
    ...config,
    plugins: {
      ...config?.plugins,
      installs: {
        ...(config?.plugins?.installs ?? {}),
        [MCP_SERVER_NAME]: {
          ...install,
          spec: POWD_PLUGIN_CLAWHUB_SPEC,
        },
      },
    },
  };
}
