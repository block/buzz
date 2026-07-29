import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import {
  _testIngestControlResult,
  resetAgentObserverStore,
  subscribeControlResults,
} from "./observerRelayStore.ts";

const AGENT = "a".repeat(64);
const MODEL = "goose-claude-fable-5";
const CHANNEL = "channel-a";
const REQUEST_ID = "a".repeat(32);

function relayEvent(overrides = {}) {
  return {
    id: "1".repeat(64),
    pubkey: AGENT,
    created_at: 1_775_000_001,
    kind: 24_200,
    tags: [
      ["agent", AGENT],
      ["frame", "telemetry"],
    ],
    content: "encrypted",
    sig: "2".repeat(128),
    ...overrides,
  };
}

function observerEvent(overrides = {}) {
  return {
    seq: 7,
    timestamp: "2026-04-01T00:00:01.123Z",
    kind: "control_result",
    agentIndex: 0,
    channelId: CHANNEL,
    sessionId: null,
    turnId: null,
    payload: {
      type: "switch_model",
      status: "switched",
      modelId: MODEL,
      requestId: REQUEST_ID,
    },
    ...overrides,
  };
}

beforeEach(() => {
  resetAgentObserverStore();
});

test("control-result dispatch carries signed relay freshness metadata", () => {
  const received = [];
  const unsubscribe = subscribeControlResults(AGENT, (frame) => {
    received.push(frame);
  });

  _testIngestControlResult(AGENT, relayEvent(), observerEvent());
  unsubscribe();

  assert.deepEqual(received, [
    {
      type: "switch_model",
      status: "switched",
      modelId: MODEL,
      requestId: REQUEST_ID,
      channelId: CHANNEL,
      relayEventId: "1".repeat(64),
      relayCreatedAt: 1_775_000_001,
      observerTimestamp: "2026-04-01T00:00:01.123Z",
      observerSeq: 7,
    },
  ]);
});

test("the authenticated observer envelope channel overrides an inner payload claim", () => {
  const received = [];
  const unsubscribe = subscribeControlResults(AGENT, (frame) => {
    received.push(frame);
  });

  _testIngestControlResult(
    AGENT,
    relayEvent(),
    observerEvent({
      payload: {
        type: "switch_model",
        status: "switched",
        modelId: MODEL,
        channelId: "spoofed-channel",
      },
    }),
  );
  unsubscribe();

  assert.equal(received.length, 1);
  assert.equal(received[0].channelId, CHANNEL);
});

test("a replay republished under a new signed event id cannot redispatch a deduped observer result", () => {
  const received = [];
  const unsubscribe = subscribeControlResults(AGENT, (frame) => {
    received.push(frame);
  });
  const parsed = observerEvent();

  _testIngestControlResult(AGENT, relayEvent(), parsed);
  _testIngestControlResult(
    AGENT,
    relayEvent({ id: "3".repeat(64), created_at: 1_775_000_099 }),
    parsed,
  );
  unsubscribe();

  assert.equal(received.length, 1);
  assert.equal(received[0].relayEventId, "1".repeat(64));
});
