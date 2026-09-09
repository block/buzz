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

  it("falls back to matching turn_started context for older turn_error payloads", () => {
    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event(),
      event({
        seq: 2,
        kind: "turn_error",
        payload: { error: "legacy failure" },
      }),
    ]);

    const [failure] = getRecentAgentTurnFailures("channel-1", ROOT);
    assert.equal(failure.error, "legacy failure");
    assert.equal(failure.disposition, "stopped");
  });
});
