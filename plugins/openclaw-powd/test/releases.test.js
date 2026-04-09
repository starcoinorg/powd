import test from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import { buildReleaseSpec, parseSha256Text, resolveLatestStableVersion } from "../src/releases.js";

async function withLatestReleaseServer(handler) {
  const server = http.createServer((req, res) => {
    const requestPath = new URL(req.url, "http://127.0.0.1").pathname;
    if (requestPath === "/api/releases/latest") {
      res.writeHead(302, { location: "/mirror/latest" });
      res.end();
      return;
    }
    if (requestPath === "/mirror/latest") {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ tag_name: "v1.2.3" }));
      return;
    }
    res.writeHead(404);
    res.end("not found");
  });

  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  const port = typeof address === "object" && address ? address.port : 0;

  try {
    await handler(`http://127.0.0.1:${port}/api/releases`);
  } finally {
    await new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())));
  }
}

test("buildReleaseSpec matches the published powd asset contract", () => {
  const spec = buildReleaseSpec({
    version: "0.1.0",
    platform: {
      assetSuffix: "linux-x86_64",
      binaryName: "powd",
    },
    baseUrlOverride: "https://example.com/releases/download",
  });

  assert.equal(spec.archiveName, "powd-v0.1.0-linux-x86_64.tar.gz");
  assert.equal(spec.sha256Name, "powd-v0.1.0-linux-x86_64.tar.gz.sha256");
  assert.equal(
    spec.archiveUrl,
    "https://example.com/releases/download/v0.1.0/powd-v0.1.0-linux-x86_64.tar.gz",
  );
});

test("buildReleaseSpec matches the macOS Apple Silicon asset contract", () => {
  const spec = buildReleaseSpec({
    version: "1.0.0-rc.1",
    platform: {
      assetSuffix: "darwin-arm64",
      binaryName: "powd",
    },
    baseUrlOverride: "https://example.com/releases/download",
  });

  assert.equal(spec.archiveName, "powd-v1.0.0-rc.1-darwin-arm64.tar.gz");
  assert.equal(spec.sha256Name, "powd-v1.0.0-rc.1-darwin-arm64.tar.gz.sha256");
  assert.equal(
    spec.archiveUrl,
    "https://example.com/releases/download/v1.0.0-rc.1/powd-v1.0.0-rc.1-darwin-arm64.tar.gz",
  );
});

test("buildReleaseSpec matches the Windows x64 asset contract", () => {
  const spec = buildReleaseSpec({
    version: "1.0.0",
    platform: {
      assetSuffix: "windows-x86_64",
      binaryName: "powd.exe",
    },
    baseUrlOverride: "https://example.com/releases/download",
  });

  assert.equal(spec.archiveName, "powd-v1.0.0-windows-x86_64.tar.gz");
  assert.equal(spec.sha256Name, "powd-v1.0.0-windows-x86_64.tar.gz.sha256");
  assert.equal(
    spec.archiveUrl,
    "https://example.com/releases/download/v1.0.0/powd-v1.0.0-windows-x86_64.tar.gz",
  );
});

test("parseSha256Text accepts both raw and filename-suffixed digests", () => {
  const digest = "a".repeat(64);
  assert.equal(parseSha256Text(`${digest}\n`), digest);
  assert.equal(parseSha256Text(`${digest}  powd-v0.1.0-linux-x86_64.tar.gz\n`), digest);
});

test("buildReleaseSpec accepts versions with a leading v", () => {
  const spec = buildReleaseSpec({
    version: "v1.0.0-rc.1",
    platform: {
      assetSuffix: "linux-x86_64",
      binaryName: "powd",
    },
    baseUrlOverride: "https://example.com/releases/download",
  });

  assert.equal(spec.version, "1.0.0-rc.1");
  assert.equal(spec.archiveName, "powd-v1.0.0-rc.1-linux-x86_64.tar.gz");
});

test("resolveLatestStableVersion returns the normalized tag name from the GitHub releases API", async () => {
  await withLatestReleaseServer(async (apiBase) => {
    const version = await resolveLatestStableVersion({
      apiBaseOverride: apiBase,
    });

    assert.equal(version, "1.2.3");
  });
});
