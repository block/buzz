import assert from "node:assert/strict";
import test from "node:test";

import {
  filterWorkstreamChannels,
  WORKSTREAM_CHANNEL_PREFIX,
} from "./discoverWorkstreamChannels.ts";

function buildChannel(overrides) {
  return {
    id: overrides.id ?? "channel-id",
    name: overrides.name,
    channelType: "stream",
    visibility: "open",
    description: "",
    topic: null,
    purpose: null,
    memberCount: overrides.memberPubkeys?.length ?? 0,
    memberPubkeys: overrides.memberPubkeys ?? [],
    lastMessageAt: null,
    archivedAt: overrides.archivedAt ?? null,
    participants: [],
    participantPubkeys: [],
    isMember: overrides.isMember ?? true,
    ttlSeconds: null,
    ttlDeadline: null,
  };
}

test("prefix constant matches the contract-specified prefix", () => {
  assert.equal(WORKSTREAM_CHANNEL_PREFIX, "loganj-ws-");
});

test("includes joined channels whose name starts exactly with the prefix", () => {
  const channels = [
    buildChannel({ id: "1", name: "loganj-ws-canvas-cards", isMember: true }),
    buildChannel({ id: "2", name: "general", isMember: true }),
  ];

  const result = filterWorkstreamChannels(channels);
  assert.deepEqual(
    result.map((c) => c.id),
    ["1"],
  );
});

test("excludes prefixed channels the current user has not joined", () => {
  const channels = [
    buildChannel({ id: "joined", name: "loganj-ws-joined", isMember: true }),
    buildChannel({
      id: "unjoined",
      name: "loganj-ws-unjoined",
      isMember: false,
    }),
  ];

  const result = filterWorkstreamChannels(channels);
  assert.deepEqual(
    result.map((c) => c.id),
    ["joined"],
  );
});

test("excludes channels that merely contain the prefix mid-name", () => {
  const channels = [
    buildChannel({ id: "1", name: "not-loganj-ws-canvas-cards" }),
    buildChannel({ id: "2", name: "loganj-ws-canvas-cards" }),
  ];

  const result = filterWorkstreamChannels(channels);
  assert.deepEqual(
    result.map((c) => c.id),
    ["2"],
  );
});

test("excludes a near-miss name missing the trailing hyphen", () => {
  const channels = [
    buildChannel({ id: "1", name: "loganj-ws" }),
    buildChannel({ id: "2", name: "loganj-ws-" }),
  ];

  const result = filterWorkstreamChannels(channels);
  assert.deepEqual(
    result.map((c) => c.id),
    ["2"],
  );
});

test("includes every joined matching channel without a creator filter", () => {
  const channels = [
    // The list response has no creator field. Distinct membership sets prove
    // that matching joined channels are included without creator/ownership
    // filtering, while the unjoined control remains excluded.
    buildChannel({
      id: "mine",
      name: "loganj-ws-mine",
      isMember: true,
      memberPubkeys: ["aa"],
    }),
    buildChannel({
      id: "delegated",
      name: "loganj-ws-delegated",
      isMember: true,
      memberPubkeys: ["bb", "cc"],
    }),
    buildChannel({
      id: "unjoined",
      name: "loganj-ws-unjoined",
      isMember: false,
      memberPubkeys: ["dd"],
    }),
  ];

  const result = filterWorkstreamChannels(channels);
  assert.deepEqual(result.map((c) => c.id).sort(), ["delegated", "mine"]);
});

test("excludes an archived channel even when its name matches the prefix", () => {
  const channels = [
    buildChannel({
      id: "1",
      name: "loganj-ws-done",
      archivedAt: "2026-01-01T00:00:00Z",
    }),
    buildChannel({ id: "2", name: "loganj-ws-active" }),
  ];

  const result = filterWorkstreamChannels(channels);
  assert.deepEqual(
    result.map((c) => c.id),
    ["2"],
  );
});

test("returns an empty array when nothing matches", () => {
  const channels = [
    buildChannel({ id: "1", name: "general" }),
    buildChannel({ id: "2", name: "random" }),
  ];

  assert.deepEqual(filterWorkstreamChannels(channels), []);
});

test("returns an empty array for an empty channel list", () => {
  assert.deepEqual(filterWorkstreamChannels([]), []);
});
