import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import zlib from "node:zlib";
import { resolvePlatform } from "../plugins/openclaw-powd/src/platform.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = process.env.POWD_REPO_ROOT ?? path.resolve(__dirname, "..");
const openClawRoot = process.env.POWD_OPENCLAW_ROOT ?? path.join(repoRoot, ".tmp", "openclaw-plugin");
const smokeEnv = {
  ...process.env,
  POWD_REPO_ROOT: repoRoot,
  POWD_OPENCLAW_ROOT: openClawRoot,
  OPENCLAW_HOME: process.env.OPENCLAW_HOME ?? path.join(openClawRoot, "home"),
  XDG_CONFIG_HOME: process.env.XDG_CONFIG_HOME ?? path.join(openClawRoot, "xdg-config"),
  XDG_STATE_HOME: process.env.XDG_STATE_HOME ?? path.join(openClawRoot, "xdg-state"),
};

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
  await fs.mkdir(path.dirname(archivePath), { recursive: true });
  await fs.writeFile(archivePath, zlib.gzipSync(tarBuffer));
}

async function runCommand(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? smokeEnv,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    child.on("error", reject);
    child.on("close", (code) => {
      if (code === 0) {
        resolve({ stdout, stderr, output: stdout + stderr });
        return;
      }
      reject(
        new Error(
          `${command} ${args.join(" ")} failed with exit code ${code}\n${stdout}${stderr}`.trim(),
        ),
      );
    });
  });
}

async function ensureCommand(command, args = ["--version"]) {
  try {
    await runCommand(command, args);
  } catch (error) {
    throw new Error(`missing required command: ${command}\n${error.message}`);
  }
}

function extractJson(text) {
  let start = -1;
  let depth = 0;
  let inString = false;
  let escaped = false;

  for (let index = 0; index < text.length; index += 1) {
    const char = text[index];

    if (start === -1) {
      if (char === "{") {
        start = index;
        depth = 1;
      }
      continue;
    }

    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (char === "\\") {
        escaped = true;
      } else if (char === "\"") {
        inString = false;
      }
      continue;
    }

    if (char === "\"") {
      inString = true;
      continue;
    }

    if (char === "{") {
      depth += 1;
      continue;
    }

    if (char !== "}") {
      continue;
    }

    depth -= 1;
    if (depth !== 0) {
      continue;
    }

    return JSON.parse(text.slice(start, index + 1));
  }

  throw new Error(`expected JSON in command output:\n${text}`);
}

async function runJsonCommand(command, args, options = {}) {
  const result = await runCommand(command, args, options);
  const text = result.stdout.trim() ? result.stdout : result.output;
  return extractJson(text);
}

async function runOpenClaw(args, options = {}) {
  return await runCommand("openclaw", args, options);
}

async function runOpenClawJson(args, options = {}) {
  return await runJsonCommand("openclaw", args, options);
}

function resolvePowdBinaryPath() {
  return path.join(repoRoot, "target", "debug", process.platform === "win32" ? "powd.exe" : "powd");
}

function readCargoVersion() {
  return fs
    .readFile(path.join(repoRoot, "Cargo.toml"), "utf8")
    .then((raw) => raw.match(/^version = "([^"]+)"/m)?.[1] ?? "")
    .then((version) => {
      if (!version) {
        throw new Error("failed to resolve powd version from Cargo.toml");
      }
      return version;
    });
}

async function createReleaseFixture({ version, platform, powdBinaryPath, rootDir }) {
  const releaseDir = path.join(rootDir, "releases", "download", `v${version}`);
  const archiveName = `powd-v${version}-${platform.assetSuffix}.tar.gz`;
  const archivePath = path.join(releaseDir, archiveName);
  const shaPath = `${archivePath}.sha256`;

  await writeTarGzSingleFile({
    archivePath,
    entryName: platform.binaryName,
    filePath: powdBinaryPath,
  });

  const archiveBytes = await fs.readFile(archivePath);
  await fs.writeFile(shaPath, `${sha256(archiveBytes)}  ${archiveName}\n`, "utf8");

  const latestApiPath = path.join(rootDir, "api", "releases", "latest");
  await fs.mkdir(path.dirname(latestApiPath), { recursive: true });
  await fs.writeFile(latestApiPath, `${JSON.stringify({ tag_name: `v${version}` })}\n`, "utf8");
}

async function withHttpServer(rootDir, fn) {
  const server = http.createServer(async (req, res) => {
    const requestPath = new URL(req.url, "http://127.0.0.1").pathname.replace(/^\/+/, "");
    const filePath = path.join(rootDir, requestPath);
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
    await fn(port);
  } finally {
    await new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())));
  }
}

