import fs from "node:fs/promises";
import path from "node:path";
import { POWD_BINARY_NAME } from "./constants.js";
import { getPowdServer, isManagedPowdServer } from "./config.js";
import { resolvePlatform } from "./platform.js";

async function exists(filePath) {
  try {
    await fs.access(filePath);
    return true;
  } catch {
    return false;
  }
}

async function readInstallRecord(metadataPath) {
  try {
    const raw = await fs.readFile(metadataPath, "utf8");
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed;
    }
  } catch {
    return null;
  }
  return null;
}

export function resolveManagedPaths(stateDir) {
  const rootDir = path.join(stateDir, "plugins", "powd");
  const binDir = path.join(rootDir, "bin");
  return {
    rootDir,
    binDir,
    binaryPath: path.join(binDir, POWD_BINARY_NAME),
    metadataPath: path.join(rootDir, "install.json"),
  };
}

export async function collectSetupStatus({ expectedVersion, stateDir, config, platform = resolvePlatform() }) {
  const paths = resolveManagedPaths(stateDir);
  const binaryExists = await exists(paths.binaryPath);
  const installRecord = await readInstallRecord(paths.metadataPath);
  const version =
    installRecord && typeof installRecord.version === "string" && installRecord.version.trim()
      ? installRecord.version.trim()
      : null;
  const installed = binaryExists;
  const server = getPowdServer(config);
  const registered = server !== null;
  const mcpCommandMatchesInstall = isManagedPowdServer(server, paths.binaryPath);
  const foreignRegistration = registered && !mcpCommandMatchesInstall;
  const platformSupported = Boolean(platform?.supported);

  let message;
  if (!platformSupported) {
    const label = platform?.key ?? `${process.platform}-${process.arch}`;
    message = `powd install is not available on this platform yet (${label}).`;
  } else if (!installed) {
    message = registered
      ? "powd is not installed, but an MCP registration already exists. Reinstalling will repair it."
      : "powd is not installed yet.";
  } else if (!version) {
    message = registered
      ? "powd is installed, but install metadata is missing. Reinstalling will refresh it."
      : "powd is installed, but it is not registered with OpenClaw yet.";
  } else if (expectedVersion && version !== expectedVersion) {
    message = `powd ${version} is installed, but the requested version is ${expectedVersion}. Reinstalling will update it.`;
  } else if (!registered) {
    message = "powd is installed, but it is not registered with OpenClaw yet.";
  } else if (!mcpCommandMatchesInstall) {
    message = "powd is installed, but OpenClaw still points to a different powd registration.";
  } else {
    message = `powd ${version} is installed and registered.`;
  }

  return {
    installed,
    registered,
    version,
    binaryPath: installed ? paths.binaryPath : null,
    mcpCommandMatchesInstall,
    platformSupported,
    message,
    foreignRegistration,
    managedPaths: paths,
    installRecord,
  };
}

export function toPublicSetupStatus(status) {
  return {
    installed: status.installed,
    registered: status.registered,
    version: status.version,
    binaryPath: status.binaryPath,
    mcpCommandMatchesInstall: status.mcpCommandMatchesInstall,
    platformSupported: status.platformSupported,
    message: status.message,
  };
}
