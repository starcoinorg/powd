import fs from "node:fs/promises";
import path from "node:path";
import { createWriteStream } from "node:fs";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";

export async function downloadFile(url, destinationPath, { fetchImpl = fetch } = {}) {
  const response = await fetchImpl(url);
  if (!response.ok || !response.body) {
    throw new Error(`download failed (${response.status} ${response.statusText}) for ${url}`);
  }

  await fs.mkdir(path.dirname(destinationPath), { recursive: true });
  await pipeline(Readable.fromWeb(response.body), createWriteStream(destinationPath));
}
