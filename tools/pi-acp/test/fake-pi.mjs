#!/usr/bin/env node

import { attachJsonlReader, writeJsonl } from "../src/jsonl.mjs";

let active = false;
let timer;
const mode = process.env.FAKE_PI_MODE || "complete";

function respond(command, data) {
  writeJsonl(process.stdout, {
    id: command.id,
    type: "response",
    command: command.type,
    success: true,
    ...(data === undefined ? {} : { data }),
  });
}

function complete(text = "hello\u2028world") {
  if (!active) return;
  active = false;
  writeJsonl(process.stdout, {
    type: "message_update",
    usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0 },
    assistantMessageEvent: { type: "text_delta", contentIndex: 0, delta: text },
  });
  writeJsonl(process.stdout, {
    type: "tool_execution_start",
    toolCallId: "tool-1",
    toolName: "read",
    args: { path: "README.md" },
  });
  writeJsonl(process.stdout, {
    type: "tool_execution_end",
    toolCallId: "tool-1",
    toolName: "read",
    result: { content: [{ type: "text", text: "ok" }] },
    isError: false,
  });
  writeJsonl(process.stdout, {
    type: "turn_end",
    message: {
      role: "assistant",
      provider: "fake",
      model: "test-model",
      stopReason: "stop",
      usage: {
        input: 100,
        output: 20,
        cacheRead: 80,
        cacheWrite: 5,
        totalTokens: 205,
        cost: { total: 0.01 },
      },
    },
    toolResults: [],
  });
  writeJsonl(process.stdout, { type: "agent_settled" });
}

attachJsonlReader(
  process.stdin,
  (command) => {
    switch (command.type) {
      case "get_state":
        respond(command, {
          model: { provider: "fake", id: "test-model" },
          isStreaming: false,
          sessionId: "fake-session",
        });
        break;
      case "prompt":
        active = true;
        respond(command);
        if (mode === "complete") timer = setTimeout(() => complete(), 10);
        else if (mode === "steer")
          timer = setTimeout(() => complete("unsteered"), 500);
        break;
      case "steer":
        clearTimeout(timer);
        respond(command);
        setTimeout(() => complete(`steered:${command.message}`), 5);
        break;
      case "abort":
        clearTimeout(timer);
        respond(command);
        if (active) {
          active = false;
          writeJsonl(process.stdout, {
            type: "turn_end",
            message: {
              role: "assistant",
              provider: "fake",
              model: "test-model",
              stopReason: "aborted",
              usage: {
                input: 10,
                output: 0,
                cacheRead: 0,
                cacheWrite: 0,
                totalTokens: 10,
                cost: { total: 0 },
              },
            },
            toolResults: [],
          });
          writeJsonl(process.stdout, { type: "agent_settled" });
        }
        break;
      default:
        writeJsonl(process.stdout, {
          id: command.id,
          type: "response",
          command: command.type,
          success: false,
          error: "unsupported fake command",
        });
    }
  },
  (error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  },
);
