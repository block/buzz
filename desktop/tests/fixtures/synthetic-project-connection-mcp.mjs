import readline from "node:readline";

if (process.env.PROJECT_CONNECTION_CANARY !== "test-only") {
  process.exit(2);
}

const lines = readline.createInterface({
  input: process.stdin,
  crlfDelay: Number.POSITIVE_INFINITY,
});

for await (const line of lines) {
  const request = JSON.parse(line);
  if (request.method === "initialize") {
    process.stdout.write(
      `${JSON.stringify({
        jsonrpc: "2.0",
        id: request.id,
        result: {
          protocolVersion: "2025-06-18",
          capabilities: { tools: {} },
          serverInfo: { name: "buzz-project-connection-test", version: "1" },
        },
      })}\n`,
    );
  }
  if (request.method === "tools/list") {
    process.stdout.write(
      `${JSON.stringify({
        jsonrpc: "2.0",
        id: request.id,
        result: {
          tools: [
            {
              name: "analytics_weekly",
              description: "Returns a synthetic weekly summary.",
              inputSchema: { type: "object", properties: {} },
            },
          ],
        },
      })}\n`,
    );
  }
}
