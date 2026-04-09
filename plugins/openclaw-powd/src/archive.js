import fs from "node:fs/promises";
import path from "node:path";
import zlib from "node:zlib";

function readTarString(buffer, start, end) {
  const value = buffer.subarray(start, end).toString("utf8");
  const nul = value.indexOf("\0");
  return (nul === -1 ? value : value.slice(0, nul)).trim();
}

function readTarSize(buffer, start, end) {
  const raw = readTarString(buffer, start, end).replace(/\0/g, "").trim();
  if (!raw) {
    return 0;
  }
  return Number.parseInt(raw, 8);
}

export async function extractBinaryFromArchive({ archivePath, extractDir, binaryName, archiveName }) {
  const archiveBytes = await fs.readFile(archivePath);
  const tarBytes = zlib.gunzipSync(archiveBytes);
  let offset = 0;
  let extracted = false;

  while (offset + 512 <= tarBytes.length) {
    const header = tarBytes.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) {
      break;
    }

    const name = readTarString(header, 0, 100);
    const prefix = readTarString(header, 345, 500);
    const entryName = prefix ? `${prefix}/${name}` : name;
    const entryType = readTarString(header, 156, 157) || "0";
    const entrySize = readTarSize(header, 124, 136);
    const contentStart = offset + 512;
    const contentEnd = contentStart + entrySize;
    if (contentEnd > tarBytes.length) {
      throw new Error(`archive ${archiveName} is truncated`);
    }

    const normalizedEntryName = entryName.replace(/^\.\/+/, "");
    const isRegularFile = entryType === "0" || entryType === "";
    if (!extracted && isRegularFile && path.posix.basename(normalizedEntryName) === binaryName) {
      const destinationPath = path.join(extractDir, binaryName);
      await fs.mkdir(path.dirname(destinationPath), { recursive: true });
      await fs.writeFile(destinationPath, tarBytes.subarray(contentStart, contentEnd));
      extracted = true;
    }

    const alignedSize = Math.ceil(entrySize / 512) * 512;
    offset = contentStart + alignedSize;
  }

  if (!extracted) {
    throw new Error(`archive ${archiveName} does not contain ${binaryName}`);
  }
}
