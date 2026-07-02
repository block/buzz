import assert from "node:assert/strict";
import test from "node:test";

import {
  buildFrontierBridgeFilter,
  frontierBridge,
  isEventAfterFrontier,
  newestEventFrontier,
  shouldApplyBridgeEvent,
} from "./relayFrontierBridge.ts";
import { buildChannelFilter } from "./relayChannelFilters.ts";
import { mergeTimelineHistoryMessages } from "../../features/messages/lib/messageQueryKeys.ts";

function event(id, createdAt, kind = 9) {
  return {
    id,
    pubkey: "pubkey",
    created_at: createdAt,
    kind,
    tags: [["h", "channel-1"]],
    content: "",
    sig: "sig",
  };
}

function verifyBridge({ before, bridgePage, atomic }) {
  const frontier = newestEventFrontier(before);
  const delivered = [];

  return frontierBridge({
    frontier,
    targetFilter: buildChannelFilter("channel-1", 0),
    knownEventIds: new Set(before.map((ev) => ev.id)),
    requestHistory: async () => bridgePage,
    onEvent: (ev) => delivered.push(ev),
  }).then(() => {
    const bridged = mergeTimelineHistoryMessages(before, delivered);
    const fresh = mergeTimelineHistoryMessages(before, atomic);
    assert.deepEqual(
      bridged.map((ev) => ev.id),
      fresh.map((ev) => ev.id),
    );
  });
}

test("newestEventFrontier uses the composite (created_at, id) frontier", () => {
  const frontier = newestEventFrontier([
    event("b", 10),
    event("a", 11),
    event("c", 11),
  ]);

  assert.deepEqual(frontier, { createdAt: 11, eventId: "c" });
});

test("frontier bridge filter keeps since inclusive for same-second ties", () => {
  const targetFilter = { ...buildChannelFilter("channel-1", 0), since: 90 };
  const filter = buildFrontierBridgeFilter({
    frontier: { createdAt: 100, eventId: "b" },
    targetFilter,
  });

  assert.equal(filter.limit, 500);
  assert.equal(filter.since, 100);
  assert.deepEqual(filter["#h"], ["channel-1"]);
  assert.deepEqual(filter.kinds, targetFilter.kinds);
});

test("frontier comparison orders dense-second events by id", () => {
  const frontier = { createdAt: 100, eventId: "b" };

  assert.equal(isEventAfterFrontier(event("a", 100), frontier), false);
  assert.equal(isEventAfterFrontier(event("b", 100), frontier), false);
  assert.equal(isEventAfterFrontier(event("c", 100), frontier), true);
  assert.equal(isEventAfterFrontier(event("a", 101), frontier), true);
});

test("bridge application keeps unknown same-second events that sort before the frontier", () => {
  const frontier = { createdAt: 100, eventId: "m" };
  const knownEventIds = new Set(["m"]);

  assert.equal(
    shouldApplyBridgeEvent({
      event: event("a", 100),
      frontier,
      knownEventIds,
    }),
    true,
  );
  assert.equal(
    shouldApplyBridgeEvent({
      event: event("m", 100),
      frontier,
      knownEventIds,
    }),
    false,
  );
});

test("verify_bridge: live-only bridge equals fresh atomic fetch across dense-second and aux events", async () => {
  const before = [event("a", 100), event("b", 100), event("m", 101)];
  const afterSameSecondLowerId = event("l", 101);
  const afterSameSecond = event("z", 101);
  const afterNextSecond = event("n", 102);
  const aux = event("aux", 103, 7);
  const duplicateAtFrontier = event("m", 101);

  await verifyBridge({
    before,
    // Relay history arrives newest-first; the bridge must sort/apply into the
    // same timeline cache projection as a fresh atomic read.
    bridgePage: [
      aux,
      afterNextSecond,
      duplicateAtFrontier,
      afterSameSecond,
      afterSameSecondLowerId,
    ],
    atomic: [
      duplicateAtFrontier,
      afterSameSecondLowerId,
      afterSameSecond,
      afterNextSecond,
      aux,
    ],
  });
});

test("frontierBridge stops applying when the owner is disposed", async () => {
  const delivered = [];
  let active = true;

  await frontierBridge({
    frontier: { createdAt: 100, eventId: "a" },
    targetFilter: buildChannelFilter("channel-1", 0),
    requestHistory: async () => {
      active = false;
      return [event("b", 101)];
    },
    isActive: () => active,
    onEvent: (ev) => delivered.push(ev),
  });

  assert.deepEqual(delivered, []);
});
