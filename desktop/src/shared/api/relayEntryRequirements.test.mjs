import assert from "node:assert/strict";
import { test } from "node:test";

let relay = "wss://a.example/";
let policy = { required: true, upgradeUrl: null };
let dials = 0;
let pending;
const calls = [];
globalThis.window = {
  setTimeout,
  clearTimeout,
  setInterval,
  clearInterval,
  __TAURI_INTERNALS__: {
    transformCallback: () => 1,
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command === "get_relay_ws_url") return relay;
      if (command === "fetch_relay_entry_requirements") {
        if (pending)
          return new Promise((resolve) => {
            pending.resolve = resolve;
          });
        if (policy instanceof Error) throw policy;
        return policy;
      }
      if (command === "plugin:websocket|connect") {
        dials++;
        throw new Error("test reached socket boundary");
      }
      if (command === "plugin:websocket|disconnect") return;
      assert.fail(`unexpected command ${command}`);
    },
  },
};
const { RelayClient } = await import("./relayClientSession.ts");

test("join/startup/reconnect share policy, retry and community-switch isolation", async () => {
  const client = new RelayClient();
  try {
    await assert.rejects(client.preconnect(), /newer Buzz/);
    assert.equal(client.entryRequirements.getSnapshot().verdict, "required");
    assert.equal(client.terminal, true);
    assert.equal(dials, 0);
    // Existing subscriptions/query callers cannot bypass the required verdict.
    await assert.rejects(client.fetchFirstEvent({ kinds: [0] }), /terminal/);
    assert.equal(dials, 0);
    // A failed retry is retryable, never a free or an upgrade verdict.
    policy = new Error("HTTP 503");
    await assert.rejects(client.preconnect(), /503/);
    assert.equal(client.entryRequirements.getSnapshot().verdict, "error");
    assert.equal(client.terminal, false);
    // Required -> false and absent (mapped by native) resume the existing transport.
    policy = { required: false, upgradeUrl: null };
    await assert.rejects(client.preconnect(), /socket boundary/);
    assert.equal(client.entryRequirements.getSnapshot().verdict, null);
    assert.equal(dials, 1);
    client.disconnect();
    // A delayed response from A cannot change B's policy or dial its socket.
    pending = {};
    const stale = client.preconnect();
    while (!pending.resolve) await new Promise(setImmediate);
    const resolveA = pending.resolve;
    pending = undefined;
    client.disconnect();
    relay = "wss://b.example/";
    policy = { required: true, upgradeUrl: "https://downloads.example/b" };
    await assert.rejects(client.preconnect(), /newer Buzz/);
    resolveA({ required: false, upgradeUrl: null });
    await assert.rejects(stale, /superseded/);
    assert.equal(client.entryRequirements.getSnapshot().relayUrl, relay);
    assert.equal(client.entryRequirements.getSnapshot().verdict, "required");
    assert.equal(dials, 1);
    assert.ok(
      calls.every(
        ([command]) => !/save|delete|reset_identity|remove/.test(command),
      ),
      "entry check does not mutate keys or saved communities",
    );
  } finally {
    client.disconnect();
  }
});
