import test from "node:test";
import assert from "node:assert/strict";
import { buildReleaseSpec, parseSha256Text } from "../src/releases.js";

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
