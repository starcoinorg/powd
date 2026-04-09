import net from "node:net";
import path from "node:path";
import fs from "node:fs/promises";

function privateTmpDirName() {
  if (typeof process.getuid === "function") {
    return `powd-${process.getuid()}`;
  }
  return "powd";
}

export function defaultPowdSocketPath(env = process.env) {
  if (typeof env.XDG_RUNTIME_DIR === "string" && env.XDG_RUNTIME_DIR.trim()) {
    return path.join(env.XDG_RUNTIME_DIR.trim(), "powd.sock");
  }
  if (typeof env.HOME === "string" && env.HOME.trim()) {
    return path.join(env.HOME.trim(), ".powd", "powd.sock");
  }
  return path.join("/tmp", privateTmpDirName(), "powd.sock");
}

async function pathExists(filePath) {
  try {
    await fs.access(filePath);
    return true;
  } catch {
    return false;
  }
}

function connectUnix(socketPath, timeoutMs) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(socketPath);
    const onError = (error) => {
      socket.destroy();
      reject(error);
    };
    socket.setTimeout(timeoutMs, () => onError(new Error("powd daemon socket timeout")));
    socket.once("connect", () => {
      socket.setTimeout(0);
      socket.off("error", onError);
      resolve(socket);
    });
    socket.once("error", onError);
  });
}

async function requestDaemonShutdown(socketPath, timeoutMs) {
  const socket = await connectUnix(socketPath, timeoutMs);
  try {
    const request = `${JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "daemon.shutdown",
    })}\n`;
    await new Promise((resolve, reject) => {
      socket.write(request, (error) => (error ? reject(error) : resolve()));
    });

    await new Promise((resolve, reject) => {
      let settled = false;
      let buffer = "";
      const finish = (handler) => (value) => {
        if (settled) {
          return;
        }
        settled = true;
        socket.destroy();
        handler(value);
      };
      const done = finish(resolve);
      const fail = finish(reject);

      socket.setTimeout(timeoutMs, () => fail(new Error("powd daemon shutdown response timed out")));
      socket.on("data", (chunk) => {
        buffer += chunk.toString("utf8");
        if (buffer.includes("\n")) {
          done();
        }
      });
      socket.once("error", fail);
      socket.once("end", done);
      socket.once("close", done);
    });
  } finally {
    socket.destroy();
  }
}

async function waitForSocketStop(socketPath, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!(await pathExists(socketPath))) {
      return true;
    }
    try {
      const socket = await connectUnix(socketPath, 200);
      socket.destroy();
    } catch {
      return true;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  return false;
}

export async function shutdownPowdDaemon({
  socketPath = defaultPowdSocketPath(),
  timeoutMs = 5000,
  logger,
} = {}) {
  if (!(await pathExists(socketPath))) {
    return { running: false, stopped: false, socketPath };
  }

  try {
    await requestDaemonShutdown(socketPath, timeoutMs);
  } catch (error) {
    if (logger?.warn) {
      logger.warn(`powd-plugin: unable to request daemon shutdown at ${socketPath}: ${error.message}`);
    }
  }

  const stopped = await waitForSocketStop(socketPath, timeoutMs);
  return {
    running: true,
    stopped,
    socketPath,
  };
}
