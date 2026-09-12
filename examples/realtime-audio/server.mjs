// Loopback ACP presentation adapter; provider credentials never reach the browser.
import http from "node:http";
import { spawn } from "node:child_process";
import { randomBytes, timingSafeEqual } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const directory = path.dirname(fileURLToPath(import.meta.url));
const port = Number(process.env.PORT || 18796);
const lifetimeHours = Number(process.env.FRANKIE_LIFETIME_HOURS || 24);
if (!Number.isInteger(port) || port < 1 || port > 65535)
  throw Error("invalid port");
if (
  !Number.isFinite(lifetimeHours) ||
  lifetimeHours <= 0 ||
  lifetimeHours > 168
)
  throw Error("lifetime must be 0..168 hours");
const origin = `http://127.0.0.1:${port}`;
const token = randomBytes(24).toString("hex");
const agentPath =
  process.env.BUZZ_AGENT_BIN ||
  path.resolve(directory, "../../target/debug/buzz-agent");
const mcpPath = process.env.BUZZ_MCP_BIN;
let agent,
  consumer,
  stopping = false;
const methods = new Set([
  "initialize",
  "session/new",
  "session/prompt",
  "session/cancel",
  ...["append", "playback", "interrupt", "close"].map(
    (x) => `_buzz/unstable/realtime/${x}`,
  ),
]);

const closing = new WeakSet();
function stopAgent(child) {
  if (
    closing.has(child) ||
    child.exitCode !== null ||
    child.signalCode !== null
  )
    return;
  closing.add(child);
  child.stdin.end();
  const timer = setTimeout(() => child.kill("SIGKILL"), 5000);
  timer.unref();
  child.once("exit", () => clearTimeout(timer));
}
function stop() {
  if (stopping) return;
  stopping = true;
  if (agent) stopAgent(agent);
  consumer?.end();
  server.close();
  setTimeout(() => {
    agent?.kill("SIGKILL");
    process.exit(process.exitCode || 0);
  }, 6000).unref();
}
async function write(stream, bytes) {
  if (stream.destroyed) throw Error("stream closed");
  if (stream.write(bytes)) return;
  await new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timer);
      stream.off("drain", drained);
      stream.off("close", closed);
      stream.off("error", closed);
    };
    const drained = () => {
      cleanup();
      resolve();
    };
    const closed = () => {
      cleanup();
      reject(Error("stream closed"));
    };
    const timer = setTimeout(() => {
      cleanup();
      reject(Error("consumer stalled"));
    }, 2000);
    stream.once("drain", drained);
    stream.once("close", closed);
    stream.once("error", closed);
  });
}
const server = http.createServer(async (req, res) => {
  res.setHeader("Cache-Control", "no-store");
  res.setHeader("X-Content-Type-Options", "nosniff");
  res.setHeader(
    "Content-Security-Policy",
    "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'",
  );
  if (
    req.headers.host !== `127.0.0.1:${port}` ||
    (req.headers.origin && req.headers.origin !== origin)
  ) {
    res.writeHead(403).end();
    return;
  }
  const pathname = new URL(req.url, origin).pathname;
  const staticFiles = {
    "/": ["index.html", "text/html"],
    "/capture-queue.mjs": ["capture-queue.mjs", "text/javascript"],
    "/voice.mjs": ["voice.mjs", "text/javascript"],
    "/ui.mjs": ["ui.mjs", "text/javascript"],
    "/client.mjs": ["client.mjs", "text/javascript"],
    "/audio-worklet.mjs": ["audio-worklet.mjs", "text/javascript"],
    "/style.css": ["style.css", "text/css"],
  };
  if (req.method === "GET" && staticFiles[pathname]) {
    const [name, type] = staticFiles[pathname];
    res.setHeader("Content-Type", type);
    res.end(readFileSync(path.join(directory, name)));
    return;
  }
  const supplied = Buffer.from(
    (req.headers.authorization || "").replace(/^Bearer /, ""),
  );
  if (
    supplied.length !== token.length ||
    !timingSafeEqual(supplied, Buffer.from(token))
  ) {
    res.writeHead(403).end();
    return;
  }
  if (pathname === "/events" && req.method === "GET") {
    if (agent || stopping) {
      res.writeHead(409).end();
      return;
    }
    const thinking =
      new URL(req.url, origin).searchParams.get("thinking") || "none";
    if (
      !["none", "minimal", "low", "medium", "high", "xhigh", "max"].includes(
        thinking,
      )
    ) {
      res.writeHead(400).end("invalid thinking level");
      return;
    }
    consumer = res;
    res.setHeader("Content-Type", "application/x-ndjson");
    res.flushHeaders();
    agent = spawn(agentPath, [], {
      env: { ...process.env, BUZZ_AGENT_THINKING_EFFORT: thinking },
      stdio: ["pipe", "pipe", "inherit"],
    });
    const child = agent;
    child.stdin.on("error", () => {
      stopAgent(child);
      res.end();
    });
    const finish = () => {
      if (agent === child) {
        agent = null;
        consumer = null;
      }
      res.end();
    };
    child.on("error", (error) => {
      console.error(error);
      finish();
    });
    child.on("exit", finish);
    res.on("close", () => stopAgent(child));
    (async () => {
      let buffer = "";
      child.stdout.setEncoding("utf8");
      for await (const chunk of child.stdout) {
        buffer += chunk;
        if (buffer.length > 2 * 1024 * 1024) throw Error("ACP frame limit");
        for (;;) {
          const index = buffer.indexOf("\n");
          if (index < 0) break;
          const line = buffer.slice(0, index);
          buffer = buffer.slice(index + 1);
          JSON.parse(line);
          await write(res, `${line}\n`);
        }
      }
    })().catch((error) => {
      console.error(error);
      stopAgent(child);
      res.end();
    });
    return;
  }
  if (pathname === "/rpc" && req.method === "POST" && agent) {
    try {
      let size = 0;
      const chunks = [];
      for await (const chunk of req) {
        size += chunk.length;
        if (size > 256 * 1024) throw Error("request limit");
        chunks.push(chunk);
      }
      const event = JSON.parse(Buffer.concat(chunks));
      if (
        event.jsonrpc !== "2.0" ||
        (event.method && !methods.has(event.method))
      )
        throw Error("method");
      if (event.method === "session/new") {
        event.params = {
          cwd: process.cwd(),
          mcpServers: mcpPath
            ? [
                {
                  name: "dev",
                  command: mcpPath,
                  args: [],
                  env: process.env.FRANKIE_DEV_MCP_BIN
                    ? [
                        {
                          name: "FRANKIE_DEV_MCP_BIN",
                          value: process.env.FRANKIE_DEV_MCP_BIN,
                        },
                      ]
                    : [],
                },
              ]
            : [],
        };
      }
      await write(agent.stdin, `${JSON.stringify(event)}\n`);
      res.writeHead(204).end();
    } catch {
      res.writeHead(400).end("invalid ACP request");
    }
    return;
  }
  res.writeHead(404).end();
});
server.requestTimeout = 10000;
server.headersTimeout = 10000;
server.maxConnections = 8;
process.on("SIGTERM", stop);
process.on("SIGINT", stop);
setTimeout(stop, lifetimeHours * 60 * 60 * 1000).unref();
server.on("error", (error) => {
  console.error(error.message);
  process.exitCode = 1;
  stop();
});
server.listen(port, "127.0.0.1", () => console.log(`${origin}/#${token}`));
