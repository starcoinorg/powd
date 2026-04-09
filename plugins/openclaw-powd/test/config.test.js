import test from "node:test";
import assert from "node:assert/strict";
import {
  buildPowdServer,
  getPowdServer,
  isManagedPowdServer,
  normalizePowdPluginInstallSpec,
  upsertPowdPluginAllow,
  upsertPowdServer,
} from "../src/config.js";

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

test("upsertPowdPluginAllow appends powd without clobbering existing allow entries", () => {
  const cfg = upsertPowdPluginAllow({
    plugins: {
      allow: ["openai", "telegram"],
    },
  });

  assert.deepEqual(cfg.plugins.allow, ["openai", "telegram", "powd"]);
  assert.deepEqual(upsertPowdPluginAllow(cfg).plugins.allow, ["openai", "telegram", "powd"]);
});

test("normalizePowdPluginInstallSpec converts a pinned ClawHub spec into the unpinned tracking spec", () => {
  const original = {
    plugins: {
      installs: {
        powd: {
          source: "clawhub",
          spec: "clawhub:@starcoinorg/openclaw-powd@1.0.0-rc.7",
          version: "1.0.0-rc.7",
        },
      },
    },
  };

  const normalized = normalizePowdPluginInstallSpec(original);
  assert.equal(normalized.plugins.installs.powd.spec, "clawhub:@starcoinorg/openclaw-powd");
  assert.equal(normalized.plugins.installs.powd.version, "1.0.0-rc.7");
});
