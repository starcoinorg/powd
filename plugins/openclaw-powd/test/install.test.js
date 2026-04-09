import test from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import crypto from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";
import { buildReleaseSpec } from "../src/releases.js";
import { installPowd } from "../src/install.js";
import { resolvePlatform } from "../src/platform.js";

function sha256(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

function writeTarString(buffer, value, offset, length) {
  const bytes = Buffer.from(value, "utf8");
  bytes.copy(buffer, offset, 0, Math.min(bytes.length, length));
}

function writeTarOctal(buffer, value, offset, length) {
  const digits = Math.max(length - 2, 1);
  const encoded = `${Math.trunc(value).toString(8).padStart(digits, "0")}\0 `;
  buffer.write(encoded.slice(-length), offset, length, "ascii");
}

function createTarHeader(name, size, mode = 0o755) {
  const header = Buffer.alloc(512, 0);
  writeTarString(header, name, 0, 100);
  writeTarOctal(header, mode, 100, 8);
  writeTarOctal(header, 0, 108, 8);
  writeTarOctal(header, 0, 116, 8);
  writeTarOctal(header, size, 124, 12);
  writeTarOctal(header, Math.floor(Date.now() / 1000), 136, 12);
  header.fill(0x20, 148, 156);
  header.write("0", 156, 1, "ascii");
  header.write("ustar", 257, 5, "ascii");
  header.write("00", 263, 2, "ascii");
  const checksum = header.reduce((sum, byte) => sum + byte, 0);
  writeTarOctal(header, checksum, 148, 8);
  return header;
}

async function writeTarGzSingleFile({ archivePath, entryName, filePath }) {
  const data = await fs.readFile(filePath);
  const header = createTarHeader(entryName, data.length);
  const remainder = data.length % 512;
  const padding = remainder === 0 ? Buffer.alloc(0) : Buffer.alloc(512 - remainder, 0);
  const tarBuffer = Buffer.concat([header, data, padding, Buffer.alloc(1024, 0)]);
  const gzipBuffer = zlib.gzipSync(tarBuffer);
  await fs.writeFile(archivePath, gzipBuffer);
}

async function createReleaseFixture(rootDir, version, platform = resolvePlatform("linux", "x64")) {
  const spec = buildReleaseSpec({
    version,
    platform,
    baseUrlOverride: "http://127.0.0.1:0/releases/download",
  });
  const releaseDir = path.join(rootDir, "releases", "download", `v${version}`);
  const stagingDir = path.join(rootDir, "staging");
  await fs.mkdir(releaseDir, { recursive: true });
  await fs.mkdir(stagingDir, { recursive: true });

  const powdPath = path.join(stagingDir, platform.binaryName);
  await fs.writeFile(powdPath, "#!/usr/bin/env sh\necho powd\n", "utf8");
  await fs.chmod(powdPath, 0o755);

  const archivePath = path.join(releaseDir, spec.archiveName);
  await writeTarGzSingleFile({
    archivePath,
    entryName: platform.binaryName,
    filePath: powdPath,
  });
  const archiveBytes = await fs.readFile(archivePath);
  await fs.writeFile(path.join(releaseDir, spec.sha256Name), `${sha256(archiveBytes)}  ${spec.archiveName}\n`, "utf8");

  const latestApiPath = path.join(rootDir, "api", "releases", "latest");
  await fs.mkdir(path.dirname(latestApiPath), { recursive: true });
  await fs.writeFile(latestApiPath, `${JSON.stringify({ tag_name: `v${version}` })}\n`, "utf8");
}

async function withHttpServer(rootDir, fn) {
  const server = http.createServer(async (req, res) => {
    const requestPath = new URL(req.url, "http://127.0.0.1").pathname;
    const filePath = path.join(rootDir, requestPath.replace(/^\/+/, ""));
    try {
      const data = await fs.readFile(filePath);
      res.writeHead(200);
      res.end(data);
    } catch {
      res.writeHead(404);
      res.end("not found");
    }
  });

  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  const port = typeof address === "object" && address ? address.port : 0;

  try {
    await fn(`http://127.0.0.1:${port}/releases/download`);
  } finally {
    await new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())));
  }
}

function createConfigApi(initialConfig = {}) {
  let current = structuredClone(initialConfig);
  return {
    loadConfig() {
      return structuredClone(current);
    },
    async writeConfigFile(nextConfig) {
      current = structuredClone(nextConfig);
    },
    snapshot() {
      return structuredClone(current);
    },
  };
}

