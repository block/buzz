import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import {
  getAgentWorkingState,
  getWorkingAgentPubkeysForChannel,
  getWorkingChannels,
  reportChannelBotTyping,
  resetAgentWorkingSignal,
  subscribeAgentWorkingSignal,
} from "./agentWorkingSignal.ts";
import {
  resetActiveAgentTurnsStore,
  syncAgentTurnsFromEvents,
} from "./activeAgentTurnsStore.ts";

const AGENT =
  "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
const AGENT_2 =
  "dcba4321dcba4321dcba4321dcba4321dcba4321dcba4321dcba4321dcba4321";

function makeEvent(overrides) {
  return {
    seq: 1,
    timestamp: new Date().toISOString(),
    kind: "turn_started",
    agentIndex: 0,
    channelId: "chan-1",
    sessionId: "sess-1",
    turnId: "turn-1",
    payload: null,
    ...overrides,
  };
}

function startTurn(agent, channelId, turnId = `turn-${channelId}`) {
  syncAgentTurnsFromEvents(agent, [
    makeEvent({ channelId, turnId, seq: Math.floor(Math.random() * 1e9) }),
  ]);
}

beforeEach(() => {
  resetActiveAgentTurnsStore();
  resetAgentWorkingSignal();
});

describe("getAgentWorkingState", () => {
  it("is idle with no signals", () => {
    const state = getAgentWorkingState(AGENT);
    assert.equal(state.working, false);
    assert.equal(state.source, "none");
    assert.deepEqual(state.channels, []);
  });

  it("reports observer-backed work, unscoped (all-channels rule)", () => {
    startTurn(AGENT, "chan-1");
    const state = getAgentWorkingState(AGENT);
    assert.equal(state.working, true);
    assert.equal(state.source, "observer");
    assert.deepEqual(
      state.channels.map((c) => [c.channelId, c.source]),
      [["chan-1", "observer"]],
    );
  });

  it("scopes working to the requested channel", () => {
    startTurn(AGENT, "chan-1");
    const inChannel = getAgentWorkingState(AGENT, "chan-1");
    assert.equal(inChannel.working, true);
    assert.equal(inChannel.source, "observer");

    const elsewhere = getAgentWorkingState(AGENT, "chan-2");
    assert.equal(elsewhere.working, false);
    assert.equal(elsewhere.source, "none");
    // The unscoped channel list is still exposed for badges.
    assert.equal(elsewhere.channels.length, 1);
  });

  it("falls back to typing when no observer turns exist", () => {
    reportChannelBotTyping("chan-1", [AGENT]);
    const state = getAgentWorkingState(AGENT, "chan-1");
    assert.equal(state.working, true);
    assert.equal(state.source, "typing");
    assert.equal(state.channels[0].source, "typing");
    assert.ok(state.channels[0].anchorAt <= Date.now());
  });

  it("prefers observer over typing for the same channel (no duplicate)", () => {
    startTurn(AGENT, "chan-1");
    reportChannelBotTyping("chan-1", [AGENT]);
    const state = getAgentWorkingState(AGENT, "chan-1");
    assert.equal(state.source, "observer");
    assert.equal(state.channels.length, 1);
    assert.equal(state.channels[0].source, "observer");
  });

  it("typing in one channel does not mark work in another", () => {
    reportChannelBotTyping("chan-1", [AGENT]);
    const state = getAgentWorkingState(AGENT, "chan-2");
    assert.equal(state.working, false);
  });

  it("typing clears when re-reported empty", () => {
    reportChannelBotTyping("chan-1", [AGENT]);
    reportChannelBotTyping("chan-1", []);
    assert.equal(getAgentWorkingState(AGENT, "chan-1").working, false);
  });

  it("preserves first-seen anchor across typing re-reports", async () => {
    reportChannelBotTyping("chan-1", [AGENT]);
    const first = getAgentWorkingState(AGENT).channels[0].anchorAt;
    await new Promise((resolve) => setTimeout(resolve, 5));
    reportChannelBotTyping("chan-1", [AGENT, AGENT_2]);
    const again = getAgentWorkingState(AGENT).channels[0].anchorAt;
    assert.equal(again, first);
  });
});

describe("getWorkingChannels", () => {
  it("merges typing-only agents into an observer channel summary", () => {
    startTurn(AGENT, "chan-1");
    reportChannelBotTyping("chan-1", [AGENT_2]);
    const channels = getWorkingChannels();
    assert.equal(channels.length, 1);
    assert.equal(channels[0].source, "observer");
    assert.equal(channels[0].agentCount, 2);
    assert.deepEqual(
      new Set(channels[0].agentPubkeys),
      new Set([AGENT, AGENT_2]),
    );
  });

  it("adds typing-only channels with a typing source", () => {
    startTurn(AGENT, "chan-1");
    reportChannelBotTyping("chan-2", [AGENT_2]);
    const channels = getWorkingChannels();
    assert.deepEqual(
      channels.map((c) => [c.channelId, c.source]),
      [
        ["chan-1", "observer"],
        ["chan-2", "typing"],
      ],
    );
  });
});

describe("getWorkingAgentPubkeysForChannel", () => {
  it("unions observer and typing agents for the channel", () => {
    startTurn(AGENT, "chan-1");
    reportChannelBotTyping("chan-1", [AGENT_2]);
    assert.deepEqual(
      new Set(getWorkingAgentPubkeysForChannel("chan-1")),
      new Set([AGENT, AGENT_2]),
    );
    assert.deepEqual(getWorkingAgentPubkeysForChannel("chan-2"), []);
    assert.deepEqual(getWorkingAgentPubkeysForChannel(null), []);
  });
});

describe("subscription and caching", () => {
  it("returns reference-stable snapshots while subscribed", () => {
    startTurn(AGENT, "chan-1");
    const unsubscribe = subscribeAgentWorkingSignal(() => {});
    try {
      assert.equal(
        getAgentWorkingState(AGENT, "chan-1"),
        getAgentWorkingState(AGENT, "chan-1"),
      );
      assert.equal(getWorkingChannels(), getWorkingChannels());
      assert.equal(
        getWorkingAgentPubkeysForChannel("chan-1"),
        getWorkingAgentPubkeysForChannel("chan-1"),
      );
    } finally {
      unsubscribe();
    }
  });

  it("notifies on typing changes but not on identical re-reports", () => {
    let notified = 0;
    const unsubscribe = subscribeAgentWorkingSignal(() => {
      notified += 1;
    });
    try {
      reportChannelBotTyping("chan-1", [AGENT]);
      assert.equal(notified, 1);
      reportChannelBotTyping("chan-1", [AGENT]);
      assert.equal(notified, 1);
      reportChannelBotTyping("chan-1", []);
      assert.equal(notified, 2);
    } finally {
      unsubscribe();
    }
  });

  it("invalidates snapshots when the turns store changes", () => {
    const unsubscribe = subscribeAgentWorkingSignal(() => {});
    try {
      const before = getAgentWorkingState(AGENT, "chan-1");
      assert.equal(before.working, false);
      startTurn(AGENT, "chan-1");
      const after = getAgentWorkingState(AGENT, "chan-1");
      assert.equal(after.working, true);
      assert.equal(after.source, "observer");
    } finally {
      unsubscribe();
    }
  });

  it("resetAgentWorkingSignal clears typing state", () => {
    reportChannelBotTyping("chan-1", [AGENT]);
    resetAgentWorkingSignal();
    assert.equal(getAgentWorkingState(AGENT, "chan-1").working, false);
  });
});
