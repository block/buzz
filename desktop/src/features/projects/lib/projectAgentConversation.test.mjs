import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import {
  restoreProjectsAgentConversation,
  visibleConversationMessages,
} from "./projectAgentConversation.ts";
import {
  clearStoredProjectsAgentConversation,
  readStoredProjectsAgentConversation,
  writeStoredProjectsAgentConversation,
} from "./projectAgentConversationStorage.ts";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";

const AGENT_PUBKEY = "a".repeat(64);
const WORKSPACE_ID = "wss://relay.example.com";
// The user opened the Projects prompt at this instant (epoch seconds).
const PROMPT_AT = 1_752_570_000;

const AGENT = { pubkey: AGENT_PUBKEY, name: "Brain" };

/** A pre-existing agent DM channel with plenty of unrelated history. */
const EXISTING_DM = {
  id: "dm-channel-1",
  channelType: "dm",
  participantPubkeys: [AGENT_PUBKEY, "b".repeat(64)],
  lastMessageAt: new Date((PROMPT_AT - 60) * 1_000).toISOString(),
};

function message(createdAt, kind = KIND_STREAM_MESSAGE) {
  return { kind, created_at: createdAt, id: `msg-${kind}-${createdAt}` };
}

const store = new Map();
globalThis.localStorage = {
  getItem: (key) => store.get(key) ?? null,
  setItem: (key, value) => store.set(key, String(value)),
  removeItem: (key) => store.delete(key),
};

beforeEach(() => store.clear());

test("an existing agent DM is never auto-restored without a stored pointer", () => {
  const restored = restoreProjectsAgentConversation({
    stored: null,
    channels: [EXISTING_DM],
    candidates: [AGENT],
  });
  assert.equal(restored, null);
});

test("restores exactly the conversation this feature persisted", () => {
  const restored = restoreProjectsAgentConversation({
    stored: {
      agentPubkey: AGENT_PUBKEY.toUpperCase(),
      channelId: EXISTING_DM.id,
      visibleAfter: PROMPT_AT,
    },
    channels: [EXISTING_DM],
    candidates: [AGENT],
  });
  assert.equal(restored?.channel, EXISTING_DM);
  assert.equal(restored?.agent, AGENT);
  assert.equal(restored?.visibleAfter, PROMPT_AT);
});

test("a zero cutoff pointer is not restorable (would expose full DM history)", () => {
  const restored = restoreProjectsAgentConversation({
    stored: {
      agentPubkey: AGENT_PUBKEY,
      channelId: EXISTING_DM.id,
      visibleAfter: 0,
    },
    channels: [EXISTING_DM],
    candidates: [AGENT],
  });
  assert.equal(restored, null);
});

test("pointers to unknown channels or agents are not restorable", () => {
  const stored = {
    agentPubkey: AGENT_PUBKEY,
    channelId: EXISTING_DM.id,
    visibleAfter: PROMPT_AT,
  };
  assert.equal(
    restoreProjectsAgentConversation({
      stored,
      channels: [],
      candidates: [AGENT],
    }),
    null,
  );
  assert.equal(
    restoreProjectsAgentConversation({
      stored,
      channels: [EXISTING_DM],
      candidates: [],
    }),
    null,
  );
});

test("messages the DM held before the first Projects prompt never appear", () => {
  const olderHistory = [
    message(PROMPT_AT - 86_400),
    message(PROMPT_AT - 3_600, KIND_STREAM_MESSAGE_V2),
    message(PROMPT_AT - 1),
  ];
  const opener = message(PROMPT_AT);
  const reply = message(PROMPT_AT + 5, KIND_STREAM_MESSAGE_V2);
  const nonChatEvent = message(PROMPT_AT + 10, 7);

  const visible = visibleConversationMessages(
    [reply, ...olderHistory, opener, nonChatEvent],
    PROMPT_AT,
  );
  assert.deepEqual(visible, [opener, reply]);
});

test("storage read rejects legacy pointers with a zero cutoff", () => {
  globalThis.localStorage.setItem(
    `buzz.projects.agentConversation.${encodeURIComponent(WORKSPACE_ID)}`,
    JSON.stringify({
      agentPubkey: AGENT_PUBKEY,
      channelId: EXISTING_DM.id,
      visibleAfter: 0,
    }),
  );
  assert.equal(readStoredProjectsAgentConversation(WORKSPACE_ID), null);
});

test("storage round-trips prompt-anchored pointers and clears them", () => {
  const stored = {
    agentPubkey: AGENT_PUBKEY,
    channelId: EXISTING_DM.id,
    visibleAfter: PROMPT_AT,
  };
  writeStoredProjectsAgentConversation(WORKSPACE_ID, stored);
  assert.deepEqual(readStoredProjectsAgentConversation(WORKSPACE_ID), stored);

  clearStoredProjectsAgentConversation(WORKSPACE_ID);
  assert.equal(readStoredProjectsAgentConversation(WORKSPACE_ID), null);
});
