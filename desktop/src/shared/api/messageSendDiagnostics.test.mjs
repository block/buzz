import assert from "node:assert/strict";
import test from "node:test";

import { createMessageSendTrace } from "./messageSendDiagnostics.ts";

function installInvoke(handler) {
  const previousWindow = globalThis.window;
  globalThis.window = {
    ...(previousWindow ?? {}),
    __TAURI_INTERNALS__: { invoke: handler },
  };
  return () => {
    globalThis.window = previousWindow;
  };
}

async function flushDiagnosticWrites() {
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));
}

test("message send trace writes ordered stages without message content", async () => {
  const entries = [];
  const restore = installInvoke(async (command, args) => {
    assert.equal(command, "append_message_send_diagnostic");
    entries.push(args.entry);
  });
  try {
    const trace = createMessageSendTrace({
      channelId: "15fed9f9-a324-5e47-917c-6f33546539b1",
      transport: "websocket",
    });
    await trace.measure("connection_wait", async () => {});
    await trace.finish(async () => "sent", "ab".repeat(32));
    await flushDiagnosticWrites();

    assert.deepEqual(
      entries.map((entry) => entry.stage),
      [
        "client_send_started",
        "connection_wait_started",
        "connection_wait_finished",
        "client_send_finished",
      ],
    );
    assert.equal(entries.at(-1).outcome, "accepted");
    assert.ok(entries.every((entry) => !("content" in entry)));
  } finally {
    restore();
  }
});

test("diagnostic write failures never fail the send operation", async () => {
  let calls = 0;
  const restore = installInvoke(async () => {
    calls += 1;
    throw new Error("diagnostic disk unavailable");
  });
  try {
    const trace = createMessageSendTrace({
      channelId: "15fed9f9-a324-5e47-917c-6f33546539b1",
      transport: "websocket",
    });
    assert.equal(await trace.finish(async () => "sent"), "sent");
    await flushDiagnosticWrites();
    assert.equal(calls, 2);
  } finally {
    restore();
  }
});

test("message send trace classifies publish timeouts", async () => {
  const entries = [];
  const restore = installInvoke(async (_command, args) => {
    entries.push(args.entry);
  });
  try {
    const trace = createMessageSendTrace({
      channelId: "15fed9f9-a324-5e47-917c-6f33546539b1",
      transport: "http",
    });
    await assert.rejects(
      trace.finish(async () => {
        throw new Error("Timed out while sending the message.");
      }),
      /Timed out/,
    );
    await flushDiagnosticWrites();
    assert.equal(entries.at(-1).outcome, "timeout");
  } finally {
    restore();
  }
});

test("message send trace classifies Tauri string connection errors", async () => {
  const entries = [];
  const restore = installInvoke(async (_command, args) => {
    entries.push(args.entry);
  });
  try {
    const trace = createMessageSendTrace({
      channelId: "15fed9f9-a324-5e47-917c-6f33546539b1",
      transport: "http",
    });
    trace.finishFailure("relay unreachable: could not connect to relay");
    await flushDiagnosticWrites();
    assert.equal(entries.at(-1).outcome, "connection_error");
  } finally {
    restore();
  }
});
