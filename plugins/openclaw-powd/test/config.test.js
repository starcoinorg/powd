import test from "node:test";
import assert from "node:assert/strict";
import { buildPowdServer, getPowdServer, isManagedPowdServer, upsertPowdServer } from "../src/config.js";

test("upsertPowdServer writes the expected OpenClaw config shape", () => {
  const cfg = upsertPowdServer({}, "/tmp/powd");
  assert.deepEqual(cfg, {
    mcp: {
      servers: {
        powd: {
          command: "/tmp/powd",
          args: ["mcp", "serve"],
          env: {},
        },
      },
    },
  });
});

test("getPowdServer returns null when no powd registration exists", () => {
  assert.equal(getPowdServer({}), null);
});

test("isManagedPowdServer recognizes the plugin-owned registration", () => {
  const server = buildPowdServer("/tmp/powd");
  assert.equal(isManagedPowdServer(server, "/tmp/powd"), true);
  assert.equal(
    isManagedPowdServer(
      {
        command: "/tmp/other",
        args: ["mcp", "serve"],
        env: {},
      },
      "/tmp/powd",
    ),
    false,
  );
});
