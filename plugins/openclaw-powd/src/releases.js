import {
  POWD_RELEASE_API_BASE,
  POWD_RELEASE_REPO_BASE,
  normalizeVersion,
} from "./constants.js";
import { requestJson } from "./download.js";

function stripTrailingSlash(value) {
  return value.endsWith("/") ? value.slice(0, -1) : value;
}

export async function resolveLatestStableVersion({
  apiBaseOverride,
  fetchImpl,
} = {}) {
  const apiBase = stripTrailingSlash(apiBaseOverride?.trim() || POWD_RELEASE_API_BASE);
  const payload = await requestJson(`${apiBase}/latest`, {
    fetchImpl,
    headers: {
      accept: "application/vnd.github+json",
    },
  });
  const tagName = typeof payload?.tag_name === "string" ? payload.tag_name : "";
  if (!tagName.trim()) {
    throw new Error("failed to resolve latest powd release version");
  }

  return normalizeVersion(tagName);
}

export function parseSha256Text(text) {
  const match = text.match(/\b([a-fA-F0-9]{64})\b/);
  if (!match || !match[1]) {
    throw new Error("invalid sha256 file: expected a 64-character hex digest");
  }
  return match[1].toLowerCase();
}

export function buildReleaseSpec({ version, platform, baseUrlOverride }) {
  const normalizedVersion = normalizeVersion(version);
  if (!platform?.assetSuffix || !platform?.binaryName) {
    throw new Error("unsupported platform");
  }

  const baseRoot = stripTrailingSlash(baseUrlOverride?.trim() || POWD_RELEASE_REPO_BASE);
  const releaseBaseUrl = `${baseRoot}/v${normalizedVersion}`;
  const assetBase = `powd-v${normalizedVersion}-${platform.assetSuffix}`;
  const archiveName = `${assetBase}.tar.gz`;
  const sha256Name = `${archiveName}.sha256`;

  return {
    version: normalizedVersion,
    assetBase,
    archiveName,
    sha256Name,
    binaryName: platform.binaryName,
    archiveUrl: `${releaseBaseUrl}/${archiveName}`,
    sha256Url: `${releaseBaseUrl}/${sha256Name}`,
  };
}