async function main() {
  await fs.rm(openClawRoot, { recursive: true, force: true });
  await fs.mkdir(smokeEnv.OPENCLAW_HOME, { recursive: true });
  await fs.mkdir(smokeEnv.XDG_CONFIG_HOME, { recursive: true });
  await fs.mkdir(smokeEnv.XDG_STATE_HOME, { recursive: true });

  await ensureCommand("cargo");
  await ensureCommand("node");
  await ensureCommand("npm");
  await ensureCommand("openclaw");
  await ensureCommand("openclaw-bootstrap", []);

  const platform = resolvePlatform(process.platform, process.arch);
  if (!platform?.supported) {
    throw new Error(`powd plugin smoke requires a supported host platform (${process.platform}:${process.arch})`);
  }

  await runCommand("cargo", ["build", "--quiet", "--bin", "powd"]);
  const version = await readCargoVersion();
  const powdBinaryPath = resolvePowdBinaryPath();
  const fixtureRoot = path.join(openClawRoot, "release-fixture");
  await createReleaseFixture({ version, platform, powdBinaryPath, rootDir: fixtureRoot });

  await withHttpServer(fixtureRoot, async (port) => {
    const pluginDir = path.join(repoRoot, "plugins", "openclaw-powd");
    const packed = await runCommand("npm", ["pack", "--silent"], { cwd: pluginDir });
    const pluginName = packed.stdout.trim().split(/\r?\n/).filter(Boolean).at(-1);
    if (!pluginName) {
      throw new Error("npm pack did not produce a plugin archive name");
    }
      const pluginPath = path.join(pluginDir, pluginName);

      try {
        await runOpenClaw(["plugins", "install", pluginPath]);
        await runOpenClaw(["gateway", "restart"]);

        const inspect = await runOpenClawJson(["plugins", "inspect", "powd", "--json"]);
        assert.equal(inspect.plugin.id, "powd");

      const before = await runOpenClawJson(["powd", "status", "--json"]);
      assert.equal(before.installed, false);
      assert.equal(before.registered, false);

      const releaseBaseUrl = `http://127.0.0.1:${port}/releases/download`;
      const releaseApiBaseUrl = `http://127.0.0.1:${port}/api/releases`;
      await runOpenClaw([
        "config",
        "set",
        "plugins.entries.powd.config.releaseBaseUrl",
        JSON.stringify(releaseBaseUrl),
      ]);
      await runOpenClaw([
        "config",
        "set",
        "plugins.entries.powd.config.releaseApiBaseUrl",
        JSON.stringify(releaseApiBaseUrl),
      ]);

      const install = await runOpenClawJson(["powd", "install", "--json"]);
      assert.equal(install.installed, true);
      assert.equal(install.registered, true);
      assert.equal(install.version, version);
      assert.equal(install.mcpCommandMatchesInstall, true);

      const saved = await runOpenClawJson(["mcp", "show", "powd", "--json"]);
      assert.equal(saved.command, install.binaryPath);
      assert.deepEqual(saved.args, ["mcp", "serve"]);
      assert.deepEqual(saved.env, {});

      const workspace = (await runCommand("openclaw-bootstrap", [])).stdout.trim();
      const materializeWorkspace = path.join(openClawRoot, "materialize-workspace");
      const materializeTest = path.join(openClawRoot, "powd-plugin-materialize.test.ts");
      await fs.writeFile(
        materializeTest,
        `
import fs from "node:fs/promises";
import { afterAll, expect, it } from "vitest";

let runtime;

afterAll(async () => {
  await runtime?.dispose();
});

it("materializes powd MCP tools from the plugin-managed registration", async () => {
  const { createBundleMcpToolRuntime } = await import(
    \`\${process.env.OPENCLAW_WORKSPACE_ROOT}/src/agents/pi-bundle-mcp-tools.ts\`,
  );
  const workspaceDir = process.env.OPENCLAW_MATERIALIZE_WORKSPACE;
  const server = JSON.parse(process.env.SERVER_JSON);
  await fs.mkdir(workspaceDir, { recursive: true });

  runtime = await createBundleMcpToolRuntime({
    workspaceDir,
    cfg: {
      mcp: {
        servers: {
          powd: server,
        },
      },
    },
  });

  expect(runtime.tools).toHaveLength(9);
  const names = runtime.tools.map((tool) => tool.name).toSorted();
  expect(names).toContain("powd__wallet_set");
  expect(names).toContain("powd__wallet_show");
  expect(names).toContain("powd__wallet_reward");
  expect(names).toContain("powd__miner_status");
  expect(names).toContain("powd__miner_set_mode");
});
`,
        "utf8",
      );

      try {
        await runCommand(
          "node",
          ["scripts/run-vitest.mjs", "run", "--config", "vitest.unit.config.ts", materializeTest],
          {
            cwd: workspace,
            env: {
              ...smokeEnv,
              SERVER_JSON: JSON.stringify(saved),
              OPENCLAW_WORKSPACE_ROOT: workspace,
              OPENCLAW_MATERIALIZE_WORKSPACE: materializeWorkspace,
            },
          },
        );
      } finally {
        await fs.rm(materializeTest, { force: true });
      }
    } finally {
      await fs.rm(pluginPath, { force: true });
    }
  });

  process.stdout.write("OpenClaw plugin smoke passed\n");
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : String(error));
  process.exitCode = 1;
});
