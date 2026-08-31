import assert from "node:assert/strict";
import test from "node:test";
import { fixture, memoryStorage } from "./hostTestFixtures.mjs";
import {
  canonicalHostEvent,
  createHostPublicationJournal,
} from "./pendingPublication.ts";

async function uncertain(f, stage) {
  const publish = f.relay.publishEvent;
  let held;
  f.relay.publishEvent = async (event) => {
    if (
      event.tags.some((t) => t[1] === (stage === "report" ? "profile" : stage))
    ) {
      held = event;
      throw new Error("uncertain");
    }
    return publish(event);
  };
  await assert.rejects(f.run(), /uncertain/);
  f.relay.publishEvent = publish;
  return held;
}

for (const stage of ["registration", "report"]) {
  test(`${stage} journal stores only canonical ciphertext before first send and survives repeated rejection`, async () => {
    const f = fixture();
    f.payload.name = "PRIVATE-HOSTNAME";
    const held = await uncertain(f, stage);
    const raw = [...f.storage.entries.values()][0];
    assert.ok(!raw.includes("PRIVATE-HOSTNAME"));
    assert.ok(!raw.includes("decoded"));
    assert.ok(!raw.includes("runtimes"));
    assert.equal(Object.keys(held).length, 7);
    f.relay.publishEvent = async (event) => {
      assert.deepEqual(event, held);
      assert.equal([...f.storage.entries.values()][0], raw);
      throw new Error("rejected again");
    };
    f.bridge.registration = f.bridge.report = async () =>
      assert.fail("must not rebuild");
    await assert.rejects(f.run(), /rejected again/);
    assert.equal([...f.storage.entries.values()][0], raw);
  });

  for (const mode of ["failed", "capped", "unreadable"]) {
    test(`pending ${stage} recovery waits for all ${mode} history before any send`, async () => {
      const f = fixture();
      await uncertain(f, stage);
      const raw = [...f.storage.entries.values()][0];
      const fetch = f.relay.fetchEvents;
      f.relay.fetchEvents = async (filter) => {
        if (filter["#l"][0] === "report") {
          if (mode === "failed") throw new Error("history failed");
          const report = await f.bridge.report(f.journal.load().registration);
          if (mode === "capped") return Array(1000).fill(report);
          return [{ ...report, tags: [] }];
        }
        return fetch(filter);
      };
      f.relay.publishEvent = async () =>
        assert.fail("send after unsafe history");
      await assert.rejects(
        f.run(),
        /history failed|cursor did not advance|Cannot establish/,
      );
      assert.equal([...f.storage.entries.values()][0], raw);
    });
  }

  test(`switch during pending ${stage} native validation fences recovery and preserves journal`, async () => {
    const f = fixture();
    await uncertain(f, stage);
    const raw = [...f.storage.entries.values()][0];
    const operation = stage === "registration" ? "inspect" : "decode";
    const native = f.bridge[operation];
    f.bridge[operation] = async (...args) => {
      const result = await native(...args);
      f.stop();
      return result;
    };
    f.relay.publishEvent = async () => assert.fail("stale send");
    await assert.rejects(f.run(), /cancelled/);
    assert.equal([...f.storage.entries.values()][0], raw);
  });

  test(`crash after saving ${stage} but before send recovers solely from persisted signed event`, async () => {
    const f = fixture();
    const registration = canonicalHostEvent(await f.bridge.registration());
    const report =
      stage === "report"
        ? canonicalHostEvent(await f.bridge.report(registration))
        : undefined;
    if (report) f.events.push(registration);
    f.journal.save({ v: 1, registration, ...(report ? { report } : {}) });
    const restarted = createHostPublicationJournal(
      "wss://fixture.invalid",
      "a".repeat(64),
      f.storage,
    );
    f.bridge.registration = async () =>
      assert.fail("rebuilding pending registration");
    if (report)
      f.bridge.report = async () => assert.fail("rebuilding pending report");
    await f.run({ journal: restarted });
    assert.deepEqual(f.writes[0], report ?? registration);
    assert.equal(restarted.load(), undefined);
  });
}

for (const raw of [
  "",
  "{",
  "null",
  "[]",
  '{"v":2}',
  '{"v":1,"registration":{}}',
]) {
  test(`malformed journal ${JSON.stringify(raw)} is not discarded or treated as absence`, async () => {
    const f = fixture();
    await uncertain(f, "registration");
    const key = [...f.storage.entries.keys()][0];
    f.storage.entries.set(key, raw);
    f.relay.publishEvent = async () => assert.fail("must fail closed");
    await assert.rejects(f.run(), /pending publication/);
    assert.equal(f.storage.entries.get(key), raw);
  });
}

for (const field of ["content", "sig", "pubkey"]) {
  for (const stage of ["registration", "report"]) {
    test(`native validation rejects shape-valid tampered ${stage} ${field} without sending or clearing`, async () => {
      const f = fixture();
      const held = await uncertain(f, stage);
      const [key, raw] = [...f.storage.entries][0];
      const pending = JSON.parse(raw);
      pending[stage][field] =
        field === "content"
          ? "tampered-ciphertext"
          : "e".repeat(field === "sig" ? 128 : 64);
      f.storage.entries.set(key, JSON.stringify(pending));
      // Simulated native cryptographic boundary: native receives and rejects
      // the changed signed fields. JS shape validation must not replace it.
      let validations = 0;
      const operation = stage === "registration" ? "inspect" : "decode";
      const original = f.bridge[operation];
      f.bridge[operation] = async (...args) => {
        validations++;
        if (args.at(-1)[field] !== held[field])
          throw new Error("native verification failed");
        return original(...args);
      };
      f.relay.publishEvent = async () => assert.fail("tampered send");
      await assert.rejects(f.run(), /native verification failed/);
      assert.equal(validations, 1);
      assert.equal(f.storage.entries.has(key), true);
    });
  }
}

