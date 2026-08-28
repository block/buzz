import assert from "node:assert/strict";
import test from "node:test";

import { createChannelLiveSubscriptionRegistry } from "./channelLiveSubscriptionRegistry.ts";

function createRig() {
  const subscriptions = new Map();
  const disposed = [];
  let subscribeCount = 0;
  const registry = createChannelLiveSubscriptionRegistry({
    onError: (_message, _channelId, error) => {
      throw error;
    },
    subscribe: async (channelId, onEvent) => {
      subscribeCount += 1;
      subscriptions.set(channelId, onEvent);
      return () => disposed.push(channelId);
    },
    subscribeToReconnects: () => () => {},
  });
  return {
    disposed,
    registry,
    subscriptions,
    subscribeCount: () => subscribeCount,
  };
}

const event = { id: "event" };

test("same-channel surfaces share one live subscription when the newer surface closes", async () => {
  const rig = createRig();
  const received = [];
  const releaseMain = rig.registry.acquire("bestie", {
    onEvent: () => received.push("main"),
    refresh: async () => {},
  });
  const releasePopover = rig.registry.acquire("bestie", {
    onEvent: () => received.push("popover"),
    refresh: async () => {},
  });
  await Promise.resolve();

  assert.equal(rig.subscribeCount(), 1);
  rig.subscriptions.get("bestie")(event);
  releasePopover();
  rig.subscriptions.get("bestie")(event);
  assert.deepEqual(received, ["popover", "main"]);
  assert.deepEqual(rig.disposed, []);

  releaseMain();
  assert.deepEqual(rig.disposed, ["bestie"]);
});

test("releasing the older owner leaves the newer same-channel owner active", async () => {
  const rig = createRig();
  const received = [];
  const releaseMain = rig.registry.acquire("bestie", {
    onEvent: () => received.push("main"),
    refresh: async () => {},
  });
  const releasePopover = rig.registry.acquire("bestie", {
    onEvent: () => received.push("popover"),
    refresh: async () => {},
  });
  await Promise.resolve();

  releaseMain();
  rig.subscriptions.get("bestie")(event);
  assert.deepEqual(received, ["popover"]);
  assert.deepEqual(rig.disposed, []);

  releasePopover();
  assert.deepEqual(rig.disposed, ["bestie"]);
});

test("the first owner receives events emitted before subscription setup settles", async () => {
  const received = [];
  const registry = createChannelLiveSubscriptionRegistry({
    onError: (_message, _channelId, error) => {
      throw error;
    },
    subscribe: async (_channelId, onEvent) => {
      onEvent(event);
      return () => {};
    },
    subscribeToReconnects: () => () => {},
  });

  registry.acquire("bestie", {
    onEvent: () => received.push("bestie"),
    refresh: async () => {},
  });
  await Promise.resolve();

  assert.deepEqual(received, ["bestie"]);
});
