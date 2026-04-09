import test from "node:test";
import assert from "node:assert/strict";
import { execFile as execFileCallback } from "node:child_process";
import http from "node:http";
import crypto from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";
import { buildReleaseSpec } from "../src/releases.js";
import { installPowd } from "../src/install.js";
import { resolvePlatform } from "../src/platform.js";

const execFile = promisify(execFileCallback);

function sha256(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

async function createReleaseFixture(rootDir, version) {
  const platform = resolvePlatform("linux", "x64");
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
  await execFile("tar", ["-C", stagingDir, "-czf", archivePath, platform.binaryName]);
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
      process.env.POWD_PLUGIN_RELEASE_BASE_URL = baseUrl;
      process.env.POWD_PLUGIN_RELEASE_API_BASE_URL = baseUrl.replace(/\/releases\/download$/, "/api/releases");
      const configApi = createConfigApi({});

      const result = await installPowd({
        stateDir: path.join(tempRoot, "state"),
        configApi,
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
    delete process.env.POWD_PLUGIN_RELEASE_BASE_URL;
    delete process.env.POWD_PLUGIN_RELEASE_API_BASE_URL;
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
});

test("installPowd replaces a foreign powd registration without forcing a network update", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "powd-plugin-test-"));
  try {
    const version = "1.2.3";
    await createReleaseFixture(tempRoot, version);

    await withHttpServer(tempRoot, async (baseUrl) => {
      process.env.POWD_PLUGIN_RELEASE_BASE_URL = baseUrl;
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
      });

      assert.equal(result.ok, true);
      assert.equal(result.overwroteForeignRegistration, true);
      assert.match(result.message, /replaced/i);
      assert.notEqual(configApi.snapshot().mcp.servers.powd.command, "/opt/custom/powd");
      assert.deepEqual(configApi.snapshot().plugins.allow, ["powd"]);
    });
  } finally {
    delete process.env.POWD_PLUGIN_RELEASE_BASE_URL;
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
});

test("installPowd accepts an explicit pinned version", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "powd-plugin-test-"));
  try {
    const version = "1.0.0-rc.1";
    await createReleaseFixture(tempRoot, version);

    await withHttpServer(tempRoot, async (baseUrl) => {
      process.env.POWD_PLUGIN_RELEASE_BASE_URL = baseUrl;
      const configApi = createConfigApi({});

      const result = await installPowd({
        version: `v${version}`,
        stateDir: path.join(tempRoot, "state"),
        configApi,
      });

      assert.equal(result.ok, true);
      assert.equal(result.status.installed, true);
      assert.equal(result.status.version, version);
      assert.equal(result.status.registered, true);
    });
  } finally {
    delete process.env.POWD_PLUGIN_RELEASE_BASE_URL;
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
});
