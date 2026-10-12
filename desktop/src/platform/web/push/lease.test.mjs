import assert from "node:assert/strict";
import test from "node:test";
import {
  leasePlaintext,
  leaseSubscriptions,
  leaseTags,
  parsePushDescriptor,
} from "./lease.ts";

const me = "a".repeat(64);
const info = {
  push: {
    origin: "wss://buzz.huladesk.com",
    keys: [{ id: "k1", pubkey: "b".repeat(64), current: true }],
    app_profiles: [
      { id: "buzz-ios-dogfood", transport: "apns" },
      { id: "buzz-web", transport: "webpush" },
    ],
    limitation: { max_lease_ttl: 2592000 },
  },
};

test("descriptor requires the buzz-web profile and one current key", () => {
  assert.equal(parsePushDescriptor(info)?.executorPubkey, "b".repeat(64));
  assert.equal(
    parsePushDescriptor({
      push: {
        ...info.push,
        app_profiles: [{ id: "buzz-ios-dogfood", transport: "apns" }],
      },
    }),
    null,
  );
  assert.equal(parsePushDescriptor({}), null);
});

test("subscriptions follow prefs; DMs and threads are relay-scoped", () => {
  const subs = leaseSubscriptions(me, {
    dm: true,
    mention: true,
    thread_reply: true,
    needs_action: true,
  });
  assert.deepEqual(
    subs.map((s) => s.class),
    ["mention", "needs_action", "dm", "thread_reply"],
  );
  assert.deepEqual(subs[0].filter["#p"], [me]);
  assert.equal(subs[2].scope, "dm");
  assert.equal(subs[3].scope, "thread");
  for (const s of subs.slice(2))
    assert.deepEqual(Object.keys(s.filter), ["kinds"]);
  assert.deepEqual(
    leaseSubscriptions(me, {
      dm: false,
      mention: false,
      thread_reply: false,
      needs_action: false,
    }),
    [],
  );
});

test("plaintext matches relay schema; empty => inactive", () => {
  const d = parsePushDescriptor(info);
  const active = JSON.parse(
    leasePlaintext({
      descriptor: d,
      generation: 5,
      grant: "G",
      subscriptions: leaseSubscriptions(me, {
        dm: false,
        mention: true,
        thread_reply: false,
        needs_action: false,
      }),
    }),
  );
  assert.deepEqual(Object.keys(active).sort(), [
    "active",
    "app_profile",
    "endpoint",
    "generation",
    "origin",
    "subscriptions",
    "transport",
    "v",
  ]);
  assert.equal(active.transport, "webpush");
  const off = JSON.parse(
    leasePlaintext({
      descriptor: d,
      generation: 6,
      grant: null,
      subscriptions: [],
    }),
  );
  assert.deepEqual(Object.keys(off).sort(), [
    "active",
    "generation",
    "origin",
    "v",
  ]);
  assert.deepEqual(
    leaseTags("inst", d, 99).map((t) => t[0]),
    ["d", "expiration", "exec", "alt"],
  );
});
