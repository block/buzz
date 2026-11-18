import assert from "node:assert/strict";
import { test } from "node:test";

let relay = "wss://a.example/";
let policy = { required: true };
let dials = 0;
let auths = 0;
let pending;
let currentClient;
let offline = false;
let authAccepted = true;
const calls = [];
const deliver = (client, frame) =>
  client.handleWsMessage(
    { type: "Text", data: JSON.stringify(frame) },
    client.connectionGeneration,
  );
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
        if (offline) throw new Error("network offline");
        const client = currentClient;
        setImmediate(() => {
          void deliver(client, ["AUTH", "test-challenge"]);
        });
        return dials;
      }
      if (command === "create_auth_event") {
        return JSON.stringify({
          id: `auth-${dials}`,
          kind: 22242,
          tags: [
            ["challenge", args.challenge],
            ["relay", args.relayUrl],
          ],
          content: "",
        });
      }
      if (command === "plugin:websocket|send") {
        const frame = JSON.parse(args.message.data);
        if (frame[0] === "AUTH") {
          auths++;
          const client = currentClient;
          setImmediate(() => {
            void deliver(client, [
              "OK",
              frame[1].id,
              authAccepted,
              authAccepted ? "" : "restricted: not a relay member",
            ]);
          });
        }
        return;
      }
      if (command === "plugin:websocket|disconnect") return;
      assert.fail(`unexpected command ${command}`);
    },
  },
};
const { RelayClient } = await import("./relayClientSession.ts");
const drop = (client) =>
  client.handleWsMessage(
    { type: "Close", data: { code: 1006, reason: "network changed" } },
    client.connectionGeneration,
  );

test("failed discovery preserves authenticated legacy reconnect and required-host isolation", async () => {
  const client = new RelayClient();
  currentClient = client;
  try {
    await assert.rejects(client.preconnect(), /newer Buzz/);
    assert.equal(client.entryRequirements.getSnapshot().verdict, "required");
    assert.equal(client.terminal, true);
    assert.equal(dials, 0);
    await assert.rejects(client.fetchFirstEvent({ kinds: [0] }), /terminal/);
    assert.equal(dials, 0);
    // Only native socket, HTTP and signing boundaries are fake. Authentication,
    // connection state, reset, reconnect pacing and entry decisions stay real.
    for (const failure of [
      "HTTP 503",
      "Community discovery timed out",
      "Invalid community discovery document (HTML)",
      "Invalid community authentication requirement",
    ]) {
      policy = new Error(failure);
      await client.preconnect();
      assert.equal(client.getConnectionState(), "connected", failure);
      assert.equal(
        client.entryRequirements.getSnapshot().verdict,
        "error",
        "failure is unknown policy, never optional",
      );
      assert.equal(client.terminal, false);
      assert.equal(
        auths,
        dials,
        "successful native dial completed real AUTH/OK session path",
      );
      await drop(client);
    }
    // Offline retry remains an ordinary transport failure, and resumes on recovery.
    policy = new Error("Could not reach community discovery");
    offline = true;
    await assert.rejects(client.resumeReconnect(), /network offline/);
    assert.equal(client.entryRequirements.getSnapshot().verdict, "error");
    assert.equal(client.terminal, false);
    offline = false;
    await client.resumeReconnect();
    assert.equal(client.getConnectionState(), "connected");
    await drop(client);
    policy = { required: false };
    await client.resumeReconnect();
    assert.equal(client.entryRequirements.getSnapshot().verdict, null);
    client.disconnect();
    // Unknown policy does not bypass relay enforcement.
    policy = new Error("HTTP 503");
    authAccepted = false;
    await assert.rejects(client.preconnect(), /not a relay member/);
    assert.equal(client.terminal, true);
    authAccepted = true;
    client.disconnect();
    // Preserve legacy socket URLs without sending query/fragment data to HTTP discovery.
    for (const legacyUrl of [
      "wss://a.example/?route=free",
      "wss://a.example/#view",
    ]) {
      relay = legacyUrl;
      policy = new Error("Community discovery rejects query or fragment");
      await client.preconnect();
      assert.equal(client.getConnectionState(), "connected");
      assert.equal(client.entryRequirements.getSnapshot().verdict, "error");
      assert.equal(
        calls
          .filter(([command]) => command === "plugin:websocket|connect")
          .at(-1)[1].url,
        legacyUrl,
      );
      client.disconnect();
    }
    // Invalid addresses cannot use discovery fallback to send credentials.
    const beforeInvalid = dials;
    relay = "wss://user:secret@a.example/";
    await assert.rejects(client.preconnect(), /without credentials/);
    assert.equal(dials, beforeInvalid);
    client.disconnect();
    relay = "wss://a.example/";
    // A delayed result from A cannot replace B's required policy or dial A.
    const beforeStale = dials;
    pending = {};
    const stale = client.preconnect();
    while (!pending.resolve) await new Promise(setImmediate);
    const resolveA = pending.resolve;
    pending = undefined;
    client.disconnect();
    relay = "wss://b.example/";
    policy = { required: true };
    await assert.rejects(client.preconnect(), /newer Buzz/);
    resolveA({ required: false });
    await assert.rejects(stale, /superseded/);
    assert.equal(client.entryRequirements.getSnapshot().relayUrl, relay);
    assert.equal(client.entryRequirements.getSnapshot().verdict, "required");
    assert.equal(dials, beforeStale);
    assert.ok(
      calls.every(
        ([command]) => !/save|delete|reset_identity|remove/.test(command),
      ),
      "entry checks do not mutate keys or communities",
    );
  } finally {
    client.disconnect();
  }
});
