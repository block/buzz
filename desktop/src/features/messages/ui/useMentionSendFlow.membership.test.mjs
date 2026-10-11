import assert from "node:assert/strict";
import { test } from "node:test";
import { renderHook } from "@testing-library/react";
import { useAgentMentionRevalidation } from "../lib/agentMentionRevalidation.ts";
import { setup, deferred, KEY } from "./useMentionSendFlow.test-support.mjs";

// The send flow runs against the production publish revalidation. Only the
// Tauri bridge is replaced: it serves the live member list, which can drift
// from the roster cached when the person was selected.
const HUMAN = "d".repeat(64);
const TEXT = "@Pat @RemoteScout hello";
const REMOVED =
  "Someone you mentioned is not in this channel now. Add them or remove the mention, then retry.";

const AGENT_MEMBER = { pubkey: KEY, role: "bot", is_agent: true };

async function membershipSetup(delay) {
  const live = {
    members: [{ pubkey: HUMAN, role: "member", is_agent: false }],
    gate: null,
    agentRead: null,
  };
  window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      if (command === "get_channel_members") {
        if (live.gate) await live.gate.promise;
        // Model the native command: it snapshots the roster, then (unless
        // roster only) awaits a profile lookup before it returns that
        // snapshot. A removal during the lookup is not in the answer.
        const snapshot = live.members;
        if (!args?.rosterOnly && live.duringProfiles) live.duringProfiles();
        return { members: snapshot };
      }
      throw new Error(`unmocked ${command}`);
    },
  };
  const { result: revalidate } = renderHook(() =>
    useAgentMentionRevalidation({
      agentPubkeys: new Set([KEY]),
      getSelectedAgentPubkeys: () => new Set(),
      channelType: "stream",
      currentPubkey: "a".repeat(64),
      eligibilityScope: { type: "channel", channelId: "general" },
      sharedChannelIds: new Set(),
      refetchManagedAgents: async () => {
        // Hold only the final publish pass, not the earlier prepare pass.
        if (live.agentRead && live.phase === "publish")
          await live.agentRead.promise;
        return { data: [{ pubkey: KEY }], error: null };
      },
    }),
  );
  const s = await setup();
  s.dismiss();
  const mentions = s.options.mentions;
  mentions.extractMentionPubkeys = () => [HUMAN, KEY];
  // Pat was a member when selected. The agent is not, so Send opens the
  // Invite prompt, and the publish continues after the invite.
  mentions.memberPubkeys = new Set([HUMAN]);
  mentions.revalidateMentionPubkeys = async (keys, channel, options) => {
    s.calls.push([options.phase, channel]);
    live.phase = options.phase;
    return revalidate.current(keys, channel, options);
  };
  if (delay === "upload")
    s.control.attachments = [{ id: "file", file: {}, spoilered: false }];
  s.rerender();
  return { s, live };
}

// Every signed recipient, the managed agent included, must be on the fresh
// roster. The agent joins through the Invite prompt, then can be removed.
const REMOVALS = {
  none: (members) => members,
  person: (members) => members.filter(({ pubkey }) => pubkey !== HUMAN),
  agent: (members) => members.filter(({ pubkey }) => pubkey !== KEY),
};

for (const delay of ["upload", "invite"]) {
  for (const removed of Object.keys(REMOVALS)) {
    test(`chat publish ${removed === "none" ? "keeps everyone present" : `refuses the ${removed} removed`} during the ${delay} delay`, async () => {
      const { s, live } = await membershipSetup(delay);
      if (delay === "invite") s.control.add = deferred();
      await s.prompt(TEXT);
      await s.invite();
      assert.equal(s.events("add").length, 1);
      live.members = [...live.members, AGENT_MEMBER];
      if (delay === "upload") {
        assert.ok(s.control.uploadCallbacks);
        assert.equal(s.options.contentRef.current, "", "optimistic clear");
      }
      live.members = REMOVALS[removed](live.members);
      if (delay === "invite") await s.finish(s.control.add);
      else
        await s.act(async () =>
          s.control.uploadCallbacks.onComplete(
            [],
            new AbortController().signal,
          ),
        );
      await s.flush();
      assertOutcome(s, removed !== "none");
    });
  }
}

function assertOutcome(s, refused) {
  const sends = s.events("SEND");
  if (!refused) {
    assert.equal(sends.length, 1);
    assert.deepEqual(sends[0][2], [HUMAN, KEY]);
    return;
  }
  assert.equal(sends.length, 0);
  assert.deepEqual(
    s.events("error").map(([, message]) => message),
    [REMOVED],
  );
  assert.equal(
    s.options.contentRef.current,
    TEXT,
    "the draft is kept for retry",
  );
}

// The roster read must be the last check. A removal while the agent
// directory read is still pending has to be seen before signing.
for (const removed of ["person", "agent"]) {
  test(`chat publish refuses the ${removed} removed while agent authorization is held`, async () => {
    const { s, live } = await membershipSetup("invite");
    s.control.add = deferred();
    await s.prompt(TEXT);
    await s.invite();
    live.members = [...live.members, AGENT_MEMBER];
    live.agentRead = deferred();
    await s.finish(s.control.add);
    await s.flush();
    assert.equal(live.phase, "publish", "held in the final publish pass");
    assert.equal(s.events("SEND").length, 0);
    assert.equal(s.events("error").length, 0, "not yet refused");
    live.members = REMOVALS[removed](live.members);
    await s.finish(live.agentRead);
    await s.flush();
    assertOutcome(s, true);
  });
}

test("Send without inviting still publishes: a declined person becomes a reference", async () => {
  const DECLINED = "e".repeat(64);
  const { s } = await membershipSetup("decline");
  s.options.mentions.extractMentionPubkeys = () => [HUMAN, DECLINED];
  s.options.mentions.isAgentPubkey = () => false;
  s.rerender();
  await s.prompt("@Pat @Sam hello");
  assert.equal(s.result.current.nonMemberPromptProps.open, true);
  await s.act(async () => s.result.current.nonMemberPromptProps.onDoNothing());
  await s.flush();
  const sends = s.events("SEND");
  assert.equal(sends.length, 1);
  assert.deepEqual(sends[0][2], [HUMAN]);
  assert.equal(s.events("error").length, 0);
});

// The publish gate must not accept a snapshot that a later await inside the
// member read made stale. Every signed recipient must be a member when the
// message is sent.
for (const removed of ["person", "agent"]) {
  test(`chat publish never signs the ${removed} removed during the member read's profile lookup`, async () => {
    const { s, live } = await membershipSetup("invite");
    await s.prompt(TEXT);
    live.members = [...live.members, AGENT_MEMBER];
    live.duringProfiles = () => {
      if (live.phase === "publish")
        live.members = REMOVALS[removed](live.members);
    };
    await s.invite();
    await s.flush();
    const sends = s.events("SEND");
    const membersAtSend = new Set(live.members.map(({ pubkey }) => pubkey));
    for (const [, , recipients] of sends)
      for (const pubkey of recipients)
        assert.ok(membersAtSend.has(pubkey), `${pubkey} was not a member`);
    assert.equal(sends.length, 1, "the roster-only read saw everyone");
  });
}
