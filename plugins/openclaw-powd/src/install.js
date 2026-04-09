import crypto from "node:crypto";
import { createReadStream, createWriteStream } from "node:fs";
import fs from "node:fs/promises";
import path from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import zlib from "node:zlib";
import { buildReleaseSpec, parseSha256Text } from "./releases.js";
import { resolvePlatform } from "./platform.js";
import { collectSetupStatus, resolveManagedPaths, toPublicSetupStatus } from "./status.js";
import { upsertPowdServer } from "./config.js";

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

async function downloadFile(url, destinationPath) {
  const response = await fetch(url);
  if (!response.ok || !response.body) {
    throw new Error(`download failed (${response.status} ${response.statusText}) for ${url}`);
  }

  await fs.mkdir(path.dirname(destinationPath), { recursive: true });
  await pipeline(Readable.fromWeb(response.body), createWriteStream(destinationPath));
}

async function sha256File(filePath) {
  const hash = crypto.createHash("sha256");
  for await (const chunk of createReadStream(filePath)) {
    hash.update(chunk);
  }
  return hash.digest("hex");
}

function readTarString(buffer, start, end) {
  const value = buffer.subarray(start, end).toString("utf8");
  const nul = value.indexOf("\0");
  return (nul === -1 ? value : value.slice(0, nul)).trim();
}

function readTarSize(buffer, start, end) {
  const raw = readTarString(buffer, start, end).replace(/\0/g, "").trim();
  if (!raw) {
    return 0;
  }
  return Number.parseInt(raw, 8);
}

async function extractBinaryFromArchive({ archivePath, extractDir, binaryName, archiveName }) {
  const archiveBytes = await fs.readFile(archivePath);
  const tarBytes = zlib.gunzipSync(archiveBytes);
  let offset = 0;
  let extracted = false;

  while (offset + 512 <= tarBytes.length) {
    const header = tarBytes.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) {
      break;
    }

    const name = readTarString(header, 0, 100);
    const prefix = readTarString(header, 345, 500);
    const entryName = prefix ? `${prefix}/${name}` : name;
    const entryType = readTarString(header, 156, 157) || "0";
    const entrySize = readTarSize(header, 124, 136);
    const contentStart = offset + 512;
    const contentEnd = contentStart + entrySize;
    if (contentEnd > tarBytes.length) {
      throw new Error(`archive ${archiveName} is truncated`);
    }

    const normalizedEntryName = entryName.replace(/^\.\/+/, "");
    const isRegularFile = entryType === "0" || entryType === "";
    if (!extracted && isRegularFile && path.posix.basename(normalizedEntryName) === binaryName) {
      const destinationPath = path.join(extractDir, binaryName);
      await fs.mkdir(path.dirname(destinationPath), { recursive: true });
      await fs.writeFile(destinationPath, tarBytes.subarray(contentStart, contentEnd));
      extracted = true;
    }

    const alignedSize = Math.ceil(entrySize / 512) * 512;
    offset = contentStart + alignedSize;
  }

  if (!extracted) {
    throw new Error(`archive ${archiveName} does not contain ${binaryName}`);
  }
}

async function installReleaseBinary({ version, stateDir, logger }) {
  const platform = resolvePlatform();
  if (!platform?.supported) {
    throw new Error("powd install is not available on this platform yet");
  }

  const managedPaths = resolveManagedPaths(stateDir);
  await fs.mkdir(managedPaths.rootDir, { recursive: true });
  const tempRoot = await fs.mkdtemp(path.join(managedPaths.rootDir, "download-"));

  try {
    const release = buildReleaseSpec({ version, platform });
    const archivePath = path.join(tempRoot, release.archiveName);
    const sha256Path = path.join(tempRoot, release.sha256Name);
    const extractDir = path.join(tempRoot, "extract");

    log(logger, "info", `powd-plugin: downloading ${release.archiveUrl}`);
    await downloadFile(release.archiveUrl, archivePath);
    await downloadFile(release.sha256Url, sha256Path);

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

export async function installPowd({ version, stateDir, configApi, logger }) {
  const currentConfig = await Promise.resolve(configApi.loadConfig());
  const initialStatus = await collectSetupStatus({
    expectedVersion: version,
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
  if (!initialStatus.installed || initialStatus.version !== version) {
    await installReleaseBinary({ version, stateDir, logger });
    downloaded = true;
  }

  const configBeforeWrite = downloaded ? await Promise.resolve(configApi.loadConfig()) : currentConfig;
  const statusBeforeWrite = await collectSetupStatus({
    expectedVersion: version,
    stateDir,
    config: configBeforeWrite,
  });
  const overwroteForeignRegistration = statusBeforeWrite.foreignRegistration;

  let finalConfig = configBeforeWrite;
  if (!statusBeforeWrite.registered || !statusBeforeWrite.mcpCommandMatchesInstall) {
    finalConfig = upsertPowdServer(configBeforeWrite, statusBeforeWrite.managedPaths.binaryPath);
    await configApi.writeConfigFile(finalConfig);
  }

  const finalStatus = await collectSetupStatus({
    expectedVersion: version,
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
      version,
    }),
  };
}
