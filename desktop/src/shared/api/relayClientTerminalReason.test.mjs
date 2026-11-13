// A terminal latch must keep the relay's rejection reason.
//
// History: a fresh identity on a members-only relay is rejected at AUTH with
// `restricted: not a relay member`, which latches the session terminal. The
// onboarding membership check ran afterwards and only saw
// "Relay session is terminal; cannot reconnect.", so it showed a generic
// "Server error" instead of the not-a-member page.
import assert from "node:assert/strict";
import test from "node:test";

globalThis.window = {
  setTimeout: () => 0,
  clearTimeout: () => {},
  __TAURI_INTERNALS__: { invoke: async () => {} },
};

const { RelayClient } = await import("./relayClientSession.ts");

test("terminal refusal carries the AUTH rejection reason", async () => {
  const client = new RelayClient();
  client.resetConnection(new Error("restricted: not a relay member"), {
    reconnect: false,
  });

  await assert.rejects(
    client.fetchFirstEvent({ kinds: [13534], limit: 1 }),
    /restricted: not a relay member/,
  );
});

test("a later terminal latch replaces the remembered reason", async () => {
  const client = new RelayClient();
  client.resetConnection(new Error("restricted: not a relay member"), {
    reconnect: false,
  });
  client.resetConnection(new Error("blocked: banned"), { reconnect: false });

  await assert.rejects(
    client.fetchFirstEvent({ kinds: [13534], limit: 1 }),
    (error) =>
      error.message.includes("blocked: banned") &&
      !error.message.includes("not a relay member"),
  );
});
