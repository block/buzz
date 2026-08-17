import assert from "node:assert/strict";
import { describe, it, beforeEach, afterEach } from "node:test";

import {
  circuitCooldownRemainingMs,
  getAgentCircuitStatus,
  getOpenCircuitPubkeySignature,
  resetAgentObserverStore,
  subscribeAgentObserverStore,
  _testProcessLiveObserverEvents,
} from "./observerRelayStore.ts";

const AGENT =
  "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
const AGENT_2 =
  "dcba4321dcba4321dcba4321dcba4321dcba4321dcba4321dcba4321dcba4321";

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
    assert.equal(status.cooldownSecs, null);
  });

  it("flips isOpen true on a circuit_open event, surfacing its message/channel/timestamp/cooldown", () => {
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
          cooldown_secs: 300,
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
    assert.equal(status.cooldownSecs, 300);
  });

  it("leaves cooldownSecs null when the payload doesn't carry a numeric cooldown_secs", () => {
    _testProcessLiveObserverEvents(AGENT, [
      makeEvent({
        seq: 1,
        timestamp: "2024-01-01T00:00:00Z",
        kind: "circuit_open",
        agentIndex: 0,
        channelId: "chan-uuid-1",
        payload: { error: "opened", cooldown_secs: "not-a-number" },
      }),
    ]);

    assert.equal(getAgentCircuitStatus(AGENT).cooldownSecs, null);
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

  it("notifies subscribers on a circuit change even when appendAgentEvents rejects the raw event as a dedup collision", () => {
    // appendAgentEvents dedups purely on (timestamp, seq), independent of
    // kind — so a circuit_open event that happens to share its (timestamp,
    // seq) with an already-appended, unrelated event is treated as a
    // duplicate by the raw journal (appendAgentEvents returns false) even
    // though it is the first-ever circuit event for its own slot, so
    // applyCircuitEvent legitimately applies it (returns true). Regression
    // guard for the case where only appendAgentEvents' return value gated
    // notifyListeners(): the circuit state would update silently with no
    // subscriber ever told, so useAgentCircuitStatus consumers (the
    // suspended-agent badge) would not re-render until unrelated traffic
    // happened to touch the same agent.
    _testProcessLiveObserverEvents(AGENT, [
      makeEvent({
        seq: 7,
        timestamp: "2024-01-01T00:00:00Z",
        kind: "acp_read",
        agentIndex: null,
        channelId: "chan-uuid-1",
        payload: {},
      }),
    ]);

    let notifyCount = 0;
    const unsubscribe = subscribeAgentObserverStore(() => {
      notifyCount += 1;
    });
    try {
      _testProcessLiveObserverEvents(AGENT, [
        makeEvent({
          seq: 7,
          timestamp: "2024-01-01T00:00:00Z",
          kind: "circuit_open",
          agentIndex: 2,
          channelId: "chan-uuid-2",
          payload: { error: "slot 2 opened" },
        }),
      ]);
    } finally {
      unsubscribe();
    }

    assert.equal(
      notifyCount,
      1,
      "a circuit-only state change must still notify useSyncExternalStore subscribers",
    );
    assert.equal(getAgentCircuitStatus(AGENT).isOpen, true);
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

describe("circuitCooldownRemainingMs", () => {
  const OPEN_STATUS = {
    isOpen: true,
    message: "opened",
    channelId: "chan-1",
    timestamp: "2024-01-01T00:00:00.000Z",
    cooldownSecs: 300,
  };
  const OPENED_AT_MS = Date.parse(OPEN_STATUS.timestamp);

  it("returns the full window right when the circuit opens", () => {
    assert.equal(
      circuitCooldownRemainingMs(OPEN_STATUS, OPENED_AT_MS),
      300_000,
    );
  });

  it("counts down as time passes", () => {
    assert.equal(
      circuitCooldownRemainingMs(OPEN_STATUS, OPENED_AT_MS + 120_000),
      180_000,
    );
  });

  it("clamps to 0 once the cooldown window has elapsed, never negative", () => {
    assert.equal(
      circuitCooldownRemainingMs(OPEN_STATUS, OPENED_AT_MS + 300_000),
      0,
    );
    assert.equal(
      circuitCooldownRemainingMs(OPEN_STATUS, OPENED_AT_MS + 999_000),
      0,
    );
  });

  it("returns null when the circuit isn't open", () => {
    assert.equal(
      circuitCooldownRemainingMs(
        { ...OPEN_STATUS, isOpen: false },
        OPENED_AT_MS,
      ),
      null,
    );
  });

  it("returns null when cooldownSecs is unknown", () => {
    assert.equal(
      circuitCooldownRemainingMs(
        { ...OPEN_STATUS, cooldownSecs: null },
        OPENED_AT_MS,
      ),
      null,
    );
  });

  it("returns null when timestamp is unknown or unparseable", () => {
    assert.equal(
      circuitCooldownRemainingMs(
        { ...OPEN_STATUS, timestamp: null },
        OPENED_AT_MS,
      ),
      null,
    );
    assert.equal(
      circuitCooldownRemainingMs(
        { ...OPEN_STATUS, timestamp: "not-a-date" },
        OPENED_AT_MS,
      ),
      null,
    );
  });
});

describe("getOpenCircuitPubkeySignature", () => {
  beforeEach(() => {
    resetAgentObserverStore();
  });

  afterEach(() => {
    resetAgentObserverStore();
  });

  it("returns an empty string when none of the candidates are open", () => {
    assert.equal(getOpenCircuitPubkeySignature([AGENT, AGENT_2]), "");
  });

  it("includes only the open agent's normalized pubkey", () => {
    _testProcessLiveObserverEvents(AGENT, [
      makeEvent({
        seq: 1,
        timestamp: "2024-01-01T00:00:00Z",
        kind: "circuit_open",
        agentIndex: 0,
        payload: { error: "opened" },
      }),
    ]);

    assert.equal(
      getOpenCircuitPubkeySignature([AGENT, AGENT_2]),
      AGENT.toLowerCase(),
    );
  });

  it("sorts multiple open pubkeys so the signature is stable regardless of input order", () => {
    for (const [agent, index] of [
      [AGENT, 0],
      [AGENT_2, 0],
    ]) {
      _testProcessLiveObserverEvents(agent, [
        makeEvent({
          seq: 1,
          timestamp: "2024-01-01T00:00:00Z",
          kind: "circuit_open",
          agentIndex: index,
          payload: { error: "opened" },
        }),
      ]);
    }

    const expected = [AGENT, AGENT_2]
      .map((a) => a.toLowerCase())
      .sort()
      .join(",");
    assert.equal(getOpenCircuitPubkeySignature([AGENT, AGENT_2]), expected);
    assert.equal(getOpenCircuitPubkeySignature([AGENT_2, AGENT]), expected);
  });

  it("excludes candidates whose circuit has recovered", () => {
    _testProcessLiveObserverEvents(AGENT, [
      makeEvent({
        seq: 1,
        timestamp: "2024-01-01T00:00:00Z",
        kind: "circuit_open",
        agentIndex: 0,
        payload: { error: "opened" },
      }),
      makeEvent({
        seq: 2,
        timestamp: "2024-01-01T00:01:00Z",
        kind: "circuit_recovered",
        agentIndex: 0,
        payload: { error: "recovered" },
      }),
    ]);

    assert.equal(getOpenCircuitPubkeySignature([AGENT, AGENT_2]), "");
  });
});
