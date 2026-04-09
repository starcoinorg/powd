import test from "node:test";
import assert from "node:assert/strict";
import { PLATFORM_MATRIX, resolvePlatform } from "../src/platform.js";

test("platform matrix keeps future expansion keys", () => {
  assert.deepEqual(Object.keys(PLATFORM_MATRIX).sort(), [
    "darwin:arm64",
    "darwin:x64",
    "linux:x64",
    "win32:x64",
  ]);
});

test("linux x64 is supported in v1", () => {
  assert.deepEqual(resolvePlatform("linux", "x64"), {
    key: "linux-x64",
    assetSuffix: "linux-x86_64",
    binaryName: "powd",
    supported: true,
  });
});

test("macOS Apple Silicon is supported", () => {
  assert.deepEqual(resolvePlatform("darwin", "arm64"), {
    key: "darwin-arm64",
    assetSuffix: "darwin-arm64",
    binaryName: "powd",
    supported: true,
  });
});

test("other future platforms are recognized but not yet supported", () => {
  assert.equal(resolvePlatform("darwin", "x64")?.supported, false);
  assert.equal(resolvePlatform("win32", "x64")?.supported, false);
});
