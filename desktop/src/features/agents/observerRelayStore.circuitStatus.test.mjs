import assert from "node:assert/strict";
import { describe, it, beforeEach, afterEach } from "node:test";

import {
  getAgentCircuitStatus,
  resetAgentObserverStore,
  _testProcessLiveObserverEvents,
} from "./observerRelayStore.ts";

const AGENT =
  "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";

function makeEvent(overrides) {
  return {
    seq: 1,
    timestamp: "2024-01-01T00:00:00Z",
    kind: "circuit_open",
    agentIndex: 0,
    channelId: "chan-1",
    sessionId: null,
    turnId: null,
    payload: null,
    ...overrides,
  };
}

describe("getAgentCircuitStatus", () => {
  beforeEach(() => {
    resetAgentObserverStore();
  });

  afterEach(() => {
    resetAgentObserverStore();
  });

  it("reports isOpen false when no circuit events have been seen for the agent", () => {
    const status = getAgentCircuitStatus(AGENT);
    assert.equal(status.isOpen, false);
    assert.equal(status.message, null);
    assert.equal(status.channelId, null);
    assert.equal(status.timestamp, null);
  });

  it("flips isOpen true on a circuit_open event, surfacing its message/channel/timestamp", () => {
    _testProcessLiveObserverEvents(AGENT, [
      makeEvent({
        seq: 1,
        timestamp: "2024-01-01T00:00:00Z",
        kind: "circuit_open",
        agentIndex: 0,
        channelId: "chan-uuid-1",
        payload: {
          error:
            "Agent slot 0 panicked repeatedly and its circuit breaker is now open.",
        },
      }),
    ]);

    const status = getAgentCircuitStatus(AGENT);
    assert.equal(status.isOpen, true);
    assert.equal(
      status.message,
      "Agent slot 0 panicked repeatedly and its circuit breaker is now open.",
    );
    assert.equal(status.channelId, "chan-uuid-1");
    assert.equal(status.timestamp, "2024-01-01T00:00:00Z");
  });

  it("flips isOpen back to false when a newer circuit_recovered event arrives on the same slot", () => {
    _testProcessLiveObserverEvents(AGENT, [
      makeEvent({
        seq: 1,
        timestamp: "2024-01-01T00:00:00Z",
        kind: "circuit_open",
        agentIndex: 0,
        channelId: "chan-uuid-1",
        payload: { error: "opened" },
      }),
    ]);
    assert.equal(getAgentCircuitStatus(AGENT).isOpen, true);

    _testProcessLiveObserverEvents(AGENT, [
      makeEvent({
        seq: 2,
        timestamp: "2024-01-01T00:01:00Z",
        kind: "circuit_recovered",
        agentIndex: 0,
        channelId: "chan-uuid-1",
        payload: { error: "recovered" },
      }),
    ]);

    const status = getAgentCircuitStatus(AGENT);
    assert.equal(status.isOpen, false);
    assert.equal(status.message, null);
    assert.equal(status.channelId, null);
    assert.equal(status.timestamp, null);
  });

  it("reports isOpen true overall (any-slot-open) with two independent slots, one open and one recovered", () => {
    // Slot 0 stays open.
    _testProcessLiveObserverEvents(AGENT, [
      makeEvent({
        seq: 1,
        timestamp: "2024-01-01T00:00:00Z",
        kind: "circuit_open",
        agentIndex: 0,
        channelId: "chan-slot-0",
        payload: { error: "slot 0 opened" },
      }),
    ]);
    // Slot 1 opens then recovers.
    _testProcessLiveObserverEvents(AGENT, [
      makeEvent({
        seq: 2,
        timestamp: "2024-01-01T00:00:01Z",
        kind: "circuit_open",
        agentIndex: 1,
        channelId: "chan-slot-1",
        payload: { error: "slot 1 opened" },
      }),
      makeEvent({
        seq: 3,
        timestamp: "2024-01-01T00:00:02Z",
        kind: "circuit_recovered",
        agentIndex: 1,
        channelId: "chan-slot-1",
        payload: { error: "slot 1 recovered" },
      }),
    ]);

    const status = getAgentCircuitStatus(AGENT);
    assert.equal(
      status.isOpen,
      true,
      "any slot still open must report the agent as open",
    );
    assert.equal(status.channelId, "chan-slot-0");
    assert.equal(status.message, "slot 0 opened");
  });

  it("ignores a stale/out-of-order event arriving after a recovery", () => {
    _testProcessLiveObserverEvents(AGENT, [
      makeEvent({
        seq: 1,
        timestamp: "2024-01-01T00:00:00Z",
        kind: "circuit_open",
        agentIndex: 0,
        channelId: "chan-uuid-1",
        payload: { error: "opened" },
      }),
      makeEvent({
        seq: 2,
        timestamp: "2024-01-01T00:01:00Z",
        kind: "circuit_recovered",
        agentIndex: 0,
        channelId: "chan-uuid-1",
        payload: { error: "recovered" },
      }),
    ]);
    assert.equal(getAgentCircuitStatus(AGENT).isOpen, false);

    // A delayed circuit_open with an OLDER timestamp+seq than the currently
    // stored (recovered) state must be ignored — it must not reopen the
    // circuit or otherwise change status.
    _testProcessLiveObserverEvents(AGENT, [
      makeEvent({
        seq: 1,
        timestamp: "2024-01-01T00:00:00Z",
        kind: "circuit_open",
        agentIndex: 0,
        channelId: "chan-uuid-1",
        payload: { error: "stale reopen" },
      }),
    ]);

    const status = getAgentCircuitStatus(AGENT);
    assert.equal(
      status.isOpen,
      false,
      "a stale out-of-order event must not regress an already-newer status",
    );
    assert.equal(status.message, null);
    assert.equal(status.channelId, null);
    assert.equal(status.timestamp, null);
  });

  it("returns a stable reference across repeated calls with no intervening event", () => {
    _testProcessLiveObserverEvents(AGENT, [
      makeEvent({
        seq: 1,
        timestamp: "2024-01-01T00:00:00Z",
        kind: "circuit_open",
        agentIndex: 0,
        channelId: "chan-uuid-1",
        payload: { error: "opened" },
      }),
    ]);

    const ref1 = getAgentCircuitStatus(AGENT);
    const ref2 = getAgentCircuitStatus(AGENT);
    assert.strictEqual(
      ref1,
      ref2,
      "must return the cached object reference when nothing changed — a " +
        "regression here would cause an infinite useSyncExternalStore render loop",
    );
  });

  it("resetAgentObserverStore clears circuit state back to isOpen:false", () => {
    _testProcessLiveObserverEvents(AGENT, [
      makeEvent({
        seq: 1,
        timestamp: "2024-01-01T00:00:00Z",
        kind: "circuit_open",
        agentIndex: 0,
        channelId: "chan-uuid-1",
        payload: { error: "opened" },
      }),
    ]);
    assert.equal(getAgentCircuitStatus(AGENT).isOpen, true);

    resetAgentObserverStore();

    const status = getAgentCircuitStatus(AGENT);
    assert.equal(status.isOpen, false);
    assert.equal(status.message, null);
    assert.equal(status.channelId, null);
    assert.equal(status.timestamp, null);
  });
});
