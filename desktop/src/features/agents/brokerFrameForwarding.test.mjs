import assert from "node:assert/strict";
import test from "node:test";

import {
  BROKER_REQUEST_TYPE,
  isBrokerRequestPayload,
} from "./brokerFrameForwarding.ts";

test("a broker request payload is recognized by its type discriminator", () => {
  assert.equal(
    isBrokerRequestPayload({
      type: BROKER_REQUEST_TYPE,
      protocolVersion: 1,
      requestId: "req-1",
    }),
    true,
  );
});

test("an ill-formed broker request is still forwarded", () => {
  // Validation belongs to the host: a request the renderer judged invalid and
  // dropped would leave the requester waiting with no answer at all.
  assert.equal(isBrokerRequestPayload({ type: BROKER_REQUEST_TYPE }), true);
});

test("agent telemetry is not forwarded", () => {
  for (const payload of [
    { kind: "turn_started", seq: 3 },
    { type: "agent_management_request", action: "create" },
    { type: "broker_result", requestId: "req-1" },
    null,
    undefined,
    "broker_request",
    42,
  ]) {
    assert.equal(
      isBrokerRequestPayload(payload),
      false,
      `unexpectedly treated as a broker request: ${JSON.stringify(payload)}`,
    );
  }
});

// ── Forwarding: signed-event payload and fault isolation ──────────────────────
//
// The Tauri IPC bridge is stubbed at globalThis.__TAURI_INTERNALS__.invoke, the
// same pattern acpRuntimesQuery.test.mjs uses, so the command name and payload
// are observed directly.

const SIGNED_FRAME = {
  id: "ffff",
  pubkey: "aaaa",
  created_at: 1,
  kind: 24200,
  tags: [["frame", "telemetry"]],
  content: "nip44-ciphertext",
  sig: "bbbb",
};

function stubInvoke(handler) {
  // @tauri-apps/api reads window.__TAURI_INTERNALS__, so the shim needs a
  // window; jsdom is not loaded for this plain-helper test.
  const hadWindow = typeof globalThis.window !== "undefined";
  if (!hadWindow) {
    globalThis.window = globalThis;
  }
  const previous = globalThis.window.__TAURI_INTERNALS__;
  const calls = [];
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke: async (command, payload) => {
      calls.push({ command, payload });
      return handler(command, payload);
    },
  };
  return {
    calls,
    restore() {
      globalThis.window.__TAURI_INTERNALS__ = previous;
      if (!hadWindow) {
        delete globalThis.window;
      }
    },
  };
}

test("forwarding hands the host the signed event, not the plaintext", async () => {
  const stub = stubInvoke(() => ({
    brokerRequest: true,
    requestId: "req-1",
    resultDelivered: true,
  }));
  try {
    const { forwardBrokerFrame } = await import("./brokerFrameForwarding.ts");
    const handled = await forwardBrokerFrame(SIGNED_FRAME);

    assert.equal(stub.calls.length, 1);
    assert.equal(stub.calls[0].command, "broker_handle_observer_frame");
    // The host must re-verify and re-decrypt from the bytes the requester
    // signed; a decrypted payload would make the renderer the authority on what
    // the request says.
    assert.deepEqual(JSON.parse(stub.calls[0].payload.eventJson), SIGNED_FRAME);
    assert.equal(handled.requestId, "req-1");
  } finally {
    stub.restore();
  }
});

test("a broker host fault does not escape to the observer subscription", async () => {
  // Regression: forwarding used to run inside the observer decrypt try/catch, so
  // a broker fault surfaced as "Observer event decrypt failed" and tore down the
  // telemetry subscription for every agent over one failed mutation.
  const stub = stubInvoke(() => {
    throw new Error("owner keys unavailable");
  });
  const consoleError = console.error;
  console.error = () => {};
  try {
    const { forwardBrokerFrameIsolated } = await import(
      "./brokerFrameForwarding.ts"
    );
    assert.equal(await forwardBrokerFrameIsolated(SIGNED_FRAME), null);
  } finally {
    console.error = consoleError;
    stub.restore();
  }
});
