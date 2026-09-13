import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import {
  isAtOrAfterConversationOpener,
  mergeProjectAgentConversationEvents,
  projectAgentMembershipInput,
  restoreProjectsAgentConversation,
  submitProjectAgentMessage,
  visibleConversationMessages,
} from "./projectAgentConversation.ts";
import {
  clearStoredProjectsAgentConversation,
  projectsConversationScope,
  readStoredProjectsAgentConversation,
  writeStoredProjectsAgentConversation,
} from "./projectAgentConversationStorage.ts";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";

const AGENT_PUBKEY = "a".repeat(64);
const SELF_PUBKEY = "b".repeat(64);
const WORKSPACE_ID = "wss://relay.example.com";
// The user opened the Projects prompt at this instant (epoch seconds).
const PROMPT_AT = 1_752_570_000;
// The relay-accepted event id of the opening prompt. Within the opener's
// second, the timeline orders by ascending id, so ids <= the opener's are
// at-or-after it and ids > it are older history.
const OPENER = { createdAt: PROMPT_AT, eventId: `d${"0".repeat(63)}` };

const AGENT = { pubkey: AGENT_PUBKEY, name: "Brain" };

/** A pre-existing agent DM channel with plenty of unrelated history. */
const EXISTING_DM = {
  id: "dm-channel-1",
  channelType: "dm",
  participantPubkeys: [AGENT_PUBKEY, SELF_PUBKEY],
  lastMessageAt: new Date((PROMPT_AT - 60) * 1_000).toISOString(),
};

function message(createdAt, kind = KIND_STREAM_MESSAGE, id) {
  return { kind, created_at: createdAt, id: id ?? `msg-${kind}-${createdAt}` };
}

const store = new Map();
globalThis.localStorage = {
  getItem: (key) => store.get(key) ?? null,
  setItem: (key, value) => store.set(key, String(value)),
  removeItem: (key) => store.delete(key),
};

beforeEach(() => store.clear());

test("conversation scopes isolate same-relay identities and restore on return", () => {
  const relay = "wss://relay.example.com";
  const resource = "30617:owner:buzz";
  const identityA = "a".repeat(64);
  const identityB = "b".repeat(64);
  const scopeA = projectsConversationScope(
    "detail",
    relay,
    identityA,
    resource,
  );
  const scopeB = projectsConversationScope(
    "detail",
    relay,
    identityB,
    resource,
  );
  assert.notEqual(scopeA, scopeB);

  const pointer = {
    agentPubkey: AGENT_PUBKEY,
    channelId: EXISTING_DM.id,
    opener: OPENER,
  };
  writeStoredProjectsAgentConversation(scopeA, pointer);
  assert.equal(readStoredProjectsAgentConversation(scopeB), null);
  assert.deepEqual(readStoredProjectsAgentConversation(scopeA), pointer);
});

test("conversation scopes fail closed without relay, signer, or resource", () => {
  assert.equal(
    projectsConversationScope("detail", null, SELF_PUBKEY, "repo"),
    null,
  );
  assert.equal(
    projectsConversationScope("detail", WORKSPACE_ID, null, "repo"),
    null,
  );
  assert.equal(
    projectsConversationScope("detail", WORKSPACE_ID, SELF_PUBKEY, ""),
    null,
  );
});

test("an existing agent DM is never auto-restored without a stored pointer", () => {
  const restored = restoreProjectsAgentConversation({
    stored: null,
    channels: [EXISTING_DM],
    candidates: [AGENT],
    currentPubkey: SELF_PUBKEY,
  });
  assert.equal(restored, null);
});

test("restores exactly the conversation this feature persisted", () => {
  const restored = restoreProjectsAgentConversation({
    stored: {
      agentPubkey: AGENT_PUBKEY.toUpperCase(),
      channelId: EXISTING_DM.id,
      opener: OPENER,
    },
    channels: [EXISTING_DM],
    candidates: [AGENT],
    currentPubkey: SELF_PUBKEY,
  });
  assert.equal(restored?.channel, EXISTING_DM);
  assert.equal(restored?.agent, AGENT);
  assert.deepEqual(restored?.opener, OPENER);
});

