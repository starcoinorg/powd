import fs from "node:fs/promises";
import http from "node:http";
import https from "node:https";
import path from "node:path";
import { createWriteStream } from "node:fs";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";

const DEFAULT_HEADERS = {
  "user-agent": "openclaw-powd-plugin",
};

function resolveTransport(url) {
  const parsed = new URL(url);
  if (parsed.protocol === "https:") {
    return { parsed, request: https.request };
  }
  if (parsed.protocol === "http:") {
    return { parsed, request: http.request };
  }
  throw new Error(`unsupported URL protocol for ${url}`);
}

async function readStreamText(stream) {
  const chunks = [];
  for await (const chunk of stream) {
    chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  }
  return Buffer.concat(chunks).toString("utf8");
}

async function withResponse(url, { headers = {}, maxRedirects = 5 } = {}, handler) {
  if (maxRedirects < 0) {
    throw new Error(`too many redirects while requesting ${url}`);
  }

  const { parsed, request } = resolveTransport(url);
  const requestHeaders = {
    ...DEFAULT_HEADERS,
    ...headers,
  };

  return await new Promise((resolve, reject) => {
    const req = request(parsed, { agent: false, family: 4, headers: requestHeaders }, (response) => {
      const statusCode = response.statusCode ?? 0;
      if (statusCode >= 300 && statusCode < 400 && response.headers.location) {
        response.resume();
        const redirectUrl = new URL(response.headers.location, parsed).toString();
        resolve(withResponse(redirectUrl, { headers, maxRedirects: maxRedirects - 1 }, handler));
        return;
      }

      if (statusCode < 200 || statusCode >= 300) {
        readStreamText(response)
          .then((body) => {
            const reason = response.statusMessage ? ` ${response.statusMessage}` : "";
            const suffix = body.trim() ? `: ${body.trim()}` : "";
            reject(new Error(`request failed (${statusCode}${reason}) for ${url}${suffix}`));
          })
          .catch(reject);
        return;
      }

      Promise.resolve(handler(response)).then(resolve, reject);
    });

    req.setTimeout(30_000, () => {
      req.destroy(new Error(`request timed out for ${url}`));
    });
    req.once("error", reject);
    req.end();
  });
}

export async function requestJson(url, { headers = {}, fetchImpl } = {}) {
  if (fetchImpl) {
    const response = await fetchImpl(url, { headers: { ...DEFAULT_HEADERS, ...headers } });
    if (!response.ok) {
      throw new Error(`request failed (${response.status} ${response.statusText}) for ${url}`);
    }
    return await response.json();
  }

  return await withResponse(url, { headers }, async (response) => {
    const text = await readStreamText(response);
    return JSON.parse(text);
  });
}

export async function downloadFile(url, destinationPath, { fetchImpl } = {}) {
  await fs.mkdir(path.dirname(destinationPath), { recursive: true });
  const tempPath = `${destinationPath}.part-${process.pid}-${Date.now()}`;

  try {
    if (fetchImpl) {
      const response = await fetchImpl(url, { headers: DEFAULT_HEADERS });
      if (!response.ok || !response.body) {
        throw new Error(`download failed (${response.status} ${response.statusText}) for ${url}`);
      }
      await pipeline(Readable.fromWeb(response.body), createWriteStream(tempPath));
    } else {
      await withResponse(url, {}, async (response) => {
        await pipeline(response, createWriteStream(tempPath));
      });
    }

    await fs.rename(tempPath, destinationPath);
  } catch (error) {
    await fs.rm(tempPath, { force: true });
    throw error;
  }
}
