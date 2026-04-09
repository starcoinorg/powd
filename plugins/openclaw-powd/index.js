import { definePluginEntry } from "openclaw/plugin-sdk/plugin-entry";
import { registerPowdCli } from "./src/cli.js";
import {
  buildApprovalRequest,
  buildInstallCommandReply,
  buildInstallToolResult,
  buildStatusCommandReply,
  buildStatusToolResult,
} from "./src/format.js";
import { installPowd } from "./src/install.js";
import { normalizePowdPluginInstallSpec } from "./src/config.js";
import { collectSetupStatus, toPublicSetupStatus } from "./src/status.js";

async function loadConfig(configApi) {
  return await Promise.resolve(configApi.loadConfig());
}

function normalizeRequestedVersion(version) {
  return typeof version === "string" && version.trim() ? version.trim() : undefined;
}

function resolvePluginConfig(config) {
  const pluginConfig = config?.plugins?.entries?.powd?.config;
  return pluginConfig && typeof pluginConfig === "object" && !Array.isArray(pluginConfig) ? pluginConfig : {};
}

function extractToolVersion(params) {
  return normalizeRequestedVersion(params?.version);
}

function extractToolReplace(params) {
  return params?.replace === true;
}

function extractEventVersion(event) {
  return (
    normalizeRequestedVersion(event?.params?.version) ??
    normalizeRequestedVersion(event?.arguments?.version) ??
    normalizeRequestedVersion(event?.args?.version)
  );
}

function resolveReleaseOverrides(pluginConfig) {
  const config =
    pluginConfig && typeof pluginConfig === "object" && !Array.isArray(pluginConfig) ? pluginConfig : {};
  const releaseBaseUrl =
    typeof config.releaseBaseUrl === "string" && config.releaseBaseUrl.trim() ? config.releaseBaseUrl.trim() : undefined;
  const releaseApiBaseUrl =
    typeof config.releaseApiBaseUrl === "string" && config.releaseApiBaseUrl.trim()
      ? config.releaseApiBaseUrl.trim()
      : undefined;
  return {
    releaseBaseUrl,
    releaseApiBaseUrl,
  };
}

async function resolveInstallReleaseOverrides(api, configOverride) {
  if (configOverride) {
    const overrides = resolveReleaseOverrides(resolvePluginConfig(configOverride));
    if (overrides.releaseBaseUrl || overrides.releaseApiBaseUrl) {
      return overrides;
    }
  }

  const pluginOverrides = resolveReleaseOverrides(api.pluginConfig);
  if (pluginOverrides.releaseBaseUrl || pluginOverrides.releaseApiBaseUrl) {
    return pluginOverrides;
  }

  const config = await loadConfig(api.runtime.config);
  return resolveReleaseOverrides(resolvePluginConfig(config));
}

async function normalizeOwnInstallSpec(api) {
  const currentConfig = await loadConfig(api.runtime.config);
  const normalizedConfig = normalizePowdPluginInstallSpec(currentConfig);
  if (normalizedConfig === currentConfig) {
    return;
  }

  await api.runtime.config.writeConfigFile(normalizedConfig);
  if (typeof api.logger?.info === "function") {
    api.logger.info("powd-plugin: normalized ClawHub install spec so future plugin updates follow the latest release");
  }
}

function parseCommandInstallArgs(parts) {
  let version;
  let replace = false;
  for (const part of parts) {
    if (part === "--replace") {
      replace = true;
      continue;
    }
    if (!version) {
      version = normalizeRequestedVersion(part);
    }
  }
  return { version, replace };
}

async function runInstall(api, requestedVersion, replace = false, configOverride) {
  const releaseOverrides = await resolveInstallReleaseOverrides(api, configOverride);
  return await installPowd({
    version: normalizeRequestedVersion(requestedVersion),
    replace,
    stateDir: api.runtime.state.resolveStateDir(),
    configApi: api.runtime.config,
    logger: api.logger,
    ...releaseOverrides,
  });
}

async function runStatus(api, expectedVersion) {
  const config = await loadConfig(api.runtime.config);
  const status = await collectSetupStatus({
    expectedVersion: normalizeRequestedVersion(expectedVersion),
    stateDir: api.runtime.state.resolveStateDir(),
    config,
  });
  return toPublicSetupStatus(status);
}

