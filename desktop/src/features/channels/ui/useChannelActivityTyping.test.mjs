import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import {
  getAgentWorkingState,
  getWorkingAgentPubkeysForChannel,
  reportChannelBotTyping,
  resetAgentWorkingSignal,
} from "../../agents/agentWorkingSignal.ts";
import { resetActiveAgentTurnsStore } from "../../agents/activeAgentTurnsStore.ts";
import { channelScopedBotTypingPubkeyKey } from "./useChannelActivityTyping.ts";

const AGENT =
  "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
const AGENT_2 =
  "dcba4321dcba4321dcba4321dcba4321dcba4321dcba4321dcba4321dcba4321";

describe("channelScopedBotTypingPubkeyKey", () => {
  it("excludes thread-scoped typing entries", () => {
    const key = channelScopedBotTypingPubkeyKey([
      { pubkey: AGENT, threadHeadId: "thread-1" },
    ]);
    assert.equal(key, "");
  });

  it("keeps channel-scoped entries and drops thread-scoped ones", () => {
    const key = channelScopedBotTypingPubkeyKey([
      { pubkey: AGENT, threadHeadId: "thread-1" },
      { pubkey: AGENT_2, threadHeadId: null },
    ]);
    assert.equal(key, AGENT_2);
  });

  it("sorts and lowercases channel-scoped pubkeys", () => {
    const key = channelScopedBotTypingPubkeyKey([
      { pubkey: AGENT_2.toUpperCase(), threadHeadId: null },
      { pubkey: AGENT, threadHeadId: null },
    ]);
    assert.equal(key, `${AGENT},${AGENT_2}`);
  });
});

describe("thread-only bot typing regression", () => {
  beforeEach(() => {
    resetActiveAgentTurnsStore();
    resetAgentWorkingSignal();
  });

  it("does not mark channel-level working", () => {
    // The mirror effect reports only the channel-scoped key; thread-only
    // typing produces an empty key, so nothing reaches the working signal.
    const key = channelScopedBotTypingPubkeyKey([
      { pubkey: AGENT, threadHeadId: "thread-1" },
    ]);
    reportChannelBotTyping("chan-1", key ? key.split(",") : []);

    assert.deepEqual(getWorkingAgentPubkeysForChannel("chan-1"), []);
    const state = getAgentWorkingState(AGENT, "chan-1");
    assert.equal(state.working, false);
    assert.equal(state.source, "none");
  });

  it("still marks channel-level working for channel-scoped typing", () => {
    const key = channelScopedBotTypingPubkeyKey([
      { pubkey: AGENT, threadHeadId: null },
      { pubkey: AGENT_2, threadHeadId: "thread-1" },
    ]);
    reportChannelBotTyping("chan-1", key ? key.split(",") : []);

    assert.deepEqual(getWorkingAgentPubkeysForChannel("chan-1"), [AGENT]);
    const state = getAgentWorkingState(AGENT, "chan-1");
    assert.equal(state.working, true);
    assert.equal(state.source, "typing");
  });
});
