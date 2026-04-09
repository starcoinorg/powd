export const PLATFORM_MATRIX = {
  "linux:x64": {
    key: "linux-x64",
    assetSuffix: "linux-x86_64",
    binaryName: "powd",
    supported: true,
  },
  "darwin:x64": {
    key: "darwin-x64",
    assetSuffix: "darwin-x86_64",
    binaryName: "powd",
    supported: false,
  },
  "darwin:arm64": {
    key: "darwin-arm64",
    assetSuffix: "darwin-arm64",
    binaryName: "powd",
    supported: true,
  },
  "win32:x64": {
    key: "win32-x64",
    assetSuffix: "windows-x86_64",
    binaryName: "powd.exe",
    supported: false,
  },
};

export function resolvePlatform(platform = process.platform, arch = process.arch) {
  return PLATFORM_MATRIX[`${platform}:${arch}`] ?? null;
}
