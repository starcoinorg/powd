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

async function runInstall(api) {
  const version = api.version?.trim();
  if (!version) {
    throw new Error("powd plugin version is unavailable");
  }

  return await installPowd({
    version,
    stateDir: api.runtime.state.resolveStateDir(),
    configApi: api.runtime.config,
    logger: api.logger,
  });
}

async function runStatus(api) {
  const version = api.version?.trim();
  if (!version) {
    throw new Error("powd plugin version is unavailable");
  }

  const config = await loadConfig(api.runtime.config);
  const status = await collectSetupStatus({
    expectedVersion: version,
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
        "Do not use this for wallet or miner operations.",
      parameters: {
        type: "object",
        additionalProperties: false,
        properties: {},
      },
      async execute() {
        const status = await runStatus(api);
        return buildStatusToolResult(status);
      },
    });

    api.registerTool({
      name: "powd_install",
      description:
        "Install or repair powd on this OpenClaw host. " +
        "Use this when the user wants powd available in OpenClaw or when the saved powd MCP registration is missing or broken. " +
        "This downloads the matching powd release from GitHub Releases, installs it locally, and registers mcp.servers.powd. " +
        "Do not use this for wallet setup or miner control.",
      parameters: {
        type: "object",
        additionalProperties: false,
        properties: {},
      },
      async execute() {
        const result = await runInstall(api);
        return buildInstallToolResult(result);
      },
    });

    api.on("before_tool_call", async (event) => {
      if (event.toolName !== "powd_install") {
        return undefined;
      }

      const version = api.version?.trim();
      if (!version) {
        return {
          block: true,
          blockReason: "powd plugin version is unavailable",
        };
      }

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
        const action = args.split(/\s+/).filter(Boolean)[0] ?? "status";

        if (action === "install") {
          const result = await runInstall(api);
          return buildInstallCommandReply(result);
        }

        if (action === "status" || action === "help") {
          const status = await runStatus(api);
          return buildStatusCommandReply(status);
        }

        return {
          text:
            "Usage:\n" +
            "/powd status\n" +
            "/powd install",
        };
      },
    });

    api.registerCli(
      ({ program }) => {
        registerPowdCli({
          program,
          api,
          runInstall: async () => buildInstallToolResult(await runInstall(api)),
          runStatus: async () => buildStatusToolResult(await runStatus(api)),
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
