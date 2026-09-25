import assert from "node:assert/strict";
import test from "node:test";
import {
  createMockSubscription,
  hasMockSubscription,
} from "./e2eBridgeSubscriptions.ts";

const ready = (filters, channel = "channel", kind = 9, exact = true) =>
  hasMockSubscription([createMockSubscription(filters)], channel, kind, exact);

test("channel readiness rejects global-only, wrong-channel and wrong-kind REQs", () => {
  for (const filter of [
    { kinds: [9] },
    { "#h": [], kinds: [9] },
    { "#h": ["other"], kinds: [9] },
    { "#h": ["channel"], kinds: [30078] },
  ])
    assert.equal(ready([filter]), false);
  assert.equal(hasMockSubscription([], "channel", 9, true), false);
  for (const kinds of [[9], [7, 9], undefined]) {
    assert.equal(ready([{ "#h": ["channel"], kinds }]), true);
  }
  const emptyKinds = [
    createMockSubscription([{ "#h": ["channel"], kinds: [] }]),
  ];
  assert.equal(hasMockSubscription(emptyKinds, "channel", 9, true), false);
  assert.equal(
    hasMockSubscription(emptyKinds, "channel", undefined, true),
    false,
  );
  assert.equal(
    ready([
      { "#h": ["channel"], kinds: [] },
      { "#h": ["channel"], kinds: [9] },
    ]),
    true,
  );
});

test("REQ storage preserves channel/kind correlation across OR filters", () => {
  for (const unrelated of [{ kinds: [9] }, { "#h": ["other"], kinds: [9] }]) {
    const filters = [{ "#h": ["channel"], kinds: [30078] }, unrelated];
    const stored = createMockSubscription(filters);
    assert.deepEqual(stored.filters, filters);
    assert.equal(hasMockSubscription([stored], "channel", 9, true), false);
    assert.equal(hasMockSubscription([stored], "channel", 30078, true), true);
    assert.equal(ready([...filters, { "#h": ["channel"], kinds: [9] }]), true);
  }
});

test("legacy readiness and explicit global queries retain their semantics", () => {
  const global = [createMockSubscription([{ kinds: [30078] }])];
  assert.equal(hasMockSubscription(global, "channel"), true);
  assert.equal(hasMockSubscription(global, "channel", 9), false);
  assert.equal(hasMockSubscription(global, "channel", 30078), true);
  assert.equal(hasMockSubscription(global, "*", 30078), true);
  assert.equal(hasMockSubscription(global, "*", 9), false);
  assert.equal(hasMockSubscription(global, "channel", 30078, true), false);
  assert.equal(
    ready(
      [{ "#h": ["channel"], kinds: [30078] }, { kinds: [9] }],
      "channel",
      9,
      false,
    ),
    true,
  );
});
