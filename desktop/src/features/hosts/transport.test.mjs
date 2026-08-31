import assert from "node:assert/strict";
import test from "node:test";
import { ReadOnlyRelayClient } from "../../shared/api/readOnlyRelayClient.ts";
import { createHostPublicationJournal } from "./pendingPublication.ts";
import { reconcileHost } from "./registration.ts";
import { fixture } from "./hostTestFixtures.mjs";

const flush = () => new Promise((resolve) => setImmediate(resolve));
function transport(t, { history, publish, connect } = {}) {
  const previousWindow = globalThis.window;
  const sent = [];
  const closed = [];
  const timers = new Map();
  let timerId = 0;
  let channel;
  const emit = (...frames) =>
    channel.onmessage(
      frames.map((frame) => ({ type: "Text", data: JSON.stringify(frame) })),
    );
  globalThis.window = {
    setTimeout: (callback) => {
      timers.set(++timerId, callback);
      return timerId;
    },
    clearTimeout: (id) => timers.delete(id),
    __TAURI_INTERNALS__: {
      transformCallback: () => 1,
      invoke: async (command, args) => {
        if (command === "plugin:websocket|connect") {
          channel = args.onMessage;
          if (connect) return connect();
          setImmediate(() => emit(["AUTH", "challenge"]));
          return 7;
        }
        if (command === "create_auth_event")
          return JSON.stringify({ id: "auth" });
        if (command === "plugin:websocket|disconnect") {
          closed.push(args.id);
          return;
        }
        if (command === "plugin:websocket|send") {
          const frame = JSON.parse(args.message.data);
          sent.push(frame);
          if (frame[0] === "AUTH") emit(["OK", "auth", true, ""]);
          if (frame[0] === "REQ") await history?.(frame, emit);
          if (frame[0] === "EVENT") {
            if (publish) await publish(frame, emit);
            else emit(["OK", frame[1].id, true, ""]);
          }
          return;
        }
        throw new Error(`Unexpected IPC command: ${command}`);
      },
    },
  };
  const client = new ReadOnlyRelayClient("wss://fixture.invalid");
  t.after(() => {
    client.disconnect();
    globalThis.window = previousWindow;
  });
  return {
    client,
    sent,
    closed,
    timers,
    emit,
    native: (frame) => channel.onmessage(frame),
  };
}
const run = (f, client, journal = f.journal) =>
  reconcileHost({
    owner: "a".repeat(64),
    relay: client,
    bridge: f.bridge,
    journal,
    active: () => true,
    now: () => 100,
  });

for (const stage of ["registration", "report"]) {
  for (const mode of [
    "closed",
    "closed-then-eose",
    "partial-closed-then-eose",
    "timeout",
    "close",
    "error",
    "send-failed",
  ]) {
    test(`${stage} history ${mode} blocks all publication through the actual client`, async (t) => {
      const f = fixture();
      const registration = await f.bridge.registration();
      const report = await f.bridge.report(registration);
      let reached = false;
      const wire = transport(t, {
        history: ([, id, filter], emit) => {
          if (filter["#l"][0] !== stage) {
            emit(["EVENT", id, registration], ["EOSE", id]);
            return;
          }
          reached = true;
          if (mode.startsWith("partial"))
            emit([
              "EVENT",
              id,
              stage === "registration" ? registration : report,
            ]);
          if (mode.includes("closed"))
            emit(["CLOSED", id, "error: host history unavailable"]);
          if (mode.endsWith("eose")) emit(["EOSE", id]);
          if (mode === "close" || mode === "error")
            wire.native({ type: mode === "close" ? "Close" : "Error" });
          if (mode === "send-failed") throw new Error("send failed");
        },
      });
      const rejected = assert.rejects(
        run(f, wire.client),
        /closed|disconnected|Timed out|send failed/,
      );
      while (!reached) await flush();
      if (mode === "timeout")
        for (const callback of [...wire.timers.values()]) callback();
      await rejected;
      assert.equal(wire.sent.filter((frame) => frame[0] === "EVENT").length, 0);
    });
  }
}

