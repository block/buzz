import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import {
  createRecentAgentTurnFailuresObserverListener,
  getRecentAgentTurnFailures,
  resetRecentAgentTurnFailuresStore,
  syncRecentAgentTurnFailuresFromEvents,
} from "./recentAgentTurnFailuresStore.ts";

const AGENT = "a".repeat(64);
const TRIGGER = "1".repeat(64);
const ROOT = "2".repeat(64);
const TOP_LEVEL = "3".repeat(64);

function event(overrides = {}) {
  return {
    seq: 1,
    timestamp: "2026-09-09T17:49:05Z",
    kind: "turn_started",
    agentIndex: 0,
    channelId: "channel-1",
    sessionId: "session-1",
    turnId: "turn-1",
    payload: {
      triggeringEventIds: [TRIGGER],
      triggeringRootEventId: ROOT,
      triggeringParentEventId: TRIGGER,
    },
    ...overrides,
  };
}

describe("recentAgentTurnFailuresStore", () => {
  beforeEach(resetRecentAgentTurnFailuresStore);

  it("retains a friendly retrying failure in its originating conversation", () => {
    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event(),
      event({
        seq: 2,
        timestamp: "2026-09-09T17:49:09Z",
        kind: "turn_error",
        payload: {
          error: "Agent reported error (code -32603): Internal error",
          code: -32603,
          disposition: "retrying",
          attempt: 1,
          triggeringEventIds: [TRIGGER],
          triggeringRootEventId: ROOT,
          triggeringParentEventId: TRIGGER,
        },
      }),
    ]);

    const failures = getRecentAgentTurnFailures("channel-1", ROOT);
    assert.equal(failures.length, 1);
    assert.equal(failures[0].disposition, "retrying");
    assert.equal(failures[0].attempt, 1);
    assert.equal(failures[0].rootEventId, ROOT);
    assert.match(failures[0].error, /upgrade the adapter/);
    assert.equal(getRecentAgentTurnFailures("channel-1", TRIGGER).length, 0);
  });

  it("updates one conversation failure instead of accumulating retry spam", () => {
    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event(),
      event({
        seq: 2,
        kind: "turn_error",
        payload: {
          error: "Internal error",
          code: -32603,
          disposition: "retrying",
          attempt: 1,
          triggeringRootEventId: ROOT,
          triggeringParentEventId: TRIGGER,
        },
      }),
      event({
        seq: 3,
        timestamp: "2026-09-09T17:50:09Z",
        kind: "turn_error",
        turnId: "turn-2",
        payload: {
          error: "Internal error",
          code: -32603,
          disposition: "dead_lettered",
          attempt: 11,
          triggeringRootEventId: ROOT,
          triggeringParentEventId: TRIGGER,
        },
      }),
    ]);

    const failures = getRecentAgentTurnFailures("channel-1", ROOT);
    assert.equal(failures.length, 1);
    assert.equal(failures[0].disposition, "dead_lettered");
    assert.equal(failures[0].attempt, 11);
    assert.equal(failures[0].turnId, "turn-2");
  });

  it("clears the retained failure when the conversation starts a new turn", () => {
    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event(),
      event({
        seq: 2,
        kind: "turn_error",
        payload: {
          error: "Internal error",
          disposition: "retrying",
          triggeringRootEventId: ROOT,
          triggeringParentEventId: TRIGGER,
        },
      }),
    ]);
    assert.equal(getRecentAgentTurnFailures("channel-1", ROOT).length, 1);

    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event({
        seq: 3,
        timestamp: "2026-09-09T17:50:09Z",
        turnId: "turn-2",
      }),
    ]);
    assert.equal(getRecentAgentTurnFailures("channel-1", ROOT).length, 0);
  });

  it("keeps threaded failures out of the main composer scope", () => {
    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event({
        seq: 2,
        kind: "turn_error",
        payload: {
          error: "thread failure",
          triggeringEventIds: [TRIGGER],
          triggeringRootEventId: ROOT,
          triggeringParentEventId: TRIGGER,
        },
      }),
      event({
        seq: 3,
        timestamp: "2026-09-09T17:50:09Z",
        kind: "turn_error",
        turnId: "turn-2",
        payload: {
          error: "top-level failure",
          triggeringEventIds: [TOP_LEVEL],
          triggeringRootEventId: TOP_LEVEL,
          triggeringParentEventId: TOP_LEVEL,
        },
      }),
    ]);

    assert.equal(
      getRecentAgentTurnFailures("channel-1")[0]?.error,
      "top-level failure",
    );
    assert.equal(
      getRecentAgentTurnFailures("channel-1", ROOT)[0]?.error,
      "thread failure",
    );
  });

  it("processes only observer updates for an active agent", () => {
    const listener = createRecentAgentTurnFailuresObserverListener([
      { pubkey: AGENT, status: "running" },
      { pubkey: "b".repeat(64), status: "stopped" },
    ]);
    const failureEvent = event({
      seq: 2,
      kind: "turn_error",
      payload: {
        error: "live failure",
        triggeringRootEventId: ROOT,
        triggeringParentEventId: TRIGGER,
      },
    });

    listener({ agentPubkey: "b".repeat(64), events: [failureEvent] });
    assert.equal(getRecentAgentTurnFailures("channel-1", ROOT).length, 0);

    listener({ agentPubkey: AGENT, events: [event(), failureEvent] });
    assert.equal(
      getRecentAgentTurnFailures("channel-1", ROOT)[0]?.error,
      "live failure",
    );
  });

  it("keeps actual legacy source-and-ID-only context unknown and visible", () => {
    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event({ payload: { source: "mention", triggeringEventIds: [TRIGGER] } }),
      event({
        seq: 2,
        kind: "turn_error",
        payload: { error: "legacy failure" },
      }),
    ]);

    const [failure] = getRecentAgentTurnFailures("channel-1");
    assert.equal(failure.error, "legacy failure");
    assert.equal(failure.rootEventId, null);
    assert.equal(failure.parentEventId, null);
    assert.equal(failure.disposition, "unknown");
    assert.equal(getRecentAgentTurnFailures("channel-1", ROOT).length, 0);
    assert.equal(getRecentAgentTurnFailures("channel-1", TRIGGER).length, 0);
  });
});

