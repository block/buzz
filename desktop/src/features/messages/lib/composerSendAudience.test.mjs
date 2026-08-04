import assert from "node:assert/strict";
import test from "node:test";

import {
  describeComposerAudienceHint,
  resolveComposerSendAudience,
} from "./composerSendAudience.ts";

const human = "1".repeat(64);
const agentA = "a".repeat(64);
const agentB = "b".repeat(64);

test("channel multi-agent unaddressed merges all verified agents into mentions", () => {
  const result = resolveComposerSendAudience({
    conversation: "channel",
    messagePosition: "top-level",
    unaddressedMode: "all-channel-agents",
    keepAddressedAgentsActive: false,
    explicitMentionPubkeys: [],
    explicitAgentPubkeys: [],
    currentAgentPubkey: null,
    channelMemberPubkeys: [human, agentA, agentB],
    verifiedChannelAgentPubkeys: [agentA, agentB],
    persistentThreadAudience: [],
  });
  assert.deepEqual([...result.mentionPubkeys].sort(), [agentA, agentB].sort());
  assert.equal(result.sharedThread, true);
  assert.equal(result.replyPlacement.kind, "top-level"); // no humanMessageEventId
});

test("explicit agent mention overrides implicit all-agents", () => {
  const result = resolveComposerSendAudience({
    conversation: "channel",
    messagePosition: "top-level",
    unaddressedMode: "all-channel-agents",
    keepAddressedAgentsActive: false,
    explicitMentionPubkeys: [agentB, human],
    explicitAgentPubkeys: [agentB],
    currentAgentPubkey: null,
    channelMemberPubkeys: [human, agentA, agentB],
    verifiedChannelAgentPubkeys: [agentA, agentB],
    persistentThreadAudience: [],
  });
  assert.deepEqual([...result.mentionPubkeys].sort(), [agentB, human].sort());
  assert.deepEqual(result.agentAudiencePubkeys, [agentB]);
});

test("mentions-only with no explicit agents yields empty agent audience", () => {
  const result = resolveComposerSendAudience({
    conversation: "channel",
    messagePosition: "top-level",
    unaddressedMode: "mentions-only",
    keepAddressedAgentsActive: false,
    explicitMentionPubkeys: [human],
    explicitAgentPubkeys: [],
    currentAgentPubkey: null,
    channelMemberPubkeys: [human, agentA],
    verifiedChannelAgentPubkeys: [agentA],
    persistentThreadAudience: [],
  });
  assert.deepEqual(result.agentAudiencePubkeys, []);
  assert.deepEqual(result.mentionPubkeys, [human]);
});

test("describeComposerAudienceHint covers modes", () => {
  assert.match(
    describeComposerAudienceHint({
      conversation: "channel",
      unaddressedMode: "all-channel-agents",
      explicitAgentCount: 0,
      implicitAgentCount: 3,
      retainDraft: false,
    }) ?? "",
    /all 3 channel agents/,
  );
  assert.match(
    describeComposerAudienceHint({
      conversation: "channel",
      unaddressedMode: "mentions-only",
      explicitAgentCount: 0,
      implicitAgentCount: 0,
      retainDraft: false,
    }) ?? "",
    /Mentions only/,
  );
  assert.equal(
    describeComposerAudienceHint({
      conversation: "direct",
      unaddressedMode: "all-channel-agents",
      explicitAgentCount: 0,
      implicitAgentCount: 1,
      retainDraft: false,
    }),
    null,
  );
});

test("keep-addressed persistent audience applies under mentions-only", () => {
  const result = resolveComposerSendAudience({
    conversation: "channel",
    messagePosition: "top-level",
    unaddressedMode: "mentions-only",
    keepAddressedAgentsActive: true,
    explicitMentionPubkeys: [],
    explicitAgentPubkeys: [],
    currentAgentPubkey: null,
    channelMemberPubkeys: [human, agentA, agentB],
    verifiedChannelAgentPubkeys: [agentA, agentB],
    persistentThreadAudience: [agentA],
  });
  assert.deepEqual(result.agentAudiencePubkeys, [agentA]);
  assert.equal(result.sharedThread, false);
});

test("recipient load error retains draft and clears audience", () => {
  const result = resolveComposerSendAudience({
    conversation: "channel",
    messagePosition: "top-level",
    unaddressedMode: "all-channel-agents",
    keepAddressedAgentsActive: false,
    explicitMentionPubkeys: [],
    explicitAgentPubkeys: [],
    currentAgentPubkey: null,
    channelMemberPubkeys: [human, agentA],
    verifiedChannelAgentPubkeys: [agentA],
    persistentThreadAudience: [],
    recipientLoadError: true,
  });
  assert.deepEqual(result.mentionPubkeys, []);
  assert.equal(result.retainDraft, true);
});

test("direct conversation addresses current agent only", () => {
  const result = resolveComposerSendAudience({
    conversation: "direct",
    messagePosition: "top-level",
    unaddressedMode: "all-channel-agents",
    keepAddressedAgentsActive: false,
    explicitMentionPubkeys: [],
    explicitAgentPubkeys: [],
    currentAgentPubkey: agentA,
    channelMemberPubkeys: [human, agentA],
    verifiedChannelAgentPubkeys: [agentA],
    persistentThreadAudience: [],
  });
  assert.deepEqual(result.mentionPubkeys, [agentA]);
  assert.equal(result.sharedThread, false);
  assert.equal(result.replyPlacement.kind, "top-level");
});

test("direct path keeps explicit agent mentions for DM expansion", () => {
  const result = resolveComposerSendAudience({
    conversation: "direct",
    messagePosition: "top-level",
    unaddressedMode: "all-channel-agents",
    keepAddressedAgentsActive: false,
    explicitMentionPubkeys: [agentB],
    explicitAgentPubkeys: [agentB],
    currentAgentPubkey: agentA,
    channelMemberPubkeys: [human, agentA],
    verifiedChannelAgentPubkeys: [agentA, agentB],
    persistentThreadAudience: [],
  });
  // Both the DM peer agent and the newly @mentioned agent must remain.
  assert.deepEqual([...result.mentionPubkeys].sort(), [agentA, agentB].sort());
});

test("manual removal drops persistent agent from audience", () => {
  const result = resolveComposerSendAudience({
    conversation: "channel",
    messagePosition: "top-level",
    unaddressedMode: "mentions-only",
    keepAddressedAgentsActive: true,
    explicitMentionPubkeys: [],
    explicitAgentPubkeys: [],
    currentAgentPubkey: null,
    channelMemberPubkeys: [human, agentA, agentB],
    verifiedChannelAgentPubkeys: [agentA, agentB],
    persistentThreadAudience: [agentA, agentB],
    manualRemovedPubkeys: [agentB],
  });
  assert.deepEqual(result.agentAudiencePubkeys, [agentA]);
});