test("EOSE with complete empty history permits publication only after accepted ACK", async (t) => {
  const f = fixture();
  let acknowledge;
  const wire = transport(t, {
    history: ([, id], emit) => emit(["EOSE", id]),
    publish: ([, event], emit) => {
      acknowledge = () => emit(["OK", event.id, true, ""]);
    },
  });
  let completed = false;
  const result = run(f, wire.client).then((value) => {
    completed = true;
    return value;
  });
  while (!acknowledge) await flush();
  assert.equal(completed, false);
  assert.equal(wire.sent.filter((frame) => frame[0] === "EVENT").length, 1);
  acknowledge();
  await flush();
  assert.equal(completed, false);
  assert.equal(wire.sent.filter((frame) => frame[0] === "EVENT").length, 2);
  acknowledge();
  assert.equal((await result).rows.length, 1);
});

test("negative registration ACK blocks report construction and success", async (t) => {
  const f = fixture();
  f.bridge.report = async () =>
    assert.fail("report before accepted registration");
  const wire = transport(t, {
    history: ([, id], emit) => emit(["EOSE", id]),
    publish: ([, event], emit) => emit(["OK", event.id, false, "rejected"]),
  });
  await assert.rejects(run(f, wire.client), /rejected/);
  assert.equal(wire.sent.filter((frame) => frame[0] === "EVENT").length, 1);
});

test("disconnect during delayed native connect closes the orphan and cannot resurrect a publisher", async (t) => {
  let resolve;
  const wire = transport(t, {
    connect: () =>
      new Promise((r) => {
        resolve = r;
      }),
  });
  const pending = wire.client.fetchEvents({ kinds: [50000], limit: 1000 });
  const rejected = assert.rejects(pending, /cancelled/);
  wire.client.disconnect();
  resolve(99);
  await rejected;
  assert.deepEqual(wire.closed, [99]);
  assert.equal(wire.sent.length, 0);
  assert.equal(wire.timers.size, 0);
});

test("concurrent reads cannot bypass pending authentication", async (t) => {
  let resolve;
  const wire = transport(t, {
    connect: () =>
      new Promise((r) => {
        resolve = r;
      }),
  });
  const first = wire.client.fetchEvents({ kinds: [50000], limit: 1000 });
  const rejectedFirst = assert.rejects(first, /disconnected/);
  resolve(99);
  await flush();
  const second = wire.client.fetchEvents({ kinds: [50000], limit: 1000 });
  const rejectedSecond = assert.rejects(second, /disconnected/);
  await flush();
  assert.equal(wire.sent.length, 0);
  wire.client.disconnect();
  await Promise.all([rejectedFirst, rejectedSecond]);
});

test("AUTH delivered before native connect resolves is not lost", async (t) => {
  let resolve;
  const wire = transport(t, {
    connect: () =>
      new Promise((r) => {
        resolve = r;
      }),
    history: ([, id], emit) => emit(["EOSE", id]),
  });
  const pending = wire.client.fetchEvents({ kinds: [50000], limit: 1000 });
  wire.emit(["AUTH", "early challenge"]);
  resolve(99);
  assert.deepEqual(await pending, []);
  assert.equal(wire.sent[0][0], "AUTH");
});