describe("retry batch coverage through observer listener", () => {
  beforeEach(resetRecentAgentTurnFailuresStore);
  it("clears A when retrying [A,B] anchored at B, preserving partial and unrelated failures", () => {
    const listener = createRecentAgentTurnFailuresObserverListener([
      { pubkey: AGENT, status: "running" },
    ]);
    const failed = (seq, ids, root, overrides = {}) =>
      event({
        seq,
        kind: "turn_error",
        turnId: `failed-${seq}`,
        payload: {
          error: "failed",
          disposition: "retrying",
          triggeringEventIds: ids,
          triggeringRootEventId: root,
          ...overrides,
        },
      });
    listener({
      agentPubkey: AGENT,
      events: [
        failed(1, ["A"], "A"),
        failed(2, ["A", "C"], "C"),
        failed(3, ["D"], "D"),
        { ...failed(4, ["A"], "A"), channelId: "other-channel" },
        event({
          seq: 5,
          turnId: "retry-ab",
          payload: {
            triggeringEventIds: ["A", "B"],
            triggeringRootEventId: "B",
            triggeringParentEventId: "B",
          },
        }),
      ],
    });
    assert.equal(getRecentAgentTurnFailures("channel-1", "A").length, 0);
    assert.equal(getRecentAgentTurnFailures("channel-1", "C").length, 1);
    assert.equal(getRecentAgentTurnFailures("channel-1", "D").length, 1);
    assert.equal(getRecentAgentTurnFailures("other-channel", "A").length, 1);
  });

  it("preserves full started batch when an error only reports its root", () => {
    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event({
        payload: { triggeringEventIds: ["A", "B"], triggeringRootEventId: "B" },
      }),
      event({
        seq: 2,
        kind: "turn_error",
        payload: { error: "failed", triggeringRootEventId: "B" },
      }),
      event({
        seq: 3,
        turnId: "partial-retry",
        payload: { triggeringEventIds: ["B"], triggeringRootEventId: "B" },
      }),
    ]);
    assert.deepEqual(
      getRecentAgentTurnFailures("channel-1", "B")[0].triggeringEventIds,
      ["A", "B"],
    );
  });
});
