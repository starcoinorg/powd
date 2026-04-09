import crypto from "node:crypto";
import { createReadStream } from "node:fs";
import fs from "node:fs/promises";
import path from "node:path";
import { normalizeVersion } from "./constants.js";
import { buildReleaseSpec, parseSha256Text, resolveLatestStableVersion } from "./releases.js";
import { resolvePlatform } from "./platform.js";
import { collectSetupStatus, resolveManagedPaths, toPublicSetupStatus } from "./status.js";
import { upsertPowdPluginAllow, upsertPowdServer } from "./config.js";
import { downloadFile } from "./download.js";
import { extractBinaryFromArchive } from "./archive.js";

function log(logger, level, message) {
  const writer = logger?.[level];
  if (typeof writer === "function") {
    writer(message);
  }
}

async function writeJsonAtomic(targetPath, value) {
  await fs.mkdir(path.dirname(targetPath), { recursive: true });
  const tempPath = `${targetPath}.tmp-${process.pid}-${Date.now()}`;
  await fs.writeFile(tempPath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
  await fs.rename(tempPath, targetPath);
}

async function sha256File(filePath) {
  const hash = crypto.createHash("sha256");
  for await (const chunk of createReadStream(filePath)) {
    hash.update(chunk);
  }
  return hash.digest("hex");
}
async function installReleaseBinary({ version, stateDir, logger, releaseBaseUrl, fetchImpl }) {
  const platform = resolvePlatform();
  if (!platform?.supported) {
    throw new Error("powd install is not available on this platform yet");
  }

  const managedPaths = resolveManagedPaths(stateDir);
  await fs.mkdir(managedPaths.rootDir, { recursive: true });
  const tempRoot = await fs.mkdtemp(path.join(managedPaths.rootDir, "download-"));

  try {
    const release = buildReleaseSpec({ version, platform, baseUrlOverride: releaseBaseUrl });
    const archivePath = path.join(tempRoot, release.archiveName);
    const sha256Path = path.join(tempRoot, release.sha256Name);
    const extractDir = path.join(tempRoot, "extract");

    log(logger, "info", `powd-plugin: downloading ${release.archiveUrl}`);
    await downloadFile(release.archiveUrl, archivePath, { fetchImpl });
    await downloadFile(release.sha256Url, sha256Path, { fetchImpl });

    const expectedSha256 = parseSha256Text(await fs.readFile(sha256Path, "utf8"));
    const actualSha256 = await sha256File(archivePath);
    if (actualSha256 !== expectedSha256) {
      throw new Error(`sha256 mismatch for ${release.archiveName}`);
    }

    await fs.mkdir(extractDir, { recursive: true });
    await extractBinaryFromArchive({
      archivePath,
      extractDir,
      binaryName: release.binaryName,
      archiveName: release.archiveName,
    });

    const extractedBinaryPath = path.join(extractDir, release.binaryName);
    await fs.access(extractedBinaryPath);

    await fs.mkdir(managedPaths.binDir, { recursive: true });
    const tempBinaryPath = `${managedPaths.binaryPath}.next`;
    await fs.copyFile(extractedBinaryPath, tempBinaryPath);
    await fs.chmod(tempBinaryPath, 0o755);
    await fs.rename(tempBinaryPath, managedPaths.binaryPath);

    await writeJsonAtomic(managedPaths.metadataPath, {
      version,
      assetName: release.archiveName,
      binaryPath: managedPaths.binaryPath,
      sha256: actualSha256,
      installedAt: new Date().toISOString(),
    });

    return {
      managedPaths,
      release,
      sha256: actualSha256,
    };
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
}

function buildInstallMessage(params) {
  if (
    params.status.installed &&
    params.status.registered &&
    params.status.mcpCommandMatchesInstall &&
    !params.downloaded &&
    !params.overwroteForeignRegistration
  ) {
    return "powd is already installed and registered.";
  }

  const lines = ["powd is installed and registered with OpenClaw."];
  if (params.downloaded) {
    lines.push(`Installed version: ${params.version}.`);
  }
  if (params.overwroteForeignRegistration) {
    lines.push("An existing powd MCP registration was replaced.");
  }
  lines.push("Mining has not started yet.");
  lines.push("Next, you can ask OpenClaw to set your wallet, show mining status, or start mining.");
  lines.push("If powd tools do not appear immediately, restart the OpenClaw gateway.");
  return lines.join("\n");
}

export async function installPowd({
  version,
  stateDir,
  configApi,
  logger,
  releaseBaseUrl,
  releaseApiBaseUrl,
  fetchImpl,
}) {
  const currentConfig = await Promise.resolve(configApi.loadConfig());
  const initialStatus = await collectSetupStatus({
    stateDir,
    config: currentConfig,
  });

  if (!initialStatus.platformSupported) {
    return {
      ok: false,
      status: toPublicSetupStatus(initialStatus),
      message: initialStatus.message,
    };
  }

  let downloaded = false;
  let targetVersion = null;

  if (typeof version === "string" && version.trim()) {
    targetVersion = normalizeVersion(version);
  } else if (!initialStatus.installed || !initialStatus.version) {
    targetVersion = await resolveLatestStableVersion({
      apiBaseOverride: releaseApiBaseUrl,
      fetchImpl,
    });
  }

  if (targetVersion && (!initialStatus.installed || initialStatus.version !== targetVersion)) {
    await installReleaseBinary({
      version: targetVersion,
      stateDir,
      logger,
      releaseBaseUrl,
      fetchImpl,
    });
    downloaded = true;
  }

  const configBeforeWrite = downloaded ? await Promise.resolve(configApi.loadConfig()) : currentConfig;
  const statusBeforeWrite = await collectSetupStatus({
    expectedVersion: targetVersion,
    stateDir,
    config: configBeforeWrite,
  });
  const overwroteForeignRegistration = statusBeforeWrite.foreignRegistration;

  let finalConfig = configBeforeWrite;
  const needsServerUpdate = !statusBeforeWrite.registered || !statusBeforeWrite.mcpCommandMatchesInstall;
  const needsPluginAllow = !(Array.isArray(configBeforeWrite?.plugins?.allow) && configBeforeWrite.plugins.allow.includes("powd"));

  if (needsServerUpdate || needsPluginAllow) {
    finalConfig = configBeforeWrite;
    if (needsServerUpdate) {
      finalConfig = upsertPowdServer(finalConfig, statusBeforeWrite.managedPaths.binaryPath);
    }
    if (needsPluginAllow) {
      finalConfig = upsertPowdPluginAllow(finalConfig);
    }
    await configApi.writeConfigFile(finalConfig);
  }

  const finalStatus = await collectSetupStatus({
    expectedVersion: targetVersion,
    stateDir,
    config: finalConfig,
  });

  return {
    ok: true,
    status: toPublicSetupStatus(finalStatus),
    downloaded,
    overwroteForeignRegistration,
    message: buildInstallMessage({
      downloaded,
      overwroteForeignRegistration,
      status: finalStatus,
      version: targetVersion ?? finalStatus.version,
    }),
  };
}
