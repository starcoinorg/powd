import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { spawn } from "node:child_process";
import { once } from "node:events";
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
  const digits = Math.max(length - 1, 1);
  const octal = Math.trunc(value).toString(8);
  if (octal.length > digits) {
    throw new Error(`tar field overflow: ${value} does not fit in ${length} bytes`);
  }
  const encoded = `${octal.padStart(digits, "0")}\0`;
  buffer.write(encoded, offset, length, "ascii");
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

function resolveCommand(command) {
  if (process.platform !== "win32") {
    return command;
  }
  if (command.includes("\\") || command.includes("/") || path.extname(command)) {
    return command;
  }
  if (command === "npm" || command === "openclaw") {
    return `${command}.cmd`;
  }
  return command;
}

function quoteCmdArg(value) {
  if (!value) {
    return "\"\"";
  }
  if (!/[\s"&()<>^|]/.test(value)) {
    return value;
  }
  return `"${value.replace(/"/g, '""')}"`;
}

async function runCommand(command, args, options = {}) {
  const resolvedCommand = resolveCommand(command);
  const useCmdShell =
    process.platform === "win32" && path.extname(resolvedCommand).toLowerCase() === ".cmd";
  const spawnCommand = useCmdShell ? process.env.ComSpec ?? "cmd.exe" : resolvedCommand;
  const spawnArgs = useCmdShell
    ? ["/d", "/s", "/c", [resolvedCommand, ...args].map(quoteCmdArg).join(" ")]
    : args;
  return await new Promise((resolve, reject) => {
    const child = spawn(spawnCommand, spawnArgs, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? smokeEnv,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const timeoutMs = options.timeoutMs ?? 0;
    let settled = false;
    let stdout = "";
    let stderr = "";
    const timeout =
      timeoutMs > 0
        ? setTimeout(() => {
            if (settled) {
              return;
            }
            child.kill();
            reject(
              new Error(
                `${spawnCommand} ${spawnArgs.join(" ")} timed out after ${timeoutMs}ms\n${stdout}${stderr}`.trim(),
              ),
            );
          }, timeoutMs)
        : null;
    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    child.on("error", (error) => {
      settled = true;
      if (timeout) {
        clearTimeout(timeout);
      }
      reject(error);
    });
    child.on("close", (code) => {
      if (settled) {
        return;
      }
      settled = true;
      if (timeout) {
        clearTimeout(timeout);
      }
      if (code === 0) {
        resolve({ stdout, stderr, output: stdout + stderr });
        return;
      }
      reject(
        new Error(
          `${spawnCommand} ${spawnArgs.join(" ")} failed with exit code ${code}\n${stdout}${stderr}`.trim(),
        ),
      );
    });
  });
}

async function ensureCommand(command, args = ["--version"]) {
  try {
    await runCommand(command, args, { timeoutMs: 30_000 });
  } catch (error) {
    throw new Error(`missing required command: ${command}\n${error.message}`);
  }
}

async function listMcpTools(server) {
  const child = spawn(server.command, server.args ?? [], {
    cwd: repoRoot,
    env: {
      ...smokeEnv,
      ...(server.env ?? {}),
    },
    stdio: ["pipe", "pipe", "pipe"],
  });

  let stderr = "";
  let buffer = Buffer.alloc(0);
  const pending = new Map();
  let nextId = 1;
  const requestTimeoutMs = 15_000;

  const rejectAll = (error) => {
    for (const { reject } of pending.values()) {
      reject(error);
    }
    pending.clear();
  };

  child.stderr.on("data", (chunk) => {
    stderr += chunk.toString();
  });

  child.stdout.on("data", (chunk) => {
    buffer = Buffer.concat([buffer, chunk]);
    for (;;) {
      const headerEnd = buffer.indexOf("\r\n\r\n");
      if (headerEnd === -1) {
        break;
      }
      const headerText = buffer.subarray(0, headerEnd).toString("utf8");
      const contentLengthLine = headerText
        .split("\r\n")
        .find((line) => line.toLowerCase().startsWith("content-length:"));
      if (!contentLengthLine) {
        rejectAll(new Error(`missing Content-Length header in MCP response\n${headerText}`));
        return;
      }
      const contentLength = Number.parseInt(contentLengthLine.split(":")[1]?.trim() ?? "", 10);
      if (!Number.isFinite(contentLength) || contentLength < 0) {
        rejectAll(new Error(`invalid Content-Length header in MCP response\n${headerText}`));
        return;
      }
      const frameLength = headerEnd + 4 + contentLength;
      if (buffer.length < frameLength) {
        break;
      }
      const payloadText = buffer.subarray(headerEnd + 4, frameLength).toString("utf8");
      buffer = buffer.subarray(frameLength);
      let message;
      try {
        message = JSON.parse(payloadText);
      } catch (error) {
        rejectAll(new Error(`invalid MCP JSON payload: ${payloadText}\n${error}`));
        return;
      }
      if (message.id == null) {
        continue;
      }
      const waiter = pending.get(String(message.id));
      if (!waiter) {
        continue;
      }
      pending.delete(String(message.id));
      if (message.error) {
        waiter.reject(
          new Error(`MCP request failed: ${JSON.stringify(message.error)}\n${stderr}`.trim()),
        );
        continue;
      }
      waiter.resolve(message.result);
    }
  });

  child.once("error", (error) => rejectAll(error));
  child.once("exit", (code, signal) => {
    if (pending.size === 0) {
      return;
    }
    const reason =
      signal != null
        ? `signal ${signal}`
        : `exit code ${code ?? "unknown"}${stderr ? `\n${stderr}` : ""}`;
    rejectAll(new Error(`powd MCP server exited before responding (${reason})`));
  });

  const request = (method, params) =>
    new Promise((resolve, reject) => {
      const id = nextId++;
      const timeout = setTimeout(() => {
        pending.delete(String(id));
        reject(new Error(`MCP request timed out: ${method}`));
      }, requestTimeoutMs);
      pending.set(String(id), {
        resolve: (value) => {
          clearTimeout(timeout);
          resolve(value);
        },
        reject: (error) => {
          clearTimeout(timeout);
          reject(error);
        },
      });
      const payload = Buffer.from(
        JSON.stringify({ jsonrpc: "2.0", id, method, params }),
        "utf8",
      );
      const header = Buffer.from(`Content-Length: ${payload.length}\r\n\r\n`, "utf8");
      child.stdin.write(Buffer.concat([header, payload]));
    });

  try {
    const initialize = await request("initialize", {
      protocolVersion: "2025-11-25",
      capabilities: {},
      clientInfo: { name: "powd-plugin-smoke", version: "1.0.0" },
    });
    assert.equal(initialize.protocolVersion, "2025-11-25");
    const notificationPayload = Buffer.from(
      JSON.stringify({
        jsonrpc: "2.0",
        method: "notifications/initialized",
        params: {},
      }),
      "utf8",
    );
    child.stdin.write(
      Buffer.concat([
        Buffer.from(`Content-Length: ${notificationPayload.length}\r\n\r\n`, "utf8"),
        notificationPayload,
      ]),
    );
    const tools = await request("tools/list", {});
    return tools.tools ?? [];
  } finally {
    child.stdin.end();
    child.kill();
    await once(child, "close").catch(() => {});
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

  const platform = resolvePlatform(process.platform, process.arch);
  if (!platform?.supported) {
    throw new Error(`powd plugin smoke requires a supported host platform (${process.platform}:${process.arch})`);
  }

  process.stderr.write(`[smoke] building powd for ${process.platform}:${process.arch}\n`);
  await runCommand("cargo", ["build", "--quiet", "--bin", "powd"], { timeoutMs: 600_000 });
  const version = await readCargoVersion();
  const powdBinaryPath = resolvePowdBinaryPath();
  const fixtureRoot = path.join(openClawRoot, "release-fixture");
  await createReleaseFixture({ version, platform, powdBinaryPath, rootDir: fixtureRoot });

  await withHttpServer(fixtureRoot, async (port) => {
    const pluginDir = path.join(repoRoot, "plugins", "openclaw-powd");
    process.stderr.write("[smoke] packing OpenClaw plugin archive\n");
    const packed = await runCommand("npm", ["pack", "--silent"], {
      cwd: pluginDir,
      timeoutMs: 120_000,
    });
    const pluginName = packed.stdout.trim().split(/\r?\n/).filter(Boolean).at(-1);
    if (!pluginName) {
      throw new Error("npm pack did not produce a plugin archive name");
    }
      const pluginPath = path.join(pluginDir, pluginName);

      try {
        process.stderr.write("[smoke] installing plugin into OpenClaw\n");
        await runOpenClaw(["plugins", "install", pluginPath], { timeoutMs: 120_000 });
        process.stderr.write("[smoke] restarting OpenClaw gateway\n");
        await runOpenClaw(["gateway", "restart"], { timeoutMs: 120_000 });

        process.stderr.write("[smoke] inspecting loaded plugin\n");
        const inspect = await runOpenClawJson(["plugins", "inspect", "powd", "--json"], {
          timeoutMs: 60_000,
        });
        assert.equal(inspect.plugin.id, "powd");

        process.stderr.write("[smoke] checking setup status before install\n");
        const before = await runOpenClawJson(["powd", "status", "--json"], { timeoutMs: 60_000 });
        assert.equal(before.installed, false);
        assert.equal(before.registered, false);

        const releaseBaseUrl = `http://127.0.0.1:${port}/releases/download`;
        const releaseApiBaseUrl = `http://127.0.0.1:${port}/api/releases`;
        process.stderr.write("[smoke] configuring local release fixture overrides\n");
        await runOpenClaw(
          [
            "config",
            "set",
            "plugins.entries.powd.config.releaseBaseUrl",
            JSON.stringify(releaseBaseUrl),
          ],
          { timeoutMs: 60_000 },
        );
        await runOpenClaw(
          [
            "config",
            "set",
            "plugins.entries.powd.config.releaseApiBaseUrl",
            JSON.stringify(releaseApiBaseUrl),
          ],
          { timeoutMs: 60_000 },
        );

        process.stderr.write("[smoke] installing powd through the plugin\n");
        const install = await runOpenClawJson(["powd", "install", "--json"], { timeoutMs: 120_000 });
        assert.equal(install.installed, true);
        assert.equal(install.registered, true);
        assert.equal(install.version, version);
        assert.equal(install.mcpCommandMatchesInstall, true);

        process.stderr.write("[smoke] verifying MCP registration\n");
        const saved = await runOpenClawJson(["mcp", "show", "powd", "--json"], { timeoutMs: 60_000 });
        assert.equal(saved.command, install.binaryPath);
        assert.deepEqual(saved.args, ["mcp", "serve"]);
        assert.deepEqual(saved.env, {});

        process.stderr.write("[smoke] listing powd MCP tools over stdio\n");
        const tools = await listMcpTools(saved);
        assert.equal(tools.length, 9);
        const toolNames = tools.map((tool) => tool.name).sort();
        assert.deepEqual(toolNames, [
          "miner_pause",
          "miner_resume",
          "miner_set_mode",
          "miner_start",
          "miner_status",
          "miner_stop",
          "wallet_reward",
          "wallet_set",
          "wallet_show",
        ]);
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
