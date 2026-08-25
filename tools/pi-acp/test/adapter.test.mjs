import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { attachJsonlReader, writeJsonl } from "../src/jsonl.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const adapterPath = path.join(here, "../src/pi-acp-rpc.mjs");
const fakePiPath = path.join(here, "fake-pi.mjs");
const buzzMeta = {
  buzz: {
    channelId: "4dcab690-a2ca-4a56-9e5d-d901d12f83c3",
    triggeringEventIds: ["a".repeat(64)],
    replyTo: "a".repeat(64),
  },
};

function startHarness(mode = "complete") {
  const child = spawn(process.execPath, [adapterPath], {
    cwd: here,
    env: {
      ...process.env,
      PI_ACP_PI_COMMAND: process.execPath,
      PI_ACP_PI_ARGS_JSON: JSON.stringify([fakePiPath]),
      FAKE_PI_MODE: mode,
    },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const messages = [];
  const waiters = [];
  let stderr = "";
  child.stderr.on("data", (chunk) => {
    stderr += chunk.toString();
  });
  attachJsonlReader(
    child.stdout,
    (message) => {
      messages.push(message);
      for (const waiter of [...waiters]) {
        if (waiter.predicate(message)) {
          waiters.splice(waiters.indexOf(waiter), 1);
          clearTimeout(waiter.timer);
          waiter.resolve(message);
        }
      }
    },
    (error) => {
      throw error;
    },
  );

  return {
    child,
    messages,
    send(message) {
      writeJsonl(child.stdin, message);
    },
    waitFor(predicate, timeoutMs = 2_000) {
      const existing = messages.find(predicate);
      if (existing) return Promise.resolve(existing);
      return new Promise((resolve, reject) => {
        const waiter = { predicate, resolve };
        waiter.timer = setTimeout(() => {
          waiters.splice(waiters.indexOf(waiter), 1);
          reject(
            new Error(
              `timed out waiting for adapter message; stderr=${stderr}`,
            ),
          );
        }, timeoutMs);
        waiters.push(waiter);
      });
    },
    async close() {
      child.stdin.end();
      await Promise.race([
        once(child, "exit"),
        new Promise((resolve) => setTimeout(resolve, 1_000)),
      ]);
      if (child.exitCode === null) child.kill("SIGKILL");
    },
  };
}

async function handshake(harness) {
  harness.send({
    jsonrpc: "2.0",
    id: 1,
    method: "initialize",
    params: { protocolVersion: 2, clientCapabilities: {} },
  });
  const initialized = await harness.waitFor((message) => message.id === 1);
  assert.equal(initialized.result.protocolVersion, 2);
  assert.equal(initialized.result._meta.steering.supported, true);
  assert.equal(initialized.result._meta.pilot.liveCanaryValidated, true);
  assert.equal(initialized.result._meta.pilot.fleetApproved, false);

  harness.send({
    jsonrpc: "2.0",
    id: 2,
    method: "session/new",
    params: { cwd: here, mcpServers: [], systemPrompt: "Answer concisely." },
  });
  const created = await harness.waitFor((message) => message.id === 2);
  assert.match(created.result.sessionId, /^pi_/);
  return created.result.sessionId;
}

test("maps ACP prompt to Pi text, tool, usage, and completion events", async (t) => {
  const harness = startHarness();
  t.after(() => harness.close());
  const sessionId = await handshake(harness);

  harness.send({
    jsonrpc: "2.0",
    id: 3,
    method: "session/prompt",
    params: {
      sessionId,
      prompt: [{ type: "text", text: "status?" }],
      _meta: buzzMeta,
    },
  });
  const completed = await harness.waitFor((message) => message.id === 3);
  assert.equal(completed.result.stopReason, "end_turn");

  const text = harness.messages.find(
    (message) =>
      message.params?.update?.sessionUpdate === "agent_message_chunk",
  );
  assert.equal(text.params.update.content.text, "hello\u2028world");
  assert.ok(
    harness.messages.some(
      (message) => message.params?.update?.sessionUpdate === "tool_call",
    ),
  );
  assert.ok(
    harness.messages.some(
      (message) =>
        message.params?.update?.sessionUpdate === "tool_call_update" &&
        message.params.update.status === "completed",
    ),
  );
  const usage = harness.messages.find(
    (message) =>
      message.method === "_goose/unstable/session/update" &&
      message.params?.update?.sessionUpdate === "usage_update",
  );
  assert.equal(usage.params.update.accumulatedInputTokens, 185);
  assert.equal(usage.params.update.accumulatedOutputTokens, 20);
  assert.equal(usage.params.update.accumulatedCachedInputTokens, 80);
  assert.equal(usage.params.update.accumulatedCacheWriteTokens, 5);
  assert.equal(usage.params.update.accumulatedTotalTokens, 205);
  assert.equal(usage.params.update.model, "fake/test-model");
});

test("steers an active Pi prompt without starting a second ACP prompt", async (t) => {
  const harness = startHarness("steer");
  t.after(() => harness.close());
  const sessionId = await handshake(harness);

  harness.send({
    jsonrpc: "2.0",
    id: 3,
    method: "session/prompt",
    params: {
      sessionId,
      prompt: [{ type: "text", text: "start" }],
      _meta: buzzMeta,
    },
  });
  await new Promise((resolve) => setTimeout(resolve, 30));
  harness.send({
    jsonrpc: "2.0",
    id: 4,
    method: "_session/steering",
    params: { sessionId, prompt: [{ type: "text", text: "focus" }] },
  });

  const steered = await harness.waitFor((message) => message.id === 4);
  assert.deepEqual(steered.result, { outcome: "injected" });
  const completed = await harness.waitFor((message) => message.id === 3);
  assert.equal(completed.result.stopReason, "end_turn");
  assert.ok(
    harness.messages.some(
      (message) => message.params?.update?.content?.text === "steered:focus",
    ),
  );
});

test("maps ACP cancellation to Pi abort and drains a cancelled prompt", async (t) => {
  const harness = startHarness("cancel");
  t.after(() => harness.close());
  const sessionId = await handshake(harness);

  harness.send({
    jsonrpc: "2.0",
    id: 3,
    method: "session/prompt",
    params: {
      sessionId,
      prompt: [{ type: "text", text: "wait" }],
      _meta: buzzMeta,
    },
  });
  await new Promise((resolve) => setTimeout(resolve, 30));
  harness.send({
    jsonrpc: "2.0",
    method: "session/cancel",
    params: { sessionId },
  });

  const completed = await harness.waitFor((message) => message.id === 3);
  assert.equal(completed.result.stopReason, "cancelled");
});

test("rejects blank prompts and a second task session", async (t) => {
  const harness = startHarness();
  t.after(() => harness.close());
  const sessionId = await handshake(harness);

  harness.send({
    jsonrpc: "2.0",
    id: 3,
    method: "session/prompt",
    params: {
      sessionId,
      prompt: [{ type: "text", text: "   " }],
      _meta: buzzMeta,
    },
  });
  const blank = await harness.waitFor((message) => message.id === 3);
  assert.equal(blank.error.code, -32602);

  harness.send({
    jsonrpc: "2.0",
    id: 4,
    method: "session/new",
    params: { cwd: here, mcpServers: [] },
  });
  const duplicate = await harness.waitFor((message) => message.id === 4);
  assert.equal(duplicate.error.code, -32602);
});
