import test from "node:test";
import assert from "node:assert/strict";
import { buildReleaseSpec, parseSha256Text, resolveLatestStableVersion } from "../src/releases.js";

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
  const version = await resolveLatestStableVersion({
    apiBaseOverride: "https://example.com/api/releases",
    fetchImpl: async (url, options) => {
      assert.equal(url, "https://example.com/api/releases/latest");
      assert.equal(options.headers.accept, "application/vnd.github+json");
      return {
        ok: true,
        async json() {
          return { tag_name: "v1.2.3" };
        },
      };
    },
  });

  assert.equal(version, "1.2.3");
});
