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
import { collectSetupStatus, toPublicSetupStatus } from "./src/status.js";

async function loadConfig(configApi) {
  return await Promise.resolve(configApi.loadConfig());
}

function normalizeRequestedVersion(version) {
  return typeof version === "string" && version.trim() ? version.trim() : undefined;
}

function extractToolVersion(params) {
  return normalizeRequestedVersion(params?.version);
}

function extractEventVersion(event) {
  return (
    normalizeRequestedVersion(event?.params?.version) ??
    normalizeRequestedVersion(event?.arguments?.version) ??
    normalizeRequestedVersion(event?.args?.version)
  );
}

async function runInstall(api, requestedVersion) {
  return await installPowd({
    version: normalizeRequestedVersion(requestedVersion),
    stateDir: api.runtime.state.resolveStateDir(),
    configApi: api.runtime.config,
    logger: api.logger,
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
        },
      },
      async execute(_toolCallId, params) {
        const result = await runInstall(api, extractToolVersion(params));
        return buildInstallToolResult(result);
      },
    });

    api.on("before_tool_call", async (event) => {
      if (event.toolName !== "powd_install") {
        return undefined;
      }

      const version = extractEventVersion(event);
      const config = await loadConfig(api.runtime.config);
      const status = await collectSetupStatus({
        expectedVersion: version,
        stateDir: api.runtime.state.resolveStateDir(),
        config,
      });

      return {
        requireApproval: buildApprovalRequest(status, version),
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
        const version = normalizeRequestedVersion(parts[1]);

        if (action === "install") {
          const result = await runInstall(api, version);
          return buildInstallCommandReply(result);
        }

        if (action === "status" || action === "help") {
          const status = await runStatus(api, version);
          return buildStatusCommandReply(status);
        }

        return {
          text:
            "Usage:\n" +
            "/powd status [version]\n" +
            "/powd install [version]",
        };
      },
    });

    api.registerCli(
      ({ program }) => {
        registerPowdCli({
          program,
          api,
          runInstall: async (version) => buildInstallToolResult(await runInstall(api, version)),
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
