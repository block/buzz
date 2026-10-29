import assert from "node:assert/strict";
import test from "node:test";

import { applyChannelLastMessageAt } from "./channelRecency.ts";
import { mergeConcurrentChannelRecency } from "./channelRecencyMerge.ts";

function makeChannel(id, lastMessageAt = null) {
  return { id, lastMessageAt };
}

test("applyChannelLastMessageAt advances only the matching channel", () => {
  const general = makeChannel("general", "2026-01-01T00:00:00.000Z");
  const design = makeChannel("design", "2026-01-01T00:00:00.000Z");

  const result = applyChannelLastMessageAt(
    [general, design],
    "design",
    1_767_225_660,
  );

  assert.notStrictEqual(result, undefined);
  assert.strictEqual(result[0], general);
  assert.notStrictEqual(result[1], design);
  assert.equal(result[1].lastMessageAt, "2026-01-01T00:01:00.000Z");
});

test("applyChannelLastMessageAt ignores stale or equal timestamps", () => {
  const design = makeChannel("design", "2026-01-01T00:01:00.000Z");
  const channels = [design];

  assert.strictEqual(
    applyChannelLastMessageAt(channels, "design", 1_767_225_600),
    channels,
  );
  assert.strictEqual(
    applyChannelLastMessageAt(channels, "design", "2026-01-01T00:01:00.000Z"),
    channels,
  );
});

test("applyChannelLastMessageAt preserves the list for invalid or unknown updates", () => {
  const channels = [makeChannel("design")];

  assert.strictEqual(
    applyChannelLastMessageAt(channels, "design", "not-a-date"),
    channels,
  );
  assert.strictEqual(
    applyChannelLastMessageAt(channels, "unknown", 1_767_225_660),
    channels,
  );
  assert.strictEqual(
    applyChannelLastMessageAt(undefined, "design", 1_767_225_660),
    undefined,
  );
});

function mergeRecency(start, displayed, refreshed) {
  return mergeConcurrentChannelRecency(
    [makeChannel("general", refreshed)],
    [makeChannel("general", displayed)],
    [makeChannel("general", start)],
  )[0];
}

test("mergeConcurrentChannelRecency preserves a newer live timestamp", () => {
  const result = mergeRecency(
    "2026-01-01T00:00:00Z",
    "2026-01-01T00:02:00Z",
    "2026-01-01T00:01:00Z",
  );
  assert.equal(result.lastMessageAt, "2026-01-01T00:02:00Z");
});

test("mergeConcurrentChannelRecency preserves monotonic and absence semantics", () => {
  assert.equal(
    mergeRecency(
      "2026-01-01T00:02:00Z",
      "2026-01-01T00:02:00Z",
      "2026-01-01T00:01:00Z",
    ).lastMessageAt,
    "2026-01-01T00:02:00Z",
  );
  assert.equal(
    mergeRecency("2026-01-01T00:01:00Z", "2026-01-01T00:01:00Z", null)
      .lastMessageAt,
    null,
  );
  assert.equal(
    mergeRecency(
      "2026-01-01T00:00:00Z",
      "2026-01-01T00:01:00Z",
      "2026-01-01T00:02:00Z",
    ).lastMessageAt,
    "2026-01-01T00:02:00Z",
  );
});
