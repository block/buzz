import assert from "node:assert/strict";
import test from "node:test";

import { shouldHideAgentFromMentions } from "./agentAutocompleteEligibility.ts";
import { snapshotDraftMentionRefs } from "../../messages/lib/draftMentionRefs.ts";
import { mentionCandidateLabel } from "../../messages/lib/mentionCandidates.ts";
import { mapMentionCandidateToSuggestion } from "../../messages/lib/mentionSuggestionMapping.ts";
import { messageMentionPubkeys } from "../../messages/lib/messageMentionPubkeys.ts";

/**
 * The eligibility predicate is well covered on its own, which is exactly why it
 * was possible to relax it and still not know whether an agent someone else
 * manages actually becomes mentionable. Answering that means walking the whole
 * path a picked mention travels — filter, label, suggestion, draft ref, `p` tag
 * — because every stage can drop it, and only the last one is observable to the
 * agent.
 */
const SENDER = "a".repeat(64);
const PEER = "b".repeat(64);
const ERP_AGENT = "e".repeat(64);

const channel = (over = {}) => ({
  channelType: "stream",
  memberPubkeys: [SENDER, PEER],
  participantPubkeys: [],
  ...over,
});

const agent = (over = {}) => ({
  kind: "identity",
  pubkey: ERP_AGENT,
  displayName: "erp",
  isAgent: true,
  isMember: true,
  ...over,
});

/**
 * One composer round trip: would the autocomplete offer this candidate, and if
 * the author picks it and sends, which pubkeys does the event address?
 */
function compose(
  candidate,
  { chan = channel(), mentionableAgentPubkeys = new Set(), typedAs } = {},
) {
  const hidden = shouldHideAgentFromMentions({
    isAgent: candidate.isAgent,
    isMember: candidate.isMember,
    pubkey: candidate.pubkey,
    mentionableAgentPubkeys,
  });
  if (hidden) return { suggested: false, pubkeys: [] };

  const label = mentionCandidateLabel(candidate);
  const suggestion = mapMentionCandidateToSuggestion({
    candidate,
    label,
    channelType: chan.channelType,
    currentPubkey: SENDER,
  });
  const content = `@${typedAs ?? suggestion.displayName} what is the ARR?`;
  const refs = snapshotDraftMentionRefs(
    content,
    new Map([[suggestion.displayName, suggestion.pubkey]]),
    [suggestion.displayName],
  );
  return {
    suggested: true,
    suggestion,
    pubkeys: messageMentionPubkeys(
      chan,
      SENDER,
      refs.map((ref) => ref.pubkey),
    ),
  };
}

test("a channel-member agent managed on another device reaches the p tag", () => {
  // The ERP and Codex agents' state on everyone's device but the operator's:
  // in the channel, in the relay directory, absent from the local managed list.
  const result = compose(agent(), { mentionableAgentPubkeys: new Set() });

  assert.equal(result.suggested, true);
  assert.equal(result.suggestion.isAgent, true);
  // Not flagged as an outsider being pulled in — it is already in the channel.
  assert.equal(result.suggestion.notInChannel, false);
  assert.deepEqual(result.pubkeys, [ERP_AGENT]);
});

test("an agent in nobody's channel and nobody's managed list never reaches a p tag", () => {
  const result = compose(agent({ isMember: false }), {
    mentionableAgentPubkeys: new Set(),
  });

  assert.equal(result.suggested, false);
  assert.deepEqual(result.pubkeys, []);
});

test("a locally managed agent still reaches the p tag from outside the channel", () => {
  // Unchanged behaviour: your own agent is invocable wherever you are, and the
  // suggestion says plainly that it is not in this channel yet.
  const result = compose(agent({ isMember: false }), {
    mentionableAgentPubkeys: new Set([ERP_AGENT]),
  });

  assert.equal(result.suggested, true);
  assert.equal(result.suggestion.notInChannel, true);
  assert.deepEqual(result.pubkeys, [ERP_AGENT]);
});

test("a DM addresses its own participants, not an agent that is not in it", () => {
  // Membership is what widened, and in a DM the member list is the two people
  // in it — so the DM's recipients are unchanged and an uninvolved agent, which
  // can never carry `isMember`, is filtered before it becomes a suggestion.
  const dm = channel({ channelType: "dm" });

  assert.deepEqual(compose(agent({ isMember: false }), { chan: dm }), {
    suggested: false,
    pubkeys: [],
  });
  assert.deepEqual(messageMentionPubkeys(dm, SENDER), [PEER]);
});

test("the mention survives being typed in a different case to the label", () => {
  // The label round-trips through free text before it becomes a `p` tag, so a
  // member agent someone types from memory rather than picking still routes.
  const result = compose(agent({ displayName: "ERP" }), { typedAs: "erp" });
  assert.deepEqual(result.pubkeys, [ERP_AGENT]);
});