for (const stage of ["registration", "report"]) {
  test(`a duplicate ${stage} history through the actual client blocks construction`, async (t) => {
    const f = fixture();
    const registration = await f.bridge.registration();
    const report = await f.bridge.report(registration);
    f.bridge.registration = f.bridge.report = async () =>
      assert.fail("construction after non-advancing history");
    const wire = transport(t, {
      history: ([, id, filter], emit) => {
        assert.equal(filter.limit, 1000);
        const event =
          filter["#l"][0] === "registration" ? registration : report;
        const count = filter["#l"][0] === stage ? 1000 : 1;
        for (let i = 0; i < count; i++) emit(["EVENT", id, event]);
        emit(["EOSE", id]);
      },
    });
    await assert.rejects(run(f, wire.client), /cursor did not advance/);
    assert.equal(wire.sent.filter((frame) => frame[0] === "EVENT").length, 0);
  });

  test(`accepted ${stage} with lost transport ACK is not duplicated on reconnect`, async (t) => {
    const f = fixture();
    let lost = false;
    let loseAck = true;
    const wire = transport(t, {
      history: async ([, id, filter], emit) => {
        for (const event of await f.relay.fetchEvents(filter))
          emit(["EVENT", id, event]);
        emit(["EOSE", id]);
      },
      publish: async ([, event], emit) => {
        await f.relay.publishEvent(event);
        if (
          loseAck &&
          event.tags.some(
            (tag) =>
              tag[0] === "l" &&
              tag[1] === (stage === "report" ? "profile" : stage),
          )
        ) {
          lost = true;
          return;
        }
        emit(["OK", event.id, true, ""]);
      },
    });
    const rejected = assert.rejects(
      run(f, wire.client),
      /Timed out publishing/,
    );
    while (!lost) await flush();
    for (const callback of [...wire.timers.values()]) callback();
    await rejected;
    wire.client.disconnect();
    loseAck = false;
    const snapshot = await run(f, wire.client);
    assert.equal(snapshot.rows.length, 1);
    assert.equal(f.writes.length, 2);
    assert.equal(wire.sent.filter((frame) => frame[0] === "EVENT").length, 2);
  });
}

for (const stage of ["registration", "report"]) {
  for (const reload of [false, true]) {
    test(`late ${stage} commit AFTER retry history reuses exact signed event${reload ? " after journal reload" : ""}`, async (t) => {
      const f = fixture();
      let held;
      let retryRead = false;
      let retrying = false;
      const wire = transport(t, {
        history: async ([, id, filter], emit) => {
          const events = await f.relay.fetchEvents(filter);
          if (retrying && filter["#l"][0] === stage) {
            assert.ok(!events.some((e) => e.id === held.id));
            retryRead = true;
          }
          for (const event of events) emit(["EVENT", id, event]);
          emit(["EOSE", id]);
        },
        publish: async ([, event], emit) => {
          if (
            event.tags.some(
              (tag) =>
                tag[0] === "l" &&
                tag[1] === (stage === "report" ? "profile" : stage),
            )
          ) {
            if (!held) {
              held = event;
              assert.deepEqual(
                f.journal.load()[
                  stage === "report" ? "report" : "registration"
                ],
                event,
              );
              return; // Detached old ingest remains uncommitted past timeout.
            }
            assert.equal(retryRead, true);
            assert.deepEqual(event, held); // All seven signed fields, not just ID.
            await f.relay.publishEvent(event);
            await f.relay.publishEvent(held); // Old EVENT commits after retry read/send.
          } else {
            await f.relay.publishEvent(event);
          }
          emit(["OK", event.id, true, ""]);
        },
      });
      const rejected = assert.rejects(
        run(f, wire.client),
        /Timed out publishing/,
      );
      while (!held) await flush();
      for (const callback of [...wire.timers.values()]) callback();
      await rejected;
      wire.client.disconnect();
      retrying = true;
      f.setNow(130);
      // A fresh journal object reads only serialized storage; no uncertain-event
      // object or decrypted report is retained by the restarted controller.
      const journal = reload
        ? createHostPublicationJournal(
            "wss://fixture.invalid",
            "a".repeat(64),
            f.storage,
          )
        : f.journal;
      const snapshot = await run(f, wire.client, journal);
      assert.equal(snapshot.rows.length, 1);
      assert.equal(f.events.length, 2);
      assert.equal(
        f.events.filter((e) =>
          e.tags.some(
            (tag) => tag[1] === (stage === "report" ? "profile" : stage),
          ),
        ).length,
        1,
      );
      assert.equal(wire.sent.filter((frame) => frame[0] === "EVENT").length, 3);
      assert.equal(journal.load(), undefined);
    });
  }
}
