import assert from "node:assert/strict";
import test from "node:test";
import { needsReport, isFresh } from "./registration.ts";
import { fixture } from "./hostTestFixtures.mjs";

test("register once; an unchanged current host does not append on restart/reconnect", async () => {
  const f = fixture();
  const first = await f.run();
  assert.equal(f.writes.length, 2);
  assert.equal(first.rows.length, 1);
  const second = await f.run();
  assert.equal(f.writes.length, 2);
  assert.equal(second.rows[0].registration.id, first.rows[0].registration.id);
});

test("unchanged profiles do not renew across multiple lease windows", async () => {
  const f = fixture();
  await f.run();
  f.setNow(220);
  await f.run();
  assert.equal(f.writes.length, 2);
  f.setNow(240);
  await f.run();
  assert.equal(f.writes.length, 2);
  f.setNow(10000);
  await f.run();
  assert.equal(f.writes.length, 2);
});

test("capability changes publish a report without re-registering", async () => {
  const f = fixture();
  await f.run();
  f.payload.runtimes[0].availability = "cli_missing";
  f.setNow(110);
  await f.run();
  assert.equal(f.writes.length, 3);
});

test("ciphertext, property order, and runtime ordering are not change detectors", async () => {
  const f = fixture();
  const state = await f.run();
  const current = {
    accepts_start: false,
    ...f.payload,
    runtimes: [...f.payload.runtimes].reverse(),
  };
  assert.equal(needsReport(state.rows[0], current, 101), false);
});

test("failed relay read does not blindly append", async () => {
  const f = fixture();
  f.relay.fetchEvents = async () => {
    throw new Error("offline");
  };
  await assert.rejects(f.run(), /offline/);
  assert.equal(f.writes.length, 0);
});

test("failed registration acknowledgement prevents reports and success", async () => {
  const f = fixture();
  f.relay.publishEvent = async () => {
    throw new Error("rejected");
  };
  await assert.rejects(f.run(), /rejected/);
  assert.equal(f.events.length, 0);
});

test("accepted registration is reused after failed report acknowledgement", async () => {
  const f = fixture();
  const publish = f.relay.publishEvent;
  f.relay.publishEvent = async (event) => {
    if (event.tags.some((t) => t[0] === "l" && t[1] === "profile"))
      throw new Error("report rejected");
    await publish(event);
  };
  await assert.rejects(f.run(), /report rejected/);
  f.relay.publishEvent = publish;
  await f.run();
  assert.equal(f.writes.length, 2);
});

test("identity/relay switch fences delayed native results from publication", async () => {
  const f = fixture();
  const local = f.bridge.local;
  f.bridge.local = async () => {
    f.stop();
    return local();
  };
  await assert.rejects(f.run(), /cancelled/);
  assert.equal(f.writes.length, 0);
});

test("malformed or foreign registration fails closed", async () => {
  const f = fixture();
  await f.run();
  f.bridge.inspect = async () => {
    throw new Error("foreign registration");
  };
  await assert.rejects(f.run(), /Cannot establish local host registration/);
  assert.equal(f.writes.length, 2);
});

test("legacy report freshness remains bounded", async () => {
  const f = fixture({ legacy: true });
  const state = await f.run();
  const event = state.rows[0].event;
  assert.equal(isFresh(event, 279), true);
  assert.equal(isFresh(event, 280), false);
  assert.equal(isFresh(undefined, 100), false);
});

test("fresh process reads correct relay state and performs zero publication calls", async () => {
  const original = fixture();
  await original.run();
  const restarted = fixture();
  restarted.events.push(...structuredClone(original.events));
  for (const [id, report] of original.decoded)
    restarted.decoded.set(id, report);
  // No in-memory publication cache from the original process survives.
  restarted.bridge.registration = async () => {
    throw new Error("unexpected registration");
  };
  restarted.bridge.report = async () => {
    throw new Error("unexpected report");
  };
  for (const trigger of ["restart", "reconnect", "manual refresh"]) {
    restarted.setNow(110);
    const result = await restarted.run();
    assert.equal(result.rows.length, 1, trigger);
    assert.equal(restarted.writes.length, 0, trigger);
  }
});