test("pointers to unknown channels or agents are not restorable", () => {
  const stored = {
    agentPubkey: AGENT_PUBKEY,
    channelId: EXISTING_DM.id,
    opener: OPENER,
  };
  assert.equal(
    restoreProjectsAgentConversation({
      stored,
      channels: [],
      candidates: [AGENT],
      currentPubkey: SELF_PUBKEY,
    }),
    null,
  );
  assert.equal(
    restoreProjectsAgentConversation({
      stored,
      channels: [EXISTING_DM],
      candidates: [],
      currentPubkey: SELF_PUBKEY,
    }),
    null,
  );
});

test("a stored project-channel pointer restores when it matches the home channel", () => {
  const home = {
    id: "project-channel-1",
    channelType: "stream",
    isMember: true,
    memberPubkeys: [SELF_PUBKEY, AGENT_PUBKEY],
    participantPubkeys: [],
  };
  const restored = restoreProjectsAgentConversation({
    stored: {
      agentPubkey: AGENT_PUBKEY,
      channelId: home.id,
      opener: OPENER,
    },
    channels: [home],
    candidates: [AGENT],
    currentPubkey: SELF_PUBKEY,
    homeChannelId: home.id,
  });
  assert.equal(restored?.channel, home);
  assert.equal(restored?.agent, AGENT);
});

test("a stored project-channel pointer does not restore a different home", () => {
  const home = {
    id: "project-channel-1",
    channelType: "stream",
    isMember: true,
    memberPubkeys: [SELF_PUBKEY],
    participantPubkeys: [],
  };
  assert.equal(
    restoreProjectsAgentConversation({
      stored: {
        agentPubkey: AGENT_PUBKEY,
        channelId: home.id,
        opener: OPENER,
      },
      channels: [home],
      candidates: [AGENT],
      currentPubkey: SELF_PUBKEY,
      homeChannelId: "other-project-channel",
    }),
    null,
  );
});

