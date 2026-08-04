import assert from "node:assert/strict";
import test from "node:test";

import { subscribeToTrayActions } from "./trayActionConsumer.ts";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function createHarness(initialActions = []) {
  const queue = [...initialActions];
  const handled = [];
  const requeued = [];
  const availableListeners = new Set();
  const focusListeners = new Set();
  let takePendingActions = async () => queue.splice(0);

  const dispose = subscribeToTrayActions({
    addFocusListener(listener) {
      focusListeners.add(listener);
      return () => focusListeners.delete(listener);
    },
    async listenForAvailable(listener) {
      availableListeners.add(listener);
      return () => availableListeners.delete(listener);
    },
    takePendingActions: () => takePendingActions(),
    async requeueActions(actions) {
      requeued.push(...actions);
      queue.unshift(...actions);
    },
    handleAction(action) {
      handled.push(action);
    },
  });

  return {
    dispose,
    available: () => {
      for (const listener of availableListeners) listener();
    },
    focus: () => {
      for (const listener of focusListeners) listener();
    },
    handled,
    queue,
    requeued,
    setTakePendingActions: (take) => {
      takePendingActions = take;
    },
  };
}

test("drains an action queued before the consumer mounts", async () => {
  const action = { kind: "openChannel", channelId: "channel-before-mount" };
  const harness = createHarness([action]);

  await tick();

  assert.deepEqual(harness.handled, [action]);
  harness.dispose();
});

test("drains an action whose native event was lost while the webview was hidden", async () => {
  const harness = createHarness();
  await tick();

  const action = { kind: "openChannel", channelId: "channel-while-hidden" };
  harness.queue.push(action);
  harness.focus();
  await tick();

  assert.deepEqual(harness.handled, [action]);
  harness.dispose();
});

test("serializes overlapping wake-ups without stranding queued actions", async () => {
  const firstAction = { kind: "openChannel", channelId: "first-channel" };
  const secondAction = { kind: "openChannel", channelId: "second-channel" };
  const harness = createHarness([firstAction]);
  let releaseFirstTake;
  let markFirstTakeStarted;
  let takeCount = 0;
  const firstTakeStarted = new Promise((resolve) => {
    markFirstTakeStarted = resolve;
  });
  const firstTakeBlocked = new Promise((resolve) => {
    releaseFirstTake = resolve;
  });
  harness.setTakePendingActions(async () => {
    takeCount += 1;
    const actions = harness.queue.splice(0);
    if (takeCount === 1) {
      markFirstTakeStarted();
      await firstTakeBlocked;
    }
    return actions;
  });

  await firstTakeStarted;
  harness.queue.push(secondAction);
  harness.available();
  harness.focus();
  releaseFirstTake();
  await tick();

  assert.deepEqual(harness.handled, [firstAction, secondAction]);
  assert.equal(takeCount, 2);
  harness.dispose();
});

test("requeues actions drained while the consumer unmounts", async () => {
  const action = { kind: "openChannel", channelId: "channel-during-unmount" };
  const harness = createHarness();
  let releaseTake;
  const pendingTake = new Promise((resolve) => {
    releaseTake = resolve;
  });
  harness.setTakePendingActions(async () => {
    await pendingTake;
    return [action];
  });

  await tick();
  harness.dispose();
  releaseTake();
  await tick();

  assert.deepEqual(harness.handled, []);
  assert.deepEqual(harness.requeued, [action]);
});
