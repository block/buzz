import assert from "node:assert/strict";
import test from "node:test";

import { KIND_NIP43_LEAVE_REQUEST, leaveCommunity } from "./leaveCommunity.ts";

const signedEvent = {
  id: "event-id",
  pubkey: "a".repeat(64),
  created_at: 1,
  kind: KIND_NIP43_LEAVE_REQUEST,
  tags: [["-"]],
  content: "",
  sig: "b".repeat(128),
};

function dependencies(overrides = {}) {
  return {
    sign: async (input) => ({ ...signedEvent, ...input }),
    publishActive: async () => {},
    createRelayClient: () => ({
      publishEvent: async () => {},
      disconnect() {},
    }),
    ...overrides,
  };
}

test("signs the protected NIP-43 leave request and awaits active relay acceptance", async () => {
  let signInput;
  let published;
  await leaveCommunity(
    "wss://active.example",
    "wss://active.example",
    dependencies({
      sign: async (input) => {
        signInput = input;
        return signedEvent;
      },
      publishActive: async (event) => {
        published = event;
      },
      createRelayClient: () => {
        throw new Error("inactive client should not be created");
      },
    }),
  );

  assert.deepEqual(signInput, {
    kind: KIND_NIP43_LEAVE_REQUEST,
    content: "",
    tags: [["-"]],
  });
  assert.equal(published, signedEvent);
});

test("targets an inactive community relay and always disconnects", async () => {
  const calls = [];
  await leaveCommunity(
    "wss://inactive.example",
    "wss://active.example",
    dependencies({
      publishActive: async () => {
        throw new Error("active relay should not be used");
      },
      createRelayClient: (relayUrl) => ({
        publishEvent: async (event) => calls.push(["publish", relayUrl, event]),
        disconnect: () => calls.push(["disconnect"]),
      }),
    }),
  );

  assert.deepEqual(calls, [
    ["publish", "wss://inactive.example", signedEvent],
    ["disconnect"],
  ]);
});

test("preserves relay rejection and disconnects without falling through", async () => {
  const rejection = new Error("invalid: relay owner cannot leave");
  let disconnected = false;

  await assert.rejects(
    leaveCommunity(
      "wss://inactive.example",
      "wss://active.example",
      dependencies({
        createRelayClient: () => ({
          publishEvent: async () => {
            throw rejection;
          },
          disconnect: () => {
            disconnected = true;
          },
        }),
      }),
    ),
    rejection,
  );
  assert.equal(disconnected, true);
});

test("turns an inactive relay timeout into an actionable leave error", async () => {
  await assert.rejects(
    leaveCommunity(
      "wss://inactive.example",
      "wss://active.example",
      dependencies({
        createRelayClient: () => ({
          publishEvent: async () => {
            throw new Error("Timed out publishing to observer relay.");
          },
          disconnect() {},
        }),
      }),
    ),
    /Timed out while leaving the community\. Try again\./,
  );
});