test("a pointer naming a non-DM or foreign-participant channel is not restorable", () => {
  const stored = {
    agentPubkey: AGENT_PUBKEY,
    channelId: EXISTING_DM.id,
    opener: OPENER,
  };
  // Same id, but not a DM — a stale/colliding pointer must not render it.
  assert.equal(
    restoreProjectsAgentConversation({
      stored,
      channels: [{ ...EXISTING_DM, channelType: "stream" }],
      candidates: [AGENT],
      currentPubkey: SELF_PUBKEY,
    }),
    null,
  );
  // A DM with a third participant is someone else's conversation.
  assert.equal(
    restoreProjectsAgentConversation({
      stored,
      channels: [
        {
          ...EXISTING_DM,
          participantPubkeys: [AGENT_PUBKEY, SELF_PUBKEY, "c".repeat(64)],
        },
      ],
      candidates: [AGENT],
      currentPubkey: SELF_PUBKEY,
    }),
    null,
  );
  // A DM that does not include the agent proves nothing about the pointer.
  assert.equal(
    restoreProjectsAgentConversation({
      stored,
      channels: [{ ...EXISTING_DM, participantPubkeys: [SELF_PUBKEY] }],
      candidates: [AGENT],
      currentPubkey: SELF_PUBKEY,
    }),
    null,
  );
  // An agent-only channel has no stranger to reject — the signed-in user's
  // own membership must be required, not merely the absence of strangers.
  assert.equal(
    restoreProjectsAgentConversation({
      stored,
      channels: [{ ...EXISTING_DM, participantPubkeys: [AGENT_PUBKEY] }],
      candidates: [AGENT],
      currentPubkey: SELF_PUBKEY,
    }),
    null,
  );
  // Without a current identity there is nothing to validate against.
  assert.equal(
    restoreProjectsAgentConversation({
      stored,
      channels: [EXISTING_DM],
      candidates: [AGENT],
      currentPubkey: null,
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
  const opener = message(PROMPT_AT, KIND_STREAM_MESSAGE, OPENER.eventId);
  const reply = message(PROMPT_AT + 5, KIND_STREAM_MESSAGE_V2);
  const nonChatEvent = message(PROMPT_AT + 10, 7);

  const visible = visibleConversationMessages(
    [reply, ...olderHistory, opener, nonChatEvent],
    OPENER,
  );
  assert.deepEqual(visible, [opener, reply]);
});

test("unrelated DM history sharing the opener's second is excluded", () => {
  // Relay order within one second is ascending id (newest first), so events
  // with ids greater than the opener's id are strictly older than it.
  const sameSecondOlder = message(
    PROMPT_AT,
    KIND_STREAM_MESSAGE,
    `e${"f".repeat(63)}`,
  );
  const opener = message(PROMPT_AT, KIND_STREAM_MESSAGE, OPENER.eventId);
  const sameSecondNewer = message(
    PROMPT_AT,
    KIND_STREAM_MESSAGE_V2,
    `c${"0".repeat(63)}`,
  );

  const visible = visibleConversationMessages(
    [sameSecondOlder, opener, sameSecondNewer],
    OPENER,
  );
  assert.deepEqual(visible, [opener, sameSecondNewer]);
  assert.equal(isAtOrAfterConversationOpener(sameSecondOlder, OPENER), false);
  assert.equal(isAtOrAfterConversationOpener(opener, OPENER), true);
});

test("a fast reply in the opener's second is admitted via its reply reference", () => {
  // An agent reply signed within the opener's second can carry an id greater
  // than the opener's, which sorts "older" in relay order. Its `e` tag names
  // the opener — causality that must win over the id tiebreak.
  const fastReply = {
    ...message(PROMPT_AT, KIND_STREAM_MESSAGE_V2, `f${"a".repeat(63)}`),
    tags: [["e", OPENER.eventId, "", "reply"]],
  };
  // An unrelated same-second event with the same unlucky id ordering and no
  // reference to the opener stays excluded.
  const unrelated = {
    ...message(PROMPT_AT, KIND_STREAM_MESSAGE, `f${"b".repeat(63)}`),
    tags: [["e", `9${"9".repeat(63)}`, "", "reply"]],
  };

  assert.equal(isAtOrAfterConversationOpener(fastReply, OPENER), true);
  assert.equal(isAtOrAfterConversationOpener(unrelated, OPENER), false);
  const opener = message(PROMPT_AT, KIND_STREAM_MESSAGE, OPENER.eventId);
  // Same-second sort is stable, so relay arrival order (opener first) holds.
  assert.deepEqual(
    visibleConversationMessages([unrelated, opener, fastReply], OPENER),
    [opener, fastReply],
  );
});

test("root questions and separately queried replies stay in conversation order", () => {
  const firstQuestion = message(PROMPT_AT, KIND_STREAM_MESSAGE, OPENER.eventId);
  const firstAnswer = message(PROMPT_AT + 2, KIND_STREAM_MESSAGE_V2);
  const secondQuestion = message(PROMPT_AT + 4);
  const secondAnswer = message(PROMPT_AT + 6, KIND_STREAM_MESSAGE_V2);

  const merged = mergeProjectAgentConversationEvents(
    [firstQuestion, secondQuestion],
    [firstAnswer, secondAnswer, firstAnswer],
  );

  assert.deepEqual(merged, [
    firstQuestion,
    firstAnswer,
    secondQuestion,
    secondAnswer,
  ]);
});

test("storage read rejects legacy timestamp-only pointers", () => {
  // Pointers written before the opener was event-anchored carry only
  // `visibleAfter`. They cannot uphold the same-second isolation invariant,
  // so they are not restorable.
  globalThis.localStorage.setItem(
    `buzz.projects.agentConversation.${encodeURIComponent(WORKSPACE_ID)}`,
    JSON.stringify({
      agentPubkey: AGENT_PUBKEY,
      channelId: EXISTING_DM.id,
      visibleAfter: PROMPT_AT,
    }),
  );
  assert.equal(readStoredProjectsAgentConversation(WORKSPACE_ID), null);
});

test("storage read rejects malformed opener pointers", () => {
  for (const opener of [
    { createdAt: 0, eventId: OPENER.eventId },
    { createdAt: Number.NaN, eventId: OPENER.eventId },
    { createdAt: PROMPT_AT, eventId: "" },
    { createdAt: PROMPT_AT },
    null,
  ]) {
    globalThis.localStorage.setItem(
      `buzz.projects.agentConversation.${encodeURIComponent(WORKSPACE_ID)}`,
      JSON.stringify({
        agentPubkey: AGENT_PUBKEY,
        channelId: EXISTING_DM.id,
        opener,
      }),
    );
    assert.equal(readStoredProjectsAgentConversation(WORKSPACE_ID), null);
  }
});

test("storage round-trips opener-anchored pointers and clears them", () => {
  const stored = {
    agentPubkey: AGENT_PUBKEY,
    channelId: EXISTING_DM.id,
    opener: OPENER,
  };
  writeStoredProjectsAgentConversation(WORKSPACE_ID, stored);
  assert.deepEqual(readStoredProjectsAgentConversation(WORKSPACE_ID), stored);

  clearStoredProjectsAgentConversation(WORKSPACE_ID);
  assert.equal(readStoredProjectsAgentConversation(WORKSPACE_ID), null);
});

test("project-home membership carries the captured relay and signer scopes", () => {
  assert.deepEqual(
    projectAgentMembershipInput({
      channelId: "project-home",
      agentPubkey: AGENT_PUBKEY,
      relayScope: "wss://tenant-a.example",
      signerScope: SELF_PUBKEY,
    }),
    {
      channelId: "project-home",
      pubkeys: [AGENT_PUBKEY],
      role: "bot",
      expectedRelayUrl: "wss://tenant-a.example",
      expectedSignerPubkey: SELF_PUBKEY,
    },
  );
});

// ── submitProjectAgentMessage ───────────────────────────────────────────────

/** Models the backend's fail-closed scope checks: commands resolve the active
 * relay AND the active signing identity when they run and reject when a
 * caller-captured scope no longer matches either. `active`/`activeSigner`
 * are mutable so tests can switch communities (or race the identity swap)
 * mid-flight. Startup is a recorded side effect too: activating the (agent,
 * relay) pair grants channel/tool access, so the cross-tenant start must be
 * observable, not merely survivable. */
function makeScopedBackend(active, activeSigner = SELF_PUBKEY) {
  const state = { active, activeSigner, starts: [], dmOpens: [], sends: [] };
  const assertScope = ({ expectedRelayUrl, expectedSignerPubkey }) => {
    if (expectedRelayUrl !== undefined && expectedRelayUrl !== state.active) {
      throw new Error(
        "active community changed before the message was submitted; not sent",
      );
    }
    if (
      expectedSignerPubkey !== undefined &&
      expectedSignerPubkey !== state.activeSigner
    ) {
      throw new Error(
        "active identity changed before the message was submitted; not sent",
      );
    }
  };
  return {
    state,
    startAgent: async (input) => {
      assertScope(input);
      state.starts.push({ relay: state.active, input });
      return {};
    },
    openDm: async (input) => {
      assertScope(input);
      state.dmOpens.push({ relay: state.active, input });
      return { id: `dm-on-${state.active}` };
    },
    send: async (request) => {
      assertScope(request);
      state.sends.push({ relay: state.active, request });
      return { eventId: `f${"0".repeat(63)}`, createdAt: PROMPT_AT };
    },
  };
}

test("a community switch during agent startup publishes nothing to either tenant", async () => {
  const backend = makeScopedBackend("wss://tenant-a.example");
  const scopedStartAgent = backend.startAgent;
  let releaseSwitch;
  const switchGate = new Promise((resolve) => {
    releaseSwitch = resolve;
  });

  const pending = submitProjectAgentMessage({
    agent: { pubkey: AGENT_PUBKEY, isManaged: true, isActive: false },
    conversation: null,
    content: "tenant A repo context",
    mentionPubkeys: [AGENT_PUBKEY],
    relayScope: "wss://tenant-a.example",
    signerScope: SELF_PUBKEY,
    startAgent: async (input) => {
      // The user switches communities while the callback is suspended on
      // the managed-agent startup await (the backend's mesh preflight).
      // Remounting removed the panel, but this callback keeps running —
      // the backend's post-await check must reject before the spawn.
      await switchGate;
      return scopedStartAgent(input);
    },
    openDm: backend.openDm,
    send: backend.send,
  });

  backend.state.active = "wss://tenant-b.example";
  releaseSwitch();

  await assert.rejects(pending, /active community changed/);
  // The start side effect itself was blocked — the agent pair was never
  // activated in tenant B — and nothing downstream ran either.
  assert.deepEqual(backend.state.starts, []);
  assert.deepEqual(backend.state.dmOpens, []);
  assert.deepEqual(backend.state.sends, []);
});

test("an identity swap racing the send fails closed at the signer check", async () => {
  const backend = makeScopedBackend("wss://tenant-a.example");
  const scopedSend = backend.send;
  const pending = submitProjectAgentMessage({
    agent: { pubkey: AGENT_PUBKEY, isManaged: false, isActive: true },
    conversation: { channel: EXISTING_DM, opener: OPENER },
    content: "tenant A repo context",
    mentionPubkeys: [AGENT_PUBKEY],
    relayScope: "wss://tenant-a.example",
    signerScope: SELF_PUBKEY,
    startAgent: backend.startAgent,
    openDm: () => {
      throw new Error("an existing conversation must reuse its channel");
    },
    send: async (request) => {
      // A workspace switch mutates relay and keys under separate locks; the
      // narrowest race leaves the relay matching while the identity has
      // already swapped. The signer scope must catch what the relay scope
      // cannot.
      backend.state.activeSigner = "e".repeat(64);
      return scopedSend(request);
    },
  });

  await assert.rejects(pending, /active identity changed/);
  assert.deepEqual(backend.state.sends, []);
});

test("a community switch during the DM open fails the send closed", async () => {
  const backend = makeScopedBackend("wss://tenant-a.example");
  const scopedOpenDm = backend.openDm;
  const pending = submitProjectAgentMessage({
    agent: { pubkey: AGENT_PUBKEY, isManaged: false, isActive: true },
    conversation: null,
    content: "tenant A repo context",
    mentionPubkeys: [AGENT_PUBKEY],
    relayScope: "wss://tenant-a.example",
    signerScope: SELF_PUBKEY,
    startAgent: () => {
      throw new Error("inactive relay agents are not startable");
    },
    openDm: async (input) => {
      const channel = await scopedOpenDm(input);
      // The switch lands after the DM was opened on tenant A but before the
      // message submit — the narrowest window Carl's finding names.
      backend.state.active = "wss://tenant-b.example";
      return channel;
    },
    send: backend.send,
  });

  await assert.rejects(pending, /active community changed/);
  // The DM was legitimately opened while tenant A was still active…
  assert.equal(backend.state.dmOpens.length, 1);
  assert.equal(backend.state.dmOpens[0].relay, "wss://tenant-a.example");
  // …but nothing was ever published anywhere.
  assert.deepEqual(backend.state.sends, []);
});

test("the captured scope rides every relay side effect of a first send", async () => {
  const backend = makeScopedBackend("wss://tenant-a.example");
  const result = await submitProjectAgentMessage({
    agent: { pubkey: AGENT_PUBKEY, isManaged: true, isActive: false },
    conversation: null,
    content: "opener",
    mentionPubkeys: [AGENT_PUBKEY],
    relayScope: "wss://tenant-a.example",
    signerScope: SELF_PUBKEY,
    startAgent: backend.startAgent,
    openDm: backend.openDm,
    send: backend.send,
  });

  // Startup, DM open, and send all carry the same captured scope pair —
  // including the startup call, whose spawn/deploy side effect the backend
  // gates on exactly these values.
  assert.equal(backend.state.starts.length, 1);
  assert.equal(
    backend.state.starts[0].input.expectedRelayUrl,
    "wss://tenant-a.example",
  );
  assert.equal(backend.state.starts[0].input.expectedSignerPubkey, SELF_PUBKEY);
  assert.equal(
    backend.state.dmOpens[0].input.expectedRelayUrl,
    "wss://tenant-a.example",
  );
  assert.equal(
    backend.state.dmOpens[0].input.expectedSignerPubkey,
    SELF_PUBKEY,
  );
  assert.equal(
    backend.state.sends[0].request.expectedRelayUrl,
    "wss://tenant-a.example",
  );
  assert.equal(
    backend.state.sends[0].request.expectedSignerPubkey,
    SELF_PUBKEY,
  );
  // The opener is a thread root: no parent reference.
  assert.equal(backend.state.sends[0].request.parentEventId, undefined);
  assert.equal(result.channel.id, "dm-on-wss://tenant-a.example");
});

test("a home channel first send does not open a DM", async () => {
  const backend = makeScopedBackend("wss://tenant-a.example");
  const home = { id: "project-channel-1" };
  const result = await submitProjectAgentMessage({
    agent: { pubkey: AGENT_PUBKEY, isManaged: false, isActive: true },
    conversation: null,
    content: "build this project",
    mentionPubkeys: [AGENT_PUBKEY],
    relayScope: "wss://tenant-a.example",
    signerScope: SELF_PUBKEY,
    homeChannel: home,
    startAgent: backend.startAgent,
    openDm: () => {
      throw new Error("project home chat must use the project channel");
    },
    send: backend.send,
  });

  assert.deepEqual(backend.state.dmOpens, []);
  assert.equal(result.channel.id, home.id);
  assert.equal(backend.state.sends[0].request.channelId, home.id);
});

test("follow-ups reply to the opener so same-second id ordering cannot hide them", async () => {
  const backend = makeScopedBackend("wss://tenant-a.example");
  await submitProjectAgentMessage({
    agent: { pubkey: AGENT_PUBKEY, isManaged: false, isActive: true },
    conversation: { channel: EXISTING_DM, opener: OPENER },
    content: "follow-up in the opener's second",
    mentionPubkeys: [AGENT_PUBKEY],
    relayScope: "wss://tenant-a.example",
    signerScope: SELF_PUBKEY,
    startAgent: async () => {},
    openDm: () => {
      throw new Error("an existing conversation must reuse its channel");
    },
    send: backend.send,
  });

  const request = backend.state.sends[0].request;
  assert.equal(request.channelId, EXISTING_DM.id);
  assert.equal(request.parentEventId, OPENER.eventId);

  // Carl's exact-head probe: a same-second follow-up whose random id lands on
  // the rejected side of the id tiebreak (`e… > d…`). As an unreferenced root
  // it would vanish; as the reply the submit path now sends, it is admitted.
  const rejectedSideId = `e${"0".repeat(63)}`;
  const asUnreferencedRoot = {
    created_at: OPENER.createdAt,
    id: rejectedSideId,
    tags: [],
  };
  assert.equal(
    isAtOrAfterConversationOpener(asUnreferencedRoot, OPENER),
    false,
  );
  const asSentReply = {
    created_at: OPENER.createdAt,
    id: rejectedSideId,
    tags: [["e", OPENER.eventId, "", "reply"]],
  };
  assert.equal(isAtOrAfterConversationOpener(asSentReply, OPENER), true);
});
