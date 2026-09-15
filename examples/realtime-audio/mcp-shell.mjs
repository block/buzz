#!/usr/bin/env node
// Isolated demo adapter: one advertised shell, executed by the existing dev MCP.
import { spawn } from "node:child_process";
const binary = process.env.FRANKIE_DEV_MCP_BIN;
if (!binary) {
  console.error("FRANKIE_DEV_MCP_BIN required");
  process.exit(1);
}
const child = spawn(binary, [], { stdio: ["pipe", "pipe", "inherit"] });
const listing = new Set(),
  calls = new Set();
let ended = false;
function stop(code) {
  if (ended) return;
  ended = true;
  for (const id of calls)
    child.stdin.write(
      `${JSON.stringify({
        jsonrpc: "2.0",
        method: "notifications/cancelled",
        params: { requestId: id, reason: "Voice demo closed" },
      })}\n`,
    );
  child.stdin.end();
  setTimeout(() => {
    child.kill("SIGKILL");
    process.exit(code);
  }, 1500).unref();
}
async function write(out, event) {
  if (!out.write(`${JSON.stringify(event)}\n`))
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        cleanup();
        reject(Error("MCP consumer stalled"));
      }, 2000);
      const cleanup = () => {
        clearTimeout(timer);
        out.off("drain", drain);
        out.off("error", error);
      };
      const drain = () => {
        cleanup();
        resolve();
      };
      const error = (e) => {
        cleanup();
        reject(e);
      };
      out.once("drain", drain);
      out.once("error", error);
    });
}
async function relay(input, output, transform) {
  let buffer = "";
  input.setEncoding("utf8");
  for await (const chunk of input) {
    buffer += chunk;
    if (buffer.length > 2 * 1024 * 1024) throw Error("MCP frame limit");
    for (;;) {
      const i = buffer.indexOf("\n");
      if (i < 0) break;
      const line = buffer.slice(0, i);
      buffer = buffer.slice(i + 1);
      if (line.trim()) {
        const event = await transform(JSON.parse(line));
        if (event) await write(output, event);
      }
    }
  }
  if (buffer.trim()) throw Error("truncated MCP frame");
}
relay(process.stdin, child.stdin, async (e) => {
  if (e.method === "tools/list") {
    if (listing.size >= 32) throw Error("MCP pending list limit");
    listing.add(e.id);
  }
  if (e.method === "tools/call" && e.params?.name !== "shell") {
    await write(process.stdout, {
      jsonrpc: "2.0",
      id: e.id,
      error: { code: -32602, message: "This voice demo exposes only shell" },
    });
    return null;
  }
  if (e.method === "tools/call") {
    if (calls.size >= 32) throw Error("MCP pending call limit");
    calls.add(e.id);
  }
  return e;
})
  .then(() => stop(0))
  .catch((e) => {
    console.error(e.message);
    stop(1);
  });
relay(child.stdout, process.stdout, async (e) => {
  calls.delete(e.id);
  if (listing.delete(e.id) && e.result) {
    const tool = e.result.tools?.find((t) => t.name === "shell");
    if (!tool) throw Error("dev MCP omitted shell");
    e.result = {
      tools: [
        {
          name: tool.name,
          inputSchema: {
            type: "object",
            properties: {
              command: {
                type: "string",
                description: "The bash command to execute.",
              },
            },
            required: ["command"],
            additionalProperties: false,
          },
          description:
            "Run a short shell command in the selected working directory and return stdout and stderr. Examples: uptime (system uptime), df -h / (disk usage), ls (list files). Use the exact command requested; keep commands read-only unless the user requests a change.",
        },
      ],
    };
  }
  return e;
}).catch((e) => {
  console.error(e.message);
  stop(1);
});
child.stdin.on("error", (e) => {
  console.error(e.message);
  stop(1);
});
child.on("error", (e) => {
  console.error(e.message);
  stop(1);
});
child.on("exit", (code) => {
  ended = true;
  process.exitCode = code ?? 1;
  process.stdin.destroy();
});
for (const signal of ["SIGTERM", "SIGINT"]) process.on(signal, () => stop(0));
