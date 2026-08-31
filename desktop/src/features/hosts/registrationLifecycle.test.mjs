import assert from "node:assert/strict";
import test from "node:test";
import { createHostRegistrationLifecycle } from "./registrationLifecycle.ts";
import { fixture } from "./hostTestFixtures.mjs";

const flush = () => new Promise((resolve) => setImmediate(resolve));
function deferred() {
  let resolve;
  const promise = new Promise((r) => {
    resolve = r;
  });
  return { promise, resolve };
}
function lifecycle(f, extra = {}) {
  const snapshots = [];
  const errors = [];
  let connections = 0;
  let disconnections = 0;
  const controller = createHostRegistrationLifecycle({
    owner: "a".repeat(64),
    bridge: f.bridge,
    journal: f.journal,
    now: () => 100,
    connect: () => {
      connections++;
      return {
        ...f.relay,
        disconnect: () => {
          disconnections++;
        },
      };
    },
    checking: () => {},
    success: (s) => snapshots.push(s),
    failure: (e) => errors.push(e),
    ...extra,
  });
  return {
    ...controller,
    snapshots,
    errors,
    counts: () => ({ connections, disconnections }),
  };
}

test("focus/online/manual/timer bursts coalesce into a serialized read-before-write retry", async () => {
  const f = fixture();
  const gate = deferred();
  const local = f.bridge.local;
  let calls = 0;
  f.bridge.local = async () => {
    calls++;
    await gate.promise;
    return local();
  };
  const controller = lifecycle(f);
  const first = controller.refresh();
  await flush();
  for (let i = 0; i < 10; i++) assert.equal(controller.refresh(), first);
  assert.equal(calls, 1);
  gate.resolve();
  await first;
  assert.equal(calls, 2);
  assert.equal(f.writes.length, 2);
  assert.equal(controller.snapshots.length, 2);
  assert.deepEqual(controller.counts(), { connections: 2, disconnections: 2 });
  await controller.stop();
});

for (const operation of [
  "local",
  "registration",
  "report",
  "inspect",
  "decode",
]) {
  test(`unmount during ${operation} fences publication, snapshots, errors and queued refresh`, async () => {
    const f = fixture();
    const gate = deferred();
    const entered = deferred();
    const original = f.bridge[operation];
    f.bridge[operation] = async (...args) => {
      entered.resolve();
      await gate.promise;
      return original(...args);
    };
    const controller = lifecycle(f);
    const first = controller.refresh();
    await entered.promise;
    const before = f.writes.length;
    void controller.refresh();
    const stopped = controller.stop();
    gate.resolve();
    await Promise.all([first, stopped]);
    await controller.refresh();
    assert.equal(f.writes.length, before);
    assert.equal(controller.snapshots.length, 0);
    assert.equal(controller.errors.length, 0);
    assert.equal(controller.counts().connections, 1);
  });
}

test("identity effect replacement waits for previous native work to drain", async () => {
  const f = fixture();
  const gate = deferred();
  const local = f.bridge.local;
  f.bridge.local = async () => {
    await gate.promise;
    return local();
  };
  const old = lifecycle(f);
  void old.refresh();
  await flush();
  const next = lifecycle(fixture(), { after: old.stop() });
  const pending = next.refresh();
  await flush();
  assert.equal(next.counts().connections, 0);
  gate.resolve();
  await pending;
  assert.equal(f.writes.length, 0);
  assert.equal(old.snapshots.length, 0);
  assert.equal(next.snapshots.length, 1);
  await next.stop();
});

test("failed refresh preserves last confirmed snapshot and a later retry recovers without duplicates", async () => {
  const f = fixture();
  const fetch = f.relay.fetchEvents;
  let fail = false;
  f.relay.fetchEvents = async (...args) => {
    if (fail) throw new Error("offline");
    return fetch(...args);
  };
  const controller = lifecycle(f);
  await controller.refresh();
  fail = true;
  await controller.refresh();
  assert.equal(controller.snapshots.length, 1);
  assert.equal(controller.errors.length, 1);
  fail = false;
  await controller.refresh();
  assert.equal(controller.snapshots.length, 2);
  assert.equal(f.writes.length, 2);
  assert.equal(controller.counts().disconnections, 3);
  await controller.stop();
});

test("unmount during publication suppresses stale success and prevents the following report", async () => {
  const f = fixture();
  const gate = deferred();
  const entered = deferred();
  const publish = f.relay.publishEvent;
  f.relay.publishEvent = async (event) => {
    await publish(event);
    entered.resolve();
    await gate.promise;
  };
  const controller = lifecycle(f);
  const first = controller.refresh();
  await entered.promise;
  const stopped = controller.stop();
  gate.resolve();
  await Promise.all([first, stopped]);
  assert.equal(controller.snapshots.length, 0);
  assert.equal(controller.errors.length, 0);
  assert.equal(f.writes.length, 1);
  f.relay.publishEvent = publish;
  const next = lifecycle(f);
  await next.refresh();
  assert.equal(f.writes.length, 2);
  await next.stop();
});

test("refresh at the completion microtask boundary is not lost", async () => {
  const f = fixture();
  let successes = 0;
  const controller = lifecycle(f, {
    success: () => {
      if (++successes === 1)
        queueMicrotask(() => {
          void controller.refresh();
        });
    },
  });
  await controller.refresh();
  assert.equal(successes, 2);
  assert.equal(f.writes.length, 2);
  await controller.stop();
});