test("installPowd downloads the latest stable release when no version is pinned", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "powd-plugin-test-"));
  try {
    const version = "1.2.3";
    await createReleaseFixture(tempRoot, version);

    await withHttpServer(tempRoot, async (baseUrl) => {
      const configApi = createConfigApi({});

      const result = await installPowd({
        stateDir: path.join(tempRoot, "state"),
        configApi,
        releaseBaseUrl: baseUrl,
        releaseApiBaseUrl: baseUrl.replace(/\/releases\/download$/, "/api/releases"),
      });

      assert.equal(result.ok, true);
      assert.equal(result.status.installed, true);
      assert.equal(result.status.registered, true);
      assert.equal(result.status.version, version);
      assert.equal(result.status.mcpCommandMatchesInstall, true);

      const cfg = configApi.snapshot();
      assert.equal(cfg.mcp.servers.powd.args[0], "mcp");
      assert.equal(cfg.mcp.servers.powd.args[1], "serve");
      assert.deepEqual(cfg.plugins.allow, ["powd"]);

      await fs.access(result.status.binaryPath);
      const metadataRaw = await fs.readFile(path.join(tempRoot, "state", "plugins", "powd", "install.json"), "utf8");
      const metadata = JSON.parse(metadataRaw);
      assert.equal(metadata.version, version);
    });
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
});

test("installPowd replaces a foreign powd registration without forcing a network update", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "powd-plugin-test-"));
  try {
    const version = "1.2.3";
    await createReleaseFixture(tempRoot, version);

    await withHttpServer(tempRoot, async (baseUrl) => {
      const stateDir = path.join(tempRoot, "state");
      const managedBinaryPath = path.join(stateDir, "plugins", "powd", "bin", "powd");
      const metadataPath = path.join(stateDir, "plugins", "powd", "install.json");
      await fs.mkdir(path.dirname(managedBinaryPath), { recursive: true });
      await fs.writeFile(managedBinaryPath, "#!/usr/bin/env sh\necho powd\n", "utf8");
      await fs.chmod(managedBinaryPath, 0o755);
      await fs.mkdir(path.dirname(metadataPath), { recursive: true });
      await fs.writeFile(
        metadataPath,
        `${JSON.stringify({ version, binaryPath: managedBinaryPath, installedAt: new Date().toISOString() }, null, 2)}\n`,
        "utf8",
      );
      const configApi = createConfigApi({
        mcp: {
          servers: {
            powd: {
              command: "/opt/custom/powd",
              args: ["mcp", "serve"],
              env: {},
            },
          },
        },
      });

      const result = await installPowd({
        stateDir,
        configApi,
        releaseBaseUrl: baseUrl,
      });

      assert.equal(result.ok, true);
      assert.equal(result.overwroteForeignRegistration, true);
      assert.match(result.message, /replaced/i);
      assert.notEqual(configApi.snapshot().mcp.servers.powd.command, "/opt/custom/powd");
      assert.deepEqual(configApi.snapshot().plugins.allow, ["powd"]);
    });
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
});

test("installPowd accepts an explicit pinned version", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "powd-plugin-test-"));
  try {
    const version = "1.0.0-rc.1";
    await createReleaseFixture(tempRoot, version);

    await withHttpServer(tempRoot, async (baseUrl) => {
      const configApi = createConfigApi({});

      const result = await installPowd({
        version: `v${version}`,
        stateDir: path.join(tempRoot, "state"),
        configApi,
        releaseBaseUrl: baseUrl,
      });

      assert.equal(result.ok, true);
      assert.equal(result.status.installed, true);
      assert.equal(result.status.version, version);
      assert.equal(result.status.registered, true);
    });
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
});

test("installPowd requires --replace before switching an existing install to a different version", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "powd-plugin-test-"));
  try {
    const currentVersion = "1.0.0";
    const requestedVersion = "1.0.1";
    await createReleaseFixture(tempRoot, requestedVersion);

    const stateDir = path.join(tempRoot, "state");
    const managedBinaryPath = path.join(stateDir, "plugins", "powd", "bin", "powd");
    const metadataPath = path.join(stateDir, "plugins", "powd", "install.json");
    await fs.mkdir(path.dirname(managedBinaryPath), { recursive: true });
    await fs.writeFile(managedBinaryPath, "#!/usr/bin/env sh\necho powd\n", "utf8");
    await fs.chmod(managedBinaryPath, 0o755);
    await fs.mkdir(path.dirname(metadataPath), { recursive: true });
    await fs.writeFile(
      metadataPath,
      `${JSON.stringify({ version: currentVersion, binaryPath: managedBinaryPath, installedAt: new Date().toISOString() }, null, 2)}\n`,
      "utf8",
    );
    const configApi = createConfigApi({
      mcp: {
        servers: {
          powd: {
            command: managedBinaryPath,
            args: ["mcp", "serve"],
            env: {},
          },
        },
      },
      plugins: { allow: ["powd"] },
    });

    const result = await installPowd({
      version: requestedVersion,
      stateDir,
      configApi,
    });

    assert.equal(result.ok, false);
    assert.equal(result.replaceRequired, true);
    assert.equal(result.status.version, currentVersion);
    assert.match(result.message, /--replace/);
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
});