test("foreign local host and missing report binding fail closed on recovery", async () => {
  const f = fixture();
  await uncertain(f, "report");
  f.bridge.local = async () => ({ host: "c".repeat(64), report: f.payload });
  await assert.rejects(f.run(), /different local host/);
  f.bridge.local = async () => ({ host: "b".repeat(64), report: f.payload });
  f.events.length = 0;
  await assert.rejects(f.run(), /registration is missing/);
  assert.ok(f.journal.load());
});

test("identity/community scopes are independent and switching back restores the exact pending event", async () => {
  const f = fixture();
  const held = await uncertain(f, "registration");
  const otherOwner = createHostPublicationJournal(
    "wss://fixture.invalid",
    "c".repeat(64),
    f.storage,
  );
  const otherRelay = createHostPublicationJournal(
    "wss://other.invalid",
    "a".repeat(64),
    f.storage,
  );
  assert.equal(otherOwner.load(), undefined);
  assert.equal(otherRelay.load(), undefined);
  const registration = await f.bridge.registration();
  otherRelay.save({ v: 1, registration });
  otherOwner.save({
    v: 1,
    registration: { ...registration, pubkey: "c".repeat(64) },
  });
  await f.run({
    journal: createHostPublicationJournal(
      "wss://fixture.invalid",
      "a".repeat(64),
      f.storage,
    ),
  });
  assert.deepEqual(f.writes[0], held);
  assert.ok(otherRelay.load());
  assert.ok(otherOwner.load());
  // A copied foreign slot is not authority: the expected-owner native bridge
  // rejects the registration, even though its stored shape is valid.
  await assert.rejects(f.run({ journal: otherOwner }), /foreign registration/);
});

for (const failure of ["read", "write", "silent-write"]) {
  test(`${failure} storage failure blocks first send`, async () => {
    const f = fixture();
    const storage = memoryStorage();
    if (failure === "read")
      storage.getItem = () => {
        throw new Error("private diagnostic");
      };
    else
      storage.setItem = () => {
        if (failure === "write") throw new Error("private diagnostic");
      };
    const journal = createHostPublicationJournal(
      "wss://fixture.invalid",
      "a".repeat(64),
      storage,
    );
    await assert.rejects(
      f.run({ journal }),
      (e) =>
        /pending publication/.test(e.message) && !e.message.includes("private"),
    );
    assert.equal(f.writes.length, 0);
  });
}

test("clear failure after accepted send preserves pending state for read-based recovery", async () => {
  const f = fixture();
  const remove = f.storage.removeItem;
  f.storage.removeItem = () => {
    throw new Error("private diagnostic");
  };
  await assert.rejects(f.run(), /pending publication/);
  assert.equal(f.writes.length, 1);
  assert.ok(f.journal.load());
  f.storage.removeItem = remove;
  await f.run();
  assert.equal(f.writes.length, 2);
  assert.equal(f.journal.load(), undefined);
});

test("unconfirmed durable profile retries the exact event even after many lease windows", async () => {
  const f = fixture();
  const held = await uncertain(f, "report");
  f.setNow(10000);
  const snapshot = await f.run();
  assert.equal(f.writes.length, 2);
  assert.deepEqual(f.writes[1], held);
  assert.equal(snapshot.rows[0].event.id, held.id);
  assert.equal(f.journal.load(), undefined);
});

test("pending older report cannot replace newer relay-confirmed capabilities", async () => {
  const f = fixture();
  const held = await uncertain(f, "report");
  f.setNow(110);
  f.payload.name = "newer";
  const newer = await f.bridge.report(f.events[0]);
  f.events.push(newer);
  const snapshot = await f.run();
  assert.deepEqual(f.writes.at(-1), held);
  assert.equal(snapshot.rows[0].event.id, newer.id);
  assert.equal(snapshot.rows[0].report.name, "newer");
});

for (const [name, mutate] of [
  [
    "extra plaintext field",
    (p) => {
      p.registration.decoded = { name: "PRIVATE-HOSTNAME" };
    },
  ],
  [
    "wrong timestamp",
    (p) => {
      p.registration.created_at = -1;
    },
  ],
  [
    "malformed tag",
    (p) => {
      p.registration.tags = [null];
    },
  ],
  [
    "wrong signature shape",
    (p) => {
      p.registration.sig = "bad";
    },
  ],
  [
    "oversized journal",
    (p) => {
      p.registration.content = "x".repeat(1024 * 1024);
    },
  ],
]) {
  test(`${name} in stored journal fails closed before native recovery`, async () => {
    const f = fixture();
    await uncertain(f, "registration");
    const [key, raw] = [...f.storage.entries][0];
    const pending = JSON.parse(raw);
    mutate(pending);
    f.storage.entries.set(key, JSON.stringify(pending));
    f.bridge.inspect = async () =>
      assert.fail("malformed journal passed to native");
    await assert.rejects(f.run(), /pending publication/);
    assert.equal(f.writes.length, 0);
    assert.ok(f.storage.entries.has(key));
  });
}
