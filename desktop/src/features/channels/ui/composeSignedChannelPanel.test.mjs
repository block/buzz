import assert from "node:assert/strict";
import test from "node:test";

import { composeSignedChannelPanelState } from "./composeSignedChannelPanel.ts";

const CHANNEL_ID = "00000000-0000-4000-8000-000000000001";
const OTHER_CHANNEL_ID = "00000000-0000-4000-8000-000000000002";

function id(hex) {
  return hex.repeat(64);
}

function event(overrides = {}) {
  return {
    id: id("a"),
    pubkey: id("b"),
    created_at: 1_700_000_000,
    kind: 43001,
    tags: [["h", CHANNEL_ID]],
    content: "request",
    sig: id("c"),
    ...overrides,
  };
}

test("composes the latest signed job event and preserves source provenance", () => {
  const state = composeSignedChannelPanelState(CHANNEL_ID, [
    event({ id: id("d"), created_at: 1_700_000_001, kind: 43002 }),
    event({
      id: id("e"),
      created_at: 1_700_000_002,
      kind: 43004,
      content: "done",
    }),
  ]);

  assert.equal(state.kind, "ready");
  assert.equal(state.manifest.status, "complete");
  assert.equal(state.manifest.updatedAt, 1_700_000_002);
  assert.equal(state.manifest.sourceEvents.length, 2);
  assert.equal(state.manifest.sourceEvents[1].label, "Job accepted");
  assert.equal(state.manifest.sections[0].fields[2].value, "Job result");
  assert.equal(state.manifest.sections[0].links[0].sourceEventId, id("e"));
});

test("maps cancellation and failure to explicit bounded statuses", () => {
  const cancelled = composeSignedChannelPanelState(CHANNEL_ID, [
    event({ kind: 43005 }),
  ]);
  const failed = composeSignedChannelPanelState(CHANNEL_ID, [
    event({ kind: 43006 }),
  ]);

  assert.equal(cancelled.kind, "ready");
  assert.equal(cancelled.manifest.status, "blocked");
  assert.equal(failed.kind, "ready");
  assert.equal(failed.manifest.status, "failed");
});

test("ignores unsigned, pending, cross-channel, and non-job events", () => {
  const state = composeSignedChannelPanelState(CHANNEL_ID, [
    event({ id: id("f"), pending: true }),
    event({ id: id("1"), sig: "" }),
    event({ id: id("2"), tags: [["h", OTHER_CHANNEL_ID]] }),
    event({ id: id("3"), kind: 9 }),
  ]);

  assert.equal(state.kind, "empty");
});

test("renders a bounded plain-text note", () => {
  const state = composeSignedChannelPanelState(CHANNEL_ID, [
    event({ content: "line 1\nline 2\u0000" }),
  ]);

  assert.equal(state.kind, "ready");
  assert.equal(state.manifest.sections[0].fields[4].value, "line 1 line 2");
});
