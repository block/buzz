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
    requiresMembership: async () => true,
    sign: async (input) => ({ ...signedEvent, ...input }),
    publishActive: async () => {},
    createRelayClient: () => ({
      publishEvent: async () => {},
      disconnect() {},
    }),
    ...overrides,
  };
}

test("skips relay publishing when the relay does not enforce membership", async () => {
  let checkedRelay;
  await leaveCommunity(
    "wss://open.example",
    "wss://open.example",
    dependencies({
      requiresMembership: async (relayUrl) => {
        checkedRelay = relayUrl;
        return false;
      },
      sign: async () => {
        throw new Error("open relay leave should not be signed");
      },
      publishActive: async () => {
        throw new Error("open relay leave should not be published");
      },
    }),
  );

  assert.equal(checkedRelay, "wss://open.example");
});

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

test("treats an already-absent active membership as successful cleanup", async () => {
  const result = await leaveCommunity(
    "wss://active.example",
    "wss://active.example",
    dependencies({
      publishActive: async () => {
        throw new Error("invalid: you are not a relay member");
      },
    }),
  );

  assert.deepEqual(result, { status: "already-absent" });
});

test("treats an already-absent inactive membership as successful cleanup and disconnects", async () => {
  let disconnected = false;
  const result = await leaveCommunity(
    "wss://inactive.example",
    "wss://active.example",
    dependencies({
      createRelayClient: () => ({
        publishEvent: async () => {
          throw new Error("invalid: you are not a relay member");
        },
        disconnect: () => {
          disconnected = true;
        },
      }),
    }),
  );
  assert.deepEqual(result, { status: "already-absent" });
  assert.equal(disconnected, true);
});

test("preserves other relay rejections and disconnects without falling through", async () => {
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
