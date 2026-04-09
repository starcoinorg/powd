import http from "node:http";
import fs from "node:fs/promises";
import path from "node:path";

const [rootDir, portValue] = process.argv.slice(2);

if (!rootDir || !portValue) {
  console.error("usage: node scripts/httpd.mjs <root-dir> <port>");
  process.exit(1);
}

const server = http.createServer(async (req, res) => {
  const requestPath = new URL(req.url, "http://127.0.0.1").pathname;
  const filePath = path.join(rootDir, requestPath);
  try {
    const data = await fs.readFile(filePath);
    res.writeHead(200);
    res.end(data);
  } catch {
    res.writeHead(404);
    res.end("not found");
  }
});

server.listen(Number(portValue), "127.0.0.1", () => {
  console.log(`listening:${portValue}`);
});
