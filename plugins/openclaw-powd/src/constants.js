export const MCP_SERVER_NAME = "powd";
export const POWD_BINARY_NAME = "powd";
export const POWD_RELEASE_BASE_URL_ENV = "POWD_PLUGIN_RELEASE_BASE_URL";
export const POWD_RELEASE_REPO_BASE = "https://github.com/starcoinorg/powd/releases/download";

export function normalizeVersion(version) {
  const normalized = typeof version === "string" ? version.trim() : "";
  if (!normalized) {
    throw new Error("powd version is required");
  }
  return normalized;
}
