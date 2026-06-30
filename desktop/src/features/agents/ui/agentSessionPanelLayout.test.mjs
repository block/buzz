import assert from "node:assert/strict";
import test from "node:test";

import {
  deriveLatestSessionId,
  resolveDisplayEvents,
  resolveRawRailLayout,
  scopeByChannel,
} from "./agentSessionPanelLayout.ts";

// ---- scopeByChannel ----

const items = [
  { id: "a", channelId: "channel-1" },
  { id: "b", channelId: "channel-2" },
  { id: "c", channelId: "channel-1" },
];

test("scopeByChannel returns the input unchanged when channelId is null", () => {
  assert.equal(scopeByChannel(items, null), items);
});

test("scopeByChannel returns the input unchanged when channelId is undefined", () => {
  assert.equal(scopeByChannel(items, undefined), items);
});

test("scopeByChannel filters items down to the requested channel", () => {
  const scoped = scopeByChannel(items, "channel-1");
  assert.deepEqual(
    scoped.map((item) => item.id),
    ["a", "c"],
  );
});

test("scopeByChannel returns an empty array when no item matches", () => {
  assert.deepEqual(scopeByChannel(items, "channel-99"), []);
});

// ---- deriveLatestSessionId ----

test("deriveLatestSessionId returns null for an empty list", () => {
  assert.equal(deriveLatestSessionId([]), null);
});

test("deriveLatestSessionId returns the last event's sessionId", () => {
  const events = [
    { seq: 1, sessionId: "sess-1" },
    { seq: 2, sessionId: "sess-2" },
  ];
  assert.equal(deriveLatestSessionId(events), "sess-2");
});

test("deriveLatestSessionId skips trailing events without a sessionId", () => {
  const events = [
    { seq: 1, sessionId: "sess-1" },
    { seq: 2, sessionId: null },
    { seq: 3, sessionId: undefined },
  ];
  assert.equal(deriveLatestSessionId(events), "sess-1");
});

test("deriveLatestSessionId returns null when no event carries a sessionId", () => {
  const events = [{ seq: 1, sessionId: null }, { seq: 2 }];
  assert.equal(deriveLatestSessionId(events), null);
});

// ---- resolveDisplayEvents ----

test("resolveDisplayEvents returns raw override events unchanged", () => {
  const scopedEvents = [{ seq: 1, channelId: "channel-1" }];
  const rawEventsOverride = [{ seq: 2, channelId: "debug-channel" }];
  assert.equal(
    resolveDisplayEvents(scopedEvents, rawEventsOverride),
    rawEventsOverride,
  );
});

test("resolveDisplayEvents falls back to scoped live events", () => {
  const scopedEvents = [{ seq: 1, channelId: "channel-1" }];
  assert.equal(resolveDisplayEvents(scopedEvents, undefined), scopedEvents);
});

// ---- resolveRawRailLayout (raw-ACP view toggle) ----

test("resolveRawRailLayout hides the rail when showRaw is off", () => {
  assert.deepEqual(resolveRawRailLayout(false, "responsive"), {
    mode: "hidden",
  });
  assert.deepEqual(resolveRawRailLayout(false, "exclusive"), {
    mode: "hidden",
  });
});

test("resolveRawRailLayout renders the rail exclusively when toggled on in exclusive layout", () => {
  assert.deepEqual(resolveRawRailLayout(true, "exclusive"), {
    mode: "exclusive",
  });
});

test("resolveRawRailLayout renders the rail beside the transcript in responsive layout", () => {
  assert.deepEqual(resolveRawRailLayout(true, "responsive"), { mode: "side" });
});
