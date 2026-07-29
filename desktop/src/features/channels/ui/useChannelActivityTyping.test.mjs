import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import {
  getAgentWorkingState,
  getWorkingAgentPubkeysForChannel,
  reportChannelBotTyping,
  resetAgentWorkingSignal,
} from "../../agents/agentWorkingSignal.ts";
import { resetActiveAgentTurnsStore } from "../../agents/activeAgentTurnsStore.ts";
import { PRESET_LOGOS } from "../../onboarding/ui/RuntimeIcon.tsx";
import {
  channelScopedBotTypingPubkeyKey,
  mergeAgentNamesIntoProfiles,
  mergeMemberAgentFlagsIntoProfiles,
} from "./useChannelActivityTyping.ts";

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

describe("mergeAgentNamesIntoProfiles", () => {
  function managedAgent(overrides = {}) {
    return {
      pubkey: AGENT,
      name: "Marshall",
      avatarUrl: null,
      backend: { type: "provider", id: "ssh", config: { ssh_host: "vps" } },
      agentCommand: "hermes",
      agentArgs: ["--profile", "marshall", "acp"],
      ...overrides,
    };
  }

  it("gives a remote agent its harness mark in the timeline", () => {
    // The host's catalog entry carries no image on purpose, so nothing ever
    // stamped this record and every message it sent rendered blank initials.
    const merged = mergeAgentNamesIntoProfiles({}, [managedAgent()], []);

    assert.equal(merged[AGENT]?.avatarUrl, PRESET_LOGOS.hermes);
  });

  it("leaves a local agent's stamped avatar exactly as it was", () => {
    const merged = mergeAgentNamesIntoProfiles(
      {},
      [
        managedAgent({
          avatarUrl: "app-avatar://claude",
          backend: { type: "local" },
          agentCommand: "claude-agent-acp",
          agentArgs: [],
        }),
      ],
      [],
    );

    assert.equal(merged[AGENT]?.avatarUrl, "app-avatar://claude");
  });

  it("keeps a published profile avatar ahead of the harness mark", () => {
    const merged = mergeAgentNamesIntoProfiles(
      {
        [AGENT]: {
          displayName: "Marshall",
          avatarUrl: "https://example.com/published.png",
          nip05Handle: null,
          ownerPubkey: null,
        },
      },
      [managedAgent()],
      [],
    );

    assert.equal(merged[AGENT]?.avatarUrl, "https://example.com/published.png");
  });
});

describe("mergeMemberAgentFlagsIntoProfiles", () => {
  it("flags a role-only bot member (no isAgent) as an agent", () => {
    const merged = mergeMemberAgentFlagsIntoProfiles({}, [
      { pubkey: AGENT, role: "bot", isAgent: false },
    ]);

    assert.equal(merged[AGENT]?.isAgent, true);
    assert.equal(merged[AGENT]?.displayName, null);
    assert.equal(merged[AGENT]?.avatarUrl, null);
    assert.equal(merged[AGENT]?.nip05Handle, null);
    assert.equal(merged[AGENT]?.ownerPubkey, null);
  });

  it("flags an isAgent-only member (non-bot role) as an agent", () => {
    const merged = mergeMemberAgentFlagsIntoProfiles({}, [
      { pubkey: AGENT, role: "member", isAgent: true },
    ]);

    assert.equal(merged[AGENT]?.isAgent, true);
  });

  it("normalises member pubkeys so lookups by normalised key hit", () => {
    const merged = mergeMemberAgentFlagsIntoProfiles({}, [
      { pubkey: ` ${AGENT.toUpperCase()}`, role: "bot", isAgent: false },
    ]);

    assert.equal(merged[AGENT]?.isAgent, true);
  });

  it("returns the same reference when no member carries an agent flag", () => {
    const profiles = {
      [AGENT]: {
        displayName: "Human",
        avatarUrl: null,
        nip05Handle: null,
        ownerPubkey: null,
      },
    };

    assert.equal(
      mergeMemberAgentFlagsIntoProfiles(profiles, undefined),
      profiles,
    );
    assert.equal(mergeMemberAgentFlagsIntoProfiles(profiles, []), profiles);
    assert.equal(
      mergeMemberAgentFlagsIntoProfiles(profiles, [
        { pubkey: AGENT_2, role: "member", isAgent: false },
      ]),
      profiles,
    );
  });

  it("preserves existing profile fields and does not mutate the input", () => {
    const profiles = {
      [AGENT]: {
        displayName: "Deploy Bot",
        name: "deploybot",
        avatarUrl: "https://example.com/a.png",
        nip05Handle: "deploy@example.com",
        ownerPubkey: AGENT_2,
      },
    };

    const merged = mergeMemberAgentFlagsIntoProfiles(profiles, [
      { pubkey: AGENT, role: "bot", isAgent: false },
    ]);

    assert.notEqual(merged, profiles);
    assert.deepEqual(merged[AGENT], {
      displayName: "Deploy Bot",
      name: "deploybot",
      avatarUrl: "https://example.com/a.png",
      nip05Handle: "deploy@example.com",
      ownerPubkey: AGENT_2,
      isAgent: true,
    });
    assert.equal(profiles[AGENT].isAgent, undefined);
  });
});
