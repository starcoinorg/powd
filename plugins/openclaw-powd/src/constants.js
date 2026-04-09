export const MCP_SERVER_NAME = "powd";
export const POWD_PLUGIN_CLAWHUB_SPEC = "clawhub:@starcoinorg/openclaw-powd";
export const POWD_RELEASE_REPO_BASE = "https://github.com/starcoinorg/powd/releases/download";
export const POWD_RELEASE_API_BASE = "https://api.github.com/repos/starcoinorg/powd/releases";

export function normalizeVersion(version) {
  const normalized = typeof version === "string" ? version.trim().replace(/^[vV]/, "") : "";
  if (!normalized) {
    throw new Error("powd version is required");
  }
  return normalized;
}