export default definePluginEntry({
  id: "powd",
  name: "powd",
  description: "Install and register powd for OpenClaw",
  register(api) {
    void normalizeOwnInstallSpec(api).catch((error) => {
      if (typeof api.logger?.warn === "function") {
        api.logger.warn(`powd-plugin: failed to normalize plugin install spec: ${error.message}`);
      }
    });

    api.registerTool({
      name: "powd_setup_status",
      description:
        "Check whether powd is already installed and registered on this OpenClaw host. " +
        "Use this before asking to install powd, or when powd tools are missing and setup may be broken. " +
        "Do not use this for wallet or miner operations. " +
        "Optionally provide version to compare the current install against a specific powd release.",
      parameters: {
        type: "object",
        additionalProperties: false,
        properties: {
          version: {
            type: "string",
            description: "Optional powd release version to compare against, such as 1.0.0 or 1.0.0-rc.1.",
          },
        },
      },
      async execute(_toolCallId, params) {
        const status = await runStatus(api, extractToolVersion(params));
        return buildStatusToolResult(status);
      },
    });

    api.registerTool({
      name: "powd_install",
      description:
        "Install or repair powd on this OpenClaw host. " +
        "Use this when the user wants powd available in OpenClaw or when the saved powd MCP registration is missing or broken. " +
        "By default this downloads the latest stable powd release from GitHub Releases, installs it locally, and registers mcp.servers.powd. " +
        "Provide version to pin a specific powd release, including prereleases. " +
        "Do not use this for wallet setup or miner control.",
      parameters: {
        type: "object",
        additionalProperties: false,
        properties: {
          version: {
            type: "string",
            description: "Optional powd release version to install, such as 1.0.0 or 1.0.0-rc.1.",
          },
          replace: {
            type: "boolean",
            description:
              "When true, stop the current local powd daemon and replace the installed binary. Use this for upgrades or reinstalls.",
          },
        },
      },
      async execute(_toolCallId, params) {
        const result = await runInstall(api, extractToolVersion(params), extractToolReplace(params));
        return buildInstallToolResult(result);
      },
    });

    api.on("before_tool_call", async (event) => {
      if (event.toolName !== "powd_install") {
        return undefined;
      }

      const version = extractEventVersion(event);
      const replace = extractToolReplace(event?.params) || extractToolReplace(event?.arguments) || extractToolReplace(event?.args);
      const config = await loadConfig(api.runtime.config);
      const status = await collectSetupStatus({
        expectedVersion: version,
        stateDir: api.runtime.state.resolveStateDir(),
        config,
      });

      return {
        requireApproval: buildApprovalRequest(status, version, replace),
      };
    });

    api.registerCommand({
      name: "powd",
      description: "Install or inspect the local powd setup.",
      acceptsArgs: true,
      requireAuth: true,
      handler: async (ctx) => {
        const args = (ctx.args ?? "").trim();
        const parts = args.split(/\s+/).filter(Boolean);
        const action = parts[0] ?? "status";

        if (action === "install") {
          const { version, replace } = parseCommandInstallArgs(parts.slice(1));
          const result = await runInstall(api, version, replace);
          return buildInstallCommandReply(result);
        }

        if (action === "status" || action === "help") {
          const version = normalizeRequestedVersion(parts[1]);
          const status = await runStatus(api, version);
          return buildStatusCommandReply(status);
        }

        return {
          text:
            "Usage:\n" +
            "/powd status [version]\n" +
            "/powd install [version] [--replace]",
        };
      },
    });

    api.registerCli(
      ({ program, config }) => {
        registerPowdCli({
          program,
          api,
          runInstall: async (version, replace) => buildInstallToolResult(await runInstall(api, version, replace, config)),
          runStatus: async (version) => buildStatusToolResult(await runStatus(api, version)),
        });
      },
      {
        descriptors: [
          {
            name: "powd",
            description: "Install or inspect the local powd setup",
            hasSubcommands: true,
          },
        ],
      },
    );
  },
});
