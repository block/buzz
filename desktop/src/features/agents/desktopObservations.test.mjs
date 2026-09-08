import assert from "node:assert/strict";
import test from "node:test";
import {
  refreshDesktopObservations,
  desktopFreshness,
} from "./desktopObservations.ts";

const scope = { owner: "owner-a", community: "wss://a.example" };
function fixture(boundary) {
  let epoch = 0;
  const calls = [];
  const event = { id: "pulse", kind: 30181 };
  const rows = [
    { id: "a", heard: 100 },
    { id: "b", heard: 10 },
  ];
  const finish = (name, value) => {
    if (name === boundary) epoch++;
    return value;
  };
  const f = {
    calls,
    rows,
    ipc: async (command, args) => {
      assert.equal(args.owner, scope.owner);
      assert.equal(args.community, scope.community);
      calls.push(command);
      if (command === "prepare_desktop_observation")
        return finish(command, { event });
      assert.equal(command, "read_desktop_observations");
      return finish(command, rows);
    },
    relay: {
      getSessionEpoch: () => epoch,
      publishEvent: async (value) => {
        assert.equal(value, event);
        calls.push("publish");
        finish("publishEvent");
      },
      fetchEvents: async (filter) => {
        assert.deepEqual(filter, {
          kinds: [30181],
          authors: [scope.owner],
          limit: 100,
        });
        calls.push("fetch");
        return finish("fetchEvents", [event]);
      },
    },
  };
  f.refresh = (active = () => true) =>
    refreshDesktopObservations(scope, active, f.ipc, f.relay);
  return f;
}

test("every async completion is fenced, including decrypt and late ACK", async () => {
  for (const boundary of [
    "prepare_desktop_observation",
    "publishEvent",
    "fetchEvents",
    "read_desktop_observations",
  ]) {
    const f = fixture(boundary);
    await assert.rejects(f.refresh(), /scope changed/);
    if (boundary === "prepare_desktop_observation")
      assert.ok(!f.calls.includes("publish"));
  }
  const f = fixture();
  await assert.rejects(f.refresh(() => false));
  assert.deepEqual(f.calls, []);
});

test("bounded/invalid reads are not silently authoritative, clocks and staleness stay advisory", async () => {
  const f = fixture();
  const good = await f.refresh();
  assert.deepEqual(good.rows, f.rows);
  assert.deepEqual(f.calls, [
    "prepare_desktop_observation",
    "publish",
    "fetch",
    "read_desktop_observations",
  ]);
  f.relay.publishEvent = async () => {
    throw Error("offline");
  };
  assert.ok((await f.refresh()).warning);
  f.relay.fetchEvents = async () => Array(100).fill({});
  assert.equal((await f.refresh()).partial, true);
  const read = f.ipc;
  f.ipc = async (command, args) => {
    if (command === "read_desktop_observations")
      throw Error("invalid signature");
    return read(command, args);
  };
  await assert.rejects(f.refresh(), /invalid signature/);
  for (const [heard, now, expected] of [
    [undefined, 100, "Unknown"],
    [100, 280, "Recent"],
    [100, 281, "Stale"],
    [101, 100, "Unknown (Desktop clock ahead)"],
  ])
    assert.ok(desktopFreshness(heard, now).startsWith(expected));
});

test("history/live replacement races select newest signed time and lower ID, not arrival", async () => {
  const f = fixture();
  const events = [
    { id: "old", created_at: 10 },
    { id: "b", created_at: 20 },
    { id: "a", created_at: 20 },
  ];
  const ipc = f.ipc;
  f.ipc = async (command, args) => {
    if (command === "read_desktop_observations")
      assert.deepEqual(
        args.events.map((e) => e.id),
        ["a", "b", "old"],
      );
    return ipc(command, args);
  };
  for (const batch of [events, [...events].reverse()]) {
    f.relay.fetchEvents = async () => batch;
    await f.refresh();
  }
});
