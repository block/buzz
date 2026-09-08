import assert from "node:assert/strict";
import test from "node:test";
import {
  prepareStop,
  readStopOutcome,
  receiveStops,
  sendStop,
} from "./desktopStop.ts";

const scope = { owner: "owner", community: "wss://one.example" };
const request = {
  id: "request",
  kind: 50180,
  pubkey: scope.owner,
  tags: [["d", "desktop"]],
};
const result = {
  id: "result",
  kind: 50181,
  pubkey: scope.owner,
  tags: [["e", request.id]],
};
const tick = () => new Promise((resolve) => setImmediate(resolve));
function fixture() {
  let epoch = 0;
  let live;
  let closed = false;
  let effect = 0;
  let outcome;
  let failResult = false;
  const saved = new Map();
  const stored = new Map();
  const publishes = [];
  const errors = [];
  const ipc = async (command, args) => {
    assert.equal(args.owner, scope.owner);
    assert.equal(args.community, scope.community);
    if (command === "prepare_desktop_stop") return request;
    if (command === "receive_desktop_stop") {
      if (!saved.has(args.event.id)) {
        effect++;
        saved.set(args.event.id, result);
      }
      return saved.get(args.event.id);
    }
    if (command === "read_desktop_stop_results") {
      assert.equal(args.request, request);
      return args.events.includes(result) ? "stopped" : "unknown";
    }
    throw Error(command);
  };
  const relay = {
    getSessionEpoch: () => epoch,
    publishEvent: async (event, _timeout, _failure, check) => {
      check();
      publishes.push(event);
      if (event.kind === 50180) {
        stored.set(event.id, event);
        // Real relay contract: same immutable Stop is explicitly redelivered.
        live?.(event);
      } else {
        if (failResult) throw Error("lost result publish");
        outcome = event;
      }
    },
    fetchEvents: async (filter) => {
      assert.deepEqual(filter, {
        kinds: [50181],
        authors: [scope.owner],
        "#e": [request.id],
        limit: 16,
      });
      return outcome ? [outcome] : [];
    },
    subscribeLive: async (filter, onEvent, ready) => {
      assert.deepEqual(filter, {
        kinds: [50180],
        authors: [scope.owner],
        limit: 0,
      });
      live = onEvent;
      ready("eose");
      return () => {
        live = undefined;
        closed = true;
      };
    },
  };
  return {
    ipc,
    relay,
    publishes,
    errors,
    stored,
    saved,
    effect: () => effect,
    closed: () => closed,
    deliver: () => live?.(request),
    switchScope: () => {
      epoch++;
    },
    failResult: (value) => {
      failResult = value;
    },
  };
}

test("lost delivery/result recovers only on explicit exact-byte retry, not history replay", async () => {
  const f = fixture();
  const prepared = await prepareStop(
    scope,
    "desktop",
    "agent",
    () => true,
    f.ipc,
    f.relay,
  );
  await sendStop(scope, prepared, () => true, f.relay); // target absent
  assert.equal(f.effect(), 0);
  const close = await receiveStops(
    scope,
    () => true,
    (e) => f.errors.push(e),
    f.ipc,
    f.relay,
  );
  assert.equal(
    f.effect(),
    0,
    "opening receiver cannot dispatch stored requests",
  );
  assert.equal(
    await readStopOutcome(scope, request, () => true, f.ipc, f.relay),
    "unknown",
  );
  assert.equal(f.effect(), 0, "status is read-only");
  f.failResult(true);
  await sendStop(scope, prepared, () => true, f.relay);
  await tick();
  assert.equal(f.effect(), 1);
  assert.equal(f.errors.length, 1);
  f.failResult(false);
  await sendStop(scope, prepared, () => true, f.relay);
  await tick();
  assert.equal(
    f.effect(),
    1,
    "consumed request returns saved outcome without effect",
  );
  assert.equal(
    await readStopOutcome(scope, request, () => true, f.ipc, f.relay),
    "stopped",
  );
  assert.ok(
    f.publishes.filter((e) => e.kind === 50180).every((e) => e === request),
  );
  assert.ok(
    f.publishes.filter((e) => e.kind === 50181).every((e) => e === result),
  );
  close();
  assert.equal(f.closed(), true);
});

test("duplicate delivery during native Stop/result publication is coalesced", async () => {
  const f = fixture();
  let release;
  const wait = new Promise((resolve) => {
    release = resolve;
  });
  let calls = 0;
  const ipc = async (...args) => {
    calls++;
    await wait;
    return f.ipc(...args);
  };
  const close = await receiveStops(
    scope,
    () => true,
    () => {},
    ipc,
    f.relay,
  );
  f.deliver();
  f.deliver();
  assert.equal(calls, 1);
  release();
  await tick();
  assert.equal(f.effect(), 1);
  close();
});

test("scope change after native effect prevents result publication", async () => {
  const f = fixture();
  const ipc = async (...args) => {
    const value = await f.ipc(...args);
    f.switchScope();
    return value;
  };
  const close = await receiveStops(
    scope,
    () => true,
    () => {},
    ipc,
    f.relay,
  );
  f.deliver();
  await tick();
  assert.equal(f.effect(), 1, "dispatched Stop may finish");
  assert.equal(
    f.publishes.length,
    0,
    "late result cannot cross the scope boundary",
  );
  close();
});

test("publish rate-limit/reconnect wait rechecks mounted owner scope before send", async () => {
  const f = fixture();
  let active = true;
  f.relay.publishEvent = async (_event, _timeout, _failure, check) => {
    active = false;
    check();
  };
  await assert.rejects(
    sendStop(scope, request, () => active, f.relay),
    /scope changed/,
  );
  const ipc = async (...args) => {
    const value = await f.ipc(...args);
    f.switchScope();
    return value;
  };
  await assert.rejects(
    prepareStop(scope, "desktop", "agent", () => true, ipc, f.relay),
    /scope changed/,
  );
});
