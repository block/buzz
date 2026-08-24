#!/usr/bin/env node

import { spawn } from "node:child_process";
import { performance } from "node:perf_hooks";
import { attachJsonlReader, writeJsonl } from "../src/jsonl.mjs";

const command = process.argv[2];
if (!command)
  throw new Error("usage: benchmark-acp.mjs <adapter-command> [args-json]");
const args = process.argv[3] ? JSON.parse(process.argv[3]) : [];
const child = spawn(command, args, {
  cwd: process.cwd(),
  env: process.env,
  stdio: ["pipe", "pipe", "pipe"],
});
let nextId = 0;
let stderr = "";
const pending = new Map();
const events = [];
child.stderr.on("data", (chunk) => {
  stderr += chunk.toString();
});
attachJsonlReader(
  child.stdout,
  (message) => {
    if (message.id !== undefined && (message.result || message.error)) {
      const waiter = pending.get(String(message.id));
      if (waiter) {
        pending.delete(String(message.id));
        if (message.error) waiter.reject(new Error(message.error.message));
        else waiter.resolve(message.result);
      }
      return;
    }
    events.push(message);
    if (message.method === "session/request_permission") {
      const option = message.params?.options?.find(
        (item) => item.kind === "reject_once",
      );
      writeJsonl(child.stdin, {
        jsonrpc: "2.0",
        id: message.id,
        result: option
          ? { outcome: { outcome: "selected", optionId: option.optionId } }
          : { outcome: { outcome: "cancelled" } },
      });
    }
  },
  (error) => {
    process.stderr.write(`${error.message}\n`);
  },
);

function request(method, params, timeoutMs = 30_000) {
  const id = String(++nextId);
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`${method} timed out; stderr=${stderr}`));
    }, timeoutMs);
    pending.set(id, {
      resolve: (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      reject: (error) => {
        clearTimeout(timer);
        reject(error);
      },
    });
    writeJsonl(child.stdin, { jsonrpc: "2.0", id, method, params });
  });
}

const started = performance.now();
try {
  const initialized = await request("initialize", {
    protocolVersion: 2,
    clientCapabilities: { auth: { terminal: true } },
    clientInfo: { name: "pi-acp-benchmark", version: "0.1.0" },
  });
  const created = await request(
    "session/new",
    {
      cwd: process.cwd(),
      mcpServers: [],
      systemPrompt:
        "Benchmark mode. Do not call tools. Return exactly the requested text and nothing else.",
    },
    60_000,
  );
  const eventId = "a".repeat(64);
  const result = await request(
    "session/prompt",
    {
      sessionId: created.sessionId,
      prompt: [{ type: "text", text: "Return exactly: PI_ACP_CANARY_OK" }],
      _meta: {
        buzz: {
          channelId: "4dcab690-a2ca-4a56-9e5d-d901d12f83c3",
          triggeringEventIds: [eventId],
          replyTo: eventId,
        },
      },
    },
    120_000,
  );
  const text = events
    .filter(
      (event) => event.params?.update?.sessionUpdate === "agent_message_chunk",
    )
    .map((event) => event.params.update.content?.text || "")
    .join("");
  const tools = events.filter(
    (event) => event.params?.update?.sessionUpdate === "tool_call",
  ).length;
  const usageEvent = events.findLast(
    (event) =>
      event.method === "_goose/unstable/session/update" &&
      event.params?.update?.sessionUpdate === "usage_update",
  );
  process.stdout.write(
    `${JSON.stringify(
      {
        adapter: initialized.agentInfo?.name || command,
        elapsedMs: Math.round(performance.now() - started),
        stopReason: result.stopReason,
        text,
        exact: text.trim() === "PI_ACP_CANARY_OK",
        tools,
        usage: usageEvent?.params?.update || result.usage || null,
      },
      null,
      2,
    )}\n`,
  );
} finally {
  child.stdin.end();
  setTimeout(() => child.kill("SIGKILL"), 500).unref();
}
