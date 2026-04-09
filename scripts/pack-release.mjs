import crypto from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

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

function sha256(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

export async function packReleaseArchive({
  binaryPath,
  version,
  assetSuffix = "linux-x86_64",
  outputDir,
}) {
  if (!binaryPath || !version || !outputDir) {
    throw new Error("binaryPath, version, and outputDir are required");
  }

  await fs.access(binaryPath);
  await fs.mkdir(outputDir, { recursive: true });

  const binaryName = path.basename(binaryPath);
  const assetBase = `powd-v${version}-${assetSuffix}`;
  const archivePath = path.join(outputDir, `${assetBase}.tar.gz`);
  const shaPath = `${archivePath}.sha256`;
  const binaryBytes = await fs.readFile(binaryPath);
  const header = createTarHeader(binaryName, binaryBytes.length);
  const remainder = binaryBytes.length % 512;
  const padding = remainder === 0 ? Buffer.alloc(0) : Buffer.alloc(512 - remainder, 0);
  const tarBuffer = Buffer.concat([header, binaryBytes, padding, Buffer.alloc(1024, 0)]);
  const archiveBytes = zlib.gzipSync(tarBuffer);
  await fs.writeFile(archivePath, archiveBytes);
  await fs.writeFile(shaPath, `${sha256(archiveBytes)}  ${path.basename(archivePath)}\n`, "utf8");
  return archivePath;
}

async function main() {
  const args = process.argv.slice(2);
  if (args.length !== 3 && args.length !== 4) {
    console.error("usage: scripts/pack-release.mjs <powd-binary> <version> [asset-suffix] <output-dir>");
    process.exit(1);
  }

  const [binaryPath, version, third, fourth] = args;
  const assetSuffix = args.length === 3 ? "linux-x86_64" : third;
  const outputDir = args.length === 3 ? third : fourth;
  const archivePath = await packReleaseArchive({
    binaryPath,
    version,
    assetSuffix,
    outputDir,
  });
  process.stdout.write(`${archivePath}\n`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.stack ?? error.message : String(error));
    process.exitCode = 1;
  });
}
