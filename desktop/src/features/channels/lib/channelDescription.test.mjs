import assert from "node:assert/strict";
import test from "node:test";

import {
  getChannelDescription,
  getChannelDetail,
} from "./channelDescription.ts";

function makeChannel(overrides = {}) {
  return {
    archivedAt: null,
    description: "",
    isMember: true,
    purpose: "",
    topic: "",
    ...overrides,
  };
}

test("getChannelDescription falls back when channel is null", () => {
  assert.equal(
    getChannelDescription(null),
    "Connect to the relay to browse channels and read messages.",
  );
});

test("getChannelDescription falls back when no detail fields are set", () => {
  assert.equal(
    getChannelDescription(makeChannel()),
    "Channel details and activity.",
  );
});

test("getChannelDetail provides one shared field order for every surface", () => {
  const channel = makeChannel({
    description: "Description paragraphs.\n\nKeep this structure.",
    purpose: "Legacy purpose",
    topic: "",
  });

  assert.equal(
    getChannelDetail(channel),
    "Description paragraphs.\n\nKeep this structure.",
  );
  assert.equal(getChannelDescription(channel), getChannelDetail(channel));
});

test("getChannelDescription returns single-line detail unchanged", () => {
  assert.equal(
    getChannelDescription(makeChannel({ description: "Team updates." })),
    "Team updates.",
  );
});

test("getChannelDescription preserves paragraph line breaks (AIDA-1980)", () => {
  const detail = "First paragraph.\n\nSecond paragraph with instructions.";
  assert.equal(
    getChannelDescription(makeChannel({ description: detail })),
    detail,
  );
});

test("getChannelDescription puts status prefixes on their own line", () => {
  const detail = "Line one.\nLine two.";
  assert.equal(
    getChannelDescription(
      makeChannel({
        archivedAt: "2026-01-01T00:00:00Z",
        description: detail,
        isMember: false,
      }),
    ),
    `Archived. Read-only until you join this open channel.\n${detail}`,
  );
});

test("getChannelDescription shows prefixes alone when no detail exists", () => {
  assert.equal(
    getChannelDescription(makeChannel({ archivedAt: "2026-01-01T00:00:00Z" })),
    "Archived.",
  );
});