test("installPowd replaces an existing install when --replace is requested", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "powd-plugin-test-"));
  try {
    const currentVersion = "1.0.0";
    const nextVersion = "1.0.1";
    await createReleaseFixture(tempRoot, nextVersion);

    await withHttpServer(tempRoot, async (baseUrl) => {
      const stateDir = path.join(tempRoot, "state");
      const managedBinaryPath = path.join(stateDir, "plugins", "powd", "bin", "powd");
      const metadataPath = path.join(stateDir, "plugins", "powd", "install.json");
      await fs.mkdir(path.dirname(managedBinaryPath), { recursive: true });
      await fs.writeFile(managedBinaryPath, "#!/usr/bin/env sh\necho powd-old\n", "utf8");
      await fs.chmod(managedBinaryPath, 0o755);
      await fs.mkdir(path.dirname(metadataPath), { recursive: true });
      await fs.writeFile(
        metadataPath,
        `${JSON.stringify({ version: currentVersion, binaryPath: managedBinaryPath, installedAt: new Date().toISOString() }, null, 2)}\n`,
        "utf8",
      );
      const configApi = createConfigApi({
        mcp: {
          servers: {
            powd: {
              command: managedBinaryPath,
              args: ["mcp", "serve"],
              env: {},
            },
          },
        },
        plugins: { allow: ["powd"] },
      });

      let shutdownCalled = false;
      const result = await installPowd({
        stateDir,
        configApi,
        replace: true,
        releaseBaseUrl: baseUrl,
        releaseApiBaseUrl: baseUrl.replace(/\/releases\/download$/, "/api/releases"),
        shutdownDaemon: async () => {
          shutdownCalled = true;
          return {
            running: true,
            stopped: true,
            socketPath: path.join(tempRoot, "powd.sock"),
          };
        },
      });

      assert.equal(shutdownCalled, true);
      assert.equal(result.ok, true);
      assert.equal(result.replaced, true);
      assert.equal(result.status.version, nextVersion);
      assert.match(result.message, /Restart the OpenClaw gateway/);
    });
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
});

test("installPowd supports darwin arm64 assets when the host platform is Apple Silicon", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "powd-plugin-test-"));
  try {
    const version = "1.2.3";
    const platform = resolvePlatform("darwin", "arm64");
    const spec = buildReleaseSpec({
      version,
      platform,
      baseUrlOverride: "http://127.0.0.1:0/releases/download",
    });
    const releaseDir = path.join(tempRoot, "releases", "download", `v${version}`);
    const stagingDir = path.join(tempRoot, "staging");
    await fs.mkdir(releaseDir, { recursive: true });
    await fs.mkdir(stagingDir, { recursive: true });

    const powdPath = path.join(stagingDir, platform.binaryName);
    await fs.writeFile(powdPath, "#!/usr/bin/env sh\necho powd-macos\n", "utf8");
    await fs.chmod(powdPath, 0o755);

    const archivePath = path.join(releaseDir, spec.archiveName);
    await writeTarGzSingleFile({
      archivePath,
      entryName: platform.binaryName,
      filePath: powdPath,
    });
    const archiveBytes = await fs.readFile(archivePath);
    await fs.writeFile(path.join(releaseDir, spec.sha256Name), `${sha256(archiveBytes)}  ${spec.archiveName}\n`, "utf8");

    await withHttpServer(tempRoot, async (baseUrl) => {
      const configApi = createConfigApi({});

      const result = await installPowd({
        version,
        stateDir: path.join(tempRoot, "state"),
        configApi,
        platform,
        releaseBaseUrl: baseUrl,
      });

      assert.equal(result.ok, true);
      assert.equal(result.status.installed, true);
      assert.equal(result.status.registered, true);
      assert.equal(result.status.version, version);
      assert.equal(result.status.binaryPath?.endsWith("/powd"), true);
      assert.deepEqual(configApi.snapshot().plugins.allow, ["powd"]);
    });
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
});

test("installPowd supports windows x64 assets when the host platform is Windows", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "powd-plugin-test-"));
  try {
    const version = "1.2.3";
    const platform = resolvePlatform("win32", "x64");
    await createReleaseFixture(tempRoot, version, platform);

    await withHttpServer(tempRoot, async (baseUrl) => {
      const configApi = createConfigApi({});

      const result = await installPowd({
        version,
        stateDir: path.join(tempRoot, "state"),
        configApi,
        platform,
        releaseBaseUrl: baseUrl,
      });

      assert.equal(result.ok, true);
      assert.equal(result.status.installed, true);
      assert.equal(result.status.registered, true);
      assert.equal(result.status.version, version);
      assert.match(result.status.binaryPath ?? "", /powd\.exe$/);
      assert.deepEqual(configApi.snapshot().plugins.allow, ["powd"]);
    });
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
});