test("failed report read never appends to an existing registration", async () => {
  const f = fixture();
  await f.run();
  f.writes.length = 0;
  const fetch = f.relay.fetchEvents;
  f.relay.fetchEvents = async (filter) => {
    if (filter["#l"][0] === "report") throw new Error("read failed");
    return fetch(filter);
  };
  f.setNow(1000);
  await assert.rejects(f.run(), /read failed/);
  assert.equal(f.writes.length, 0);
});

test("a same-second capability change waits instead of creating ambiguous heads", async () => {
  const f = fixture();
  await f.run();
  f.payload.name = "changed";
  await assert.rejects(f.run(), /current second/);
  assert.equal(f.writes.length, 2);
  f.setNow(101);
  await f.run();
  assert.equal(f.writes.length, 3);
  await f.run();
  assert.equal(f.writes.length, 3);
});

test("a future profile does not cause needless publication", async () => {
  const f = fixture();
  const state = await f.run();
  state.rows[0].event.created_at = 150;
  assert.equal(needsReport(state.rows[0], f.payload, 100), false);
});

test("incomplete registration history fails closed without publishing", async () => {
  const f = fixture();
  const event = await f.bridge.registration();
  f.events.push(...Array(1000).fill(event));
  await assert.rejects(f.run(), /cursor did not advance/);
  assert.equal(f.writes.length, 0);
});

test("failed remembered-host read prevents even a new local registration", async () => {
  const f = fixture();
  await f.run();
  f.writes.length = 0;
  f.bridge.local = async () => ({ host: "c".repeat(64), report: f.payload });
  let constructions = 0;
  f.bridge.registration = async () => {
    constructions++;
    throw new Error("must not construct");
  };
  const fetch = f.relay.fetchEvents;
  f.relay.fetchEvents = async (filter) => {
    if (filter["#l"][0] === "report") throw new Error("history unavailable");
    return fetch(filter);
  };
  await assert.rejects(f.run(), /history unavailable/);
  assert.equal(constructions, 0);
  assert.equal(f.writes.length, 0);
});

for (const label of ["registration", "profile"]) {
  test(`accepted ${label} with lost ACK is reused after reconnect`, async () => {
    const f = fixture();
    const publish = f.relay.publishEvent;
    f.relay.publishEvent = async (event) => {
      await publish(event);
      if (event.tags.some((t) => t[0] === "l" && t[1] === label))
        throw new Error("ACK lost");
    };
    await assert.rejects(f.run(), /ACK lost/);
    f.relay.publishEvent = publish;
    const result = await f.run();
    assert.equal(result.rows.length, 1);
    assert.equal(f.writes.length, 2);
    assert.equal(f.events.length, 2);
  });
}

for (const operation of ["registration", "report", "inspect", "decode"]) {
  test(`identity switch during ${operation} prevents later publication`, async () => {
    const f = fixture();
    await f.run();
    f.writes.length = 0;
    // Force the native stage under test to be reached.
    if (operation === "registration") f.events.length = 0;
    if (operation === "report") {
      f.setNow(240);
      f.payload.name = "Changed";
    }
    const original = f.bridge[operation];
    f.bridge[operation] = async (...args) => {
      const result = await original(...args);
      f.stop();
      return result;
    };
    await assert.rejects(f.run(), /cancelled/);
    assert.equal(f.writes.length, 0);
  });
}

test("legacy report is upgraded once to a durable profile", async () => {
  const legacy = fixture({ legacy: true });
  await legacy.run();
  const current = fixture();
  current.events.push(...structuredClone(legacy.events));
  for (const [id, report] of legacy.decoded) current.decoded.set(id, report);
  current.setNow(300);
  await current.run();
  assert.equal(current.writes.length, 1);
  assert.equal(current.writes[0].tags.find((t) => t[0] === "l")[1], "profile");
  current.setNow(10000);
  await current.run();
  assert.equal(current.writes.length, 1);
});
