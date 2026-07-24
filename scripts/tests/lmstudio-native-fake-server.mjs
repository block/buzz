#!/usr/bin/env node

import fs from "node:fs";
import http from "node:http";

const [portFile, requestLog, payloadLog] = process.argv.slice(2);
if (!(portFile && requestLog && payloadLog)) {
  process.stderr.write("fake server requires port, request, and payload files\n");
  process.exit(64);
}

const mode = process.env.FAKE_LMSTUDIO_MODE ?? "valid";
const expectedToken = process.env.FAKE_LMSTUDIO_TOKEN ?? "test-token";
const chatVariant = process.env.FAKE_LMSTUDIO_CHAT_VARIANT ?? "message";

const loadedModel = {
  key: "qwen/test-model",
  type: "llm",
  display_name: "Qwen test model",
  max_context_length: 32768,
  loaded_instances: [
    {
      id: "qwen/test-model",
      config: { context_length: 32768 },
    },
  ],
  capabilities: {
    reasoning: {
      allowed_options: ["off", "on"],
      default: "on",
    },
    trained_for_tool_use: true,
  },
};

function catalogForMode() {
  switch (mode) {
    case "no-loaded":
      return {
        models: [
          {
            ...loadedModel,
            loaded_instances: [],
          },
        ],
      };
    case "valid":
    case "auth":
    case "chat":
      return {
        models: [
          loadedModel,
          {
            key: "embedding/test-model",
            type: "embedding",
            loaded_instances: [{ id: "embedding/test-model" }],
          },
        ],
      };
    default:
      return { models: [loadedModel] };
  }
}

function chatResponse() {
  const terminal = {
    type: "message",
    content: "PRIVATE_FAKE_RESPONSE_CONTENT",
  };
  const output =
    chatVariant === "pseudo-tool"
      ? [
          {
            type: "reasoning",
            content:
              '<tool_call>{"name":"memory_search","arguments":{"query":"PRIVATE_FAKE_PROMPT"}}</tool_call>',
          },
          terminal,
        ]
      : chatVariant === "tool-call"
        ? [
            {
              type: "tool_call",
              tool: "search",
              arguments: { query: "PRIVATE_FAKE_PROMPT" },
              output: "PRIVATE_FAKE_TOOL_OUTPUT",
              provider_info: {
                type: "ephemeral_mcp",
                server_label: "memory",
              },
            },
            terminal,
          ]
        : [terminal];

  return {
    model_instance_id:
      chatVariant === "parallel-instance"
        ? "qwen/test-model:2"
        : "qwen/test-model",
    output,
    stats: {
      input_tokens: 7,
      total_output_tokens: 5,
      reasoning_output_tokens: chatVariant === "pseudo-tool" ? 2 : 0,
    },
    response_id: "resp_fake_1",
  };
}

const server = http.createServer((request, response) => {
  fs.appendFileSync(
    requestLog,
    `${request.method ?? ""}\t${request.url ?? ""}\t${request.headers.authorization ?? ""}\n`,
  );

  const chunks = [];
  let bytes = 0;
  request.on("data", (chunk) => {
    bytes += chunk.length;
    if (bytes <= 128 * 1024) {
      chunks.push(chunk);
    }
  });
  request.on("end", () => {
    fs.appendFileSync(payloadLog, Buffer.concat(chunks).toString("utf8"));
    fs.appendFileSync(payloadLog, "\n");

    if (
      mode === "auth" &&
      request.headers.authorization !== `Bearer ${expectedToken}`
    ) {
      response.writeHead(401, { "content-type": "application/json" });
      response.end('{"error":"PRIVATE_AUTH_BODY"}');
      return;
    }

    if (request.method === "GET" && request.url === "/api/v1/models") {
      if (mode === "malformed") {
        response.writeHead(200, { "content-type": "application/json" });
        response.end('{"models":"PRIVATE_MALFORMED"}');
        return;
      }
      if (mode === "oversize") {
        const body = `{"models":[],"padding":"${"x".repeat(2 * 1024 * 1024)}"}`;
        response.writeHead(200, {
          "content-type": "application/json",
          "content-length": Buffer.byteLength(body),
        });
        response.end(body);
        return;
      }
      response.writeHead(200, { "content-type": "application/json" });
      response.end(JSON.stringify(catalogForMode()));
      return;
    }

    if (request.method === "POST" && request.url === "/api/v1/chat") {
      response.writeHead(200, { "content-type": "application/json" });
      response.end(JSON.stringify(chatResponse()));
      return;
    }

    response.writeHead(404, { "content-type": "application/json" });
    response.end('{"error":"not found"}');
  });
});

server.listen(0, "127.0.0.1", () => {
  const address = server.address();
  if (!address || typeof address === "string") {
    process.stderr.write("fake server has no TCP address\n");
    process.exit(1);
  }
  fs.writeFileSync(portFile, String(address.port), { mode: 0o600 });
});

for (const signal of ["SIGTERM", "SIGINT"]) {
  process.on(signal, () => {
    server.close(() => process.exit(0));
  });
}
