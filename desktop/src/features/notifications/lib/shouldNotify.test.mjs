import assert from "node:assert/strict";
import test from "node:test";

import {
  hasAuthoredMentionForEvent,
  isHighPriorityEventForUser,
  needsResolvedParentAuthor,
  shouldNotifyForEvent,
} from "./shouldNotify.ts";

const PUBKEY = "a".repeat(64);
const OTHER_PUBKEY = "b".repeat(64);
const ROOT_ID = `root-${"0".repeat(59)}`;
const PARENT_ID = `parent-${"0".repeat(57)}`;

const EMPTY = new Set();

/** Returns a minimal RelayEvent with the given tags. */
function makeEvent(tags = [], overrides = {}) {
  return {
    id: `event-${"0".repeat(59)}`,
    pubkey: OTHER_PUBKEY,
    created_at: 1700000000,
    kind: 9,
    tags,
    content: "hello",
    sig: "s".repeat(128),
    ...overrides,
  };
}

const rootTag = (id) => ["e", id, "", "root"];
const replyTag = (id) => ["e", id, "", "reply"];
const pTag = (pubkey) => ["p", pubkey];
const broadcastTag = () => ["broadcast", "1"];

const opts = (overrides = {}) => ({
  participatedRootIds: EMPTY,
  followedRootIds: EMPTY,
  authoredRootIds: EMPTY,
  ...overrides,
});

test("top-level message (no e-tags) notifies", () => {
  assert.equal(shouldNotifyForEvent(makeEvent([]), PUBKEY, opts()), true);
});

test("top-level message with unrelated p-tag notifies", () => {
  assert.equal(
    shouldNotifyForEvent(makeEvent([pTag(OTHER_PUBKEY)]), PUBKEY, opts()),
    true,
  );
});

test("broadcast reply to unrelated thread notifies", () => {
  const event = makeEvent([replyTag(ROOT_ID), broadcastTag()]);
  assert.equal(shouldNotifyForEvent(event, PUBKEY, opts()), true);
});

test("broadcast reply with root+reply tags notifies", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    broadcastTag(),
  ]);
  assert.equal(shouldNotifyForEvent(event, PUBKEY, opts()), true);
});

test("thread reply with p-tag mention of currentPubkey notifies", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(shouldNotifyForEvent(event, PUBKEY, opts()), true);
});

test("p-tag mention matching is case-insensitive", () => {
  const event = makeEvent([replyTag(ROOT_ID), pTag(PUBKEY.toUpperCase())]);
  assert.equal(shouldNotifyForEvent(event, PUBKEY, opts()), true);
});

test("p-tag mention of a different pubkey does not trigger mention path", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(OTHER_PUBKEY),
  ]);
  assert.equal(shouldNotifyForEvent(event, PUBKEY, opts()), false);
});

test("thread reply to participated thread notifies", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ participatedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

test("shallow thread reply (root===parent) to participated thread notifies", () => {
  const event = makeEvent([replyTag(ROOT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ participatedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

test("thread reply to followed thread notifies", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ followedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

test("thread reply to authored thread notifies", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ authoredRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

test("thread reply to unrelated thread does not notify", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(shouldNotifyForEvent(event, PUBKEY, opts()), false);
});

test("muted thread reply suppresses participated", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({
        participatedRootIds: new Set([ROOT_ID]),
        mutedRootIds: new Set([ROOT_ID]),
      }),
    ),
    false,
  );
});

test("muted thread reply suppresses followed", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({
        followedRootIds: new Set([ROOT_ID]),
        mutedRootIds: new Set([ROOT_ID]),
      }),
    ),
    false,
  );
});

test("muted thread reply suppresses authored", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({
        authoredRootIds: new Set([ROOT_ID]),
        mutedRootIds: new Set([ROOT_ID]),
      }),
    ),
    false,
  );
});

test("muted thread reply still notifies when currentPubkey is mentioned via p-tag", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ mutedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

test("muted thread reply answering us does not pierce the mute via its p-tag", () => {
  // Replies p-tag the author they answer so agent `require_mention`
  // subscriptions receive them. That tag must not read as a mention.
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ mutedRootIds: new Set([ROOT_ID]), parentAuthorPubkey: PUBKEY }),
    ),
    false,
  );
});

test("unmuted thread reply answering us still notifies via authoredRootIds", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({
        authoredRootIds: new Set([ROOT_ID]),
        parentAuthorPubkey: PUBKEY,
      }),
    ),
    true,
  );
});

test("muted thread reply answering someone else still notifies us on mention", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({
        mutedRootIds: new Set([ROOT_ID]),
        parentAuthorPubkey: OTHER_PUBKEY,
      }),
    ),
    true,
  );
});

test("muted channel reply answering us is suppressed", () => {
  const event = makeEvent([replyTag(PARENT_ID), pTag(PUBKEY)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({
        mutedChannelIds: new Set(["channel-1"]),
        channelId: "channel-1",
        parentAuthorPubkey: PUBKEY.toUpperCase(),
      }),
    ),
    false,
  );
});

test("parentAuthorPubkey is ignored on a top-level message", () => {
  const event = makeEvent([pTag(PUBKEY)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({
        mutedChannelIds: new Set(["channel-1"]),
        channelId: "channel-1",
        parentAuthorPubkey: PUBKEY,
      }),
    ),
    true,
  );
});

test("unmuted reply answering us notifies with no local thread state at all", () => {
  // The participated/authored sets are local and rebuilt from the unread
  // window, so a fresh install must not lose "someone replied to you".
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(
    shouldNotifyForEvent(event, PUBKEY, opts({ parentAuthorPubkey: PUBKEY })),
    true,
  );
});

test("hasAuthoredMentionForEvent: a reply answering us is not an authored mention", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(hasAuthoredMentionForEvent(event, PUBKEY, PUBKEY), false);
});

test("hasAuthoredMentionForEvent: a reply answering someone else is a mention", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(hasAuthoredMentionForEvent(event, PUBKEY, OTHER_PUBKEY), true);
});

test("hasAuthoredMentionForEvent: falls back to the raw p-tag without a parent author", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(hasAuthoredMentionForEvent(event, PUBKEY, null), true);
  assert.equal(hasAuthoredMentionForEvent(event, PUBKEY), true);
});

test("hasAuthoredMentionForEvent: a top-level message we authored the parent of is still a mention", () => {
  const event = makeEvent([pTag(PUBKEY)]);
  assert.equal(hasAuthoredMentionForEvent(event, PUBKEY, PUBKEY), true);
});

test("hasAuthoredMentionForEvent: no p-tag is never a mention", () => {
  const event = makeEvent([replyTag(PARENT_ID)]);
  assert.equal(hasAuthoredMentionForEvent(event, PUBKEY, OTHER_PUBKEY), false);
});

test("muted rootId does not suppress a top-level (non-reply) message", () => {
  const event = makeEvent([]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ mutedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

test("omitting mutedRootIds parameter defaults to empty set and still notifies participated", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ participatedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

test("omitting mutedRootIds for unrelated thread returns false without throwing", () => {
  const event = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID)]);
  assert.equal(shouldNotifyForEvent(event, PUBKEY, opts()), false);
});

test("muted shallow thread reply (rootId falls back to parentId) is suppressed", () => {
  const event = makeEvent([replyTag(ROOT_ID)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({
        participatedRootIds: new Set([ROOT_ID]),
        mutedRootIds: new Set([ROOT_ID]),
      }),
    ),
    false,
  );
});

test("broadcast reply on a muted thread still notifies (broadcast overrides mute)", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    broadcastTag(),
  ]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({ mutedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

test("empty currentPubkey skips p-tag check — muted thread is suppressed", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      "",
      opts({
        participatedRootIds: new Set([ROOT_ID]),
        mutedRootIds: new Set([ROOT_ID]),
      }),
    ),
    false,
  );
});

test("empty currentPubkey with participated thread still notifies (no mute)", () => {
  const event = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      "",
      opts({ participatedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
  );
});

test("isHighPriorityEventForUser returns true when p-tag matches currentPubkey", () => {
  // A reply needs its parent resolved to prove the tag is a real mention;
  // pass an author other than the user so it is one.
  const event = makeEvent([replyTag(ROOT_ID), pTag(PUBKEY)]);
  assert.equal(isHighPriorityEventForUser(event, PUBKEY, OTHER_PUBKEY), true);
});

test("isHighPriorityEventForUser returns true for broadcast reply", () => {
  const event = makeEvent([replyTag(ROOT_ID), broadcastTag()]);
  assert.equal(isHighPriorityEventForUser(event, PUBKEY), true);
});

test("isHighPriorityEventForUser returns false when no matching p-tag and no broadcast tag", () => {
  const event = makeEvent([replyTag(ROOT_ID), pTag(OTHER_PUBKEY)]);
  assert.equal(isHighPriorityEventForUser(event, PUBKEY), false);
});

test("isHighPriorityEventForUser p-tag matching is case-insensitive", () => {
  const event = makeEvent([replyTag(ROOT_ID), pTag(PUBKEY.toUpperCase())]);
  assert.equal(isHighPriorityEventForUser(event, PUBKEY, OTHER_PUBKEY), true);
});

test("isHighPriorityEventForUser returns false when currentPubkey is empty", () => {
  // Short-circuits before p-tag check; broadcast absent so also false
  const event = makeEvent([replyTag(ROOT_ID), pTag(PUBKEY)]);
  assert.equal(isHighPriorityEventForUser(event, ""), false);
});

test("isHighPriorityEventForUser returns false for event with no tags at all", () => {
  const event = makeEvent([]);
  assert.equal(isHighPriorityEventForUser(event, PUBKEY), false);
});

const CHANNEL_ID = "11111111-1111-4111-8111-111111111111";
const parentOpts = (overrides = {}) => ({
  cachedParentAuthor: null,
  ...overrides,
});

test("an uncached reply that p-tags the user escalates to a parent lookup", () => {
  // The regression this guards: the cache only holds the viewed channel, so a
  // channel the user never opened always misses, and the addressing p-tag then
  // reads as a mention. Asserted for both reply shapes — with and without a
  // root tag — because the escalation is decided from the reply marker.
  const bare = makeEvent([replyTag(PARENT_ID), pTag(PUBKEY)]);
  assert.equal(needsResolvedParentAuthor(bare, PUBKEY, parentOpts()), true);

  const rooted = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(PUBKEY),
  ]);
  assert.equal(needsResolvedParentAuthor(rooted, PUBKEY, parentOpts()), true);
});

test("a broadcast reply never escalates to a parent lookup", () => {
  const event = makeEvent([replyTag(PARENT_ID), pTag(PUBKEY), broadcastTag()]);
  assert.equal(needsResolvedParentAuthor(event, PUBKEY, parentOpts()), false);
});

test("no escalation in a DM, where no consumer reads the parent author", () => {
  const event = makeEvent([replyTag(PARENT_ID), pTag(PUBKEY)]);
  assert.equal(
    needsResolvedParentAuthor(event, PUBKEY, parentOpts({ isDmChannel: true })),
    false,
  );
});

test("no escalation once the cache already answered", () => {
  const event = makeEvent([replyTag(PARENT_ID), pTag(PUBKEY)]);
  assert.equal(
    needsResolvedParentAuthor(
      event,
      PUBKEY,
      parentOpts({ cachedParentAuthor: OTHER_PUBKEY }),
    ),
    false,
  );
});

test("no escalation for a reply that does not p-tag the user", () => {
  const event = makeEvent([replyTag(PARENT_ID), pTag(OTHER_PUBKEY)]);
  assert.equal(needsResolvedParentAuthor(event, PUBKEY, parentOpts()), false);
});

test("no escalation for a top-level message", () => {
  const event = makeEvent([pTag(PUBKEY)]);
  assert.equal(needsResolvedParentAuthor(event, PUBKEY, parentOpts()), false);
});

test("a broadcast reply answering us is still an authored mention", () => {
  // shouldNotifyForEvent admits broadcast replies before it ever reads the
  // parent, and the live thread-reply path never sees them. Demoting one here
  // left the community badge and the Home feed disagreeing about the event.
  const event = makeEvent([replyTag(PARENT_ID), pTag(PUBKEY), broadcastTag()]);
  assert.equal(hasAuthoredMentionForEvent(event, PUBKEY, PUBKEY), true);
});

test("a non-broadcast reply answering us is still not an authored mention", () => {
  const event = makeEvent([replyTag(PARENT_ID), pTag(PUBKEY)]);
  assert.equal(hasAuthoredMentionForEvent(event, PUBKEY, PUBKEY), false);
});

test("a muted DM notifies for a reply answering us, like any other DM message", () => {
  // Every DM message p-tags both participants. Demoting the addressing tag
  // there silenced answers to you while new messages still came through.
  const event = makeEvent([replyTag(PARENT_ID), pTag(PUBKEY)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({
        mutedChannelIds: new Set([CHANNEL_ID]),
        channelId: CHANNEL_ID,
        parentAuthorPubkey: PUBKEY,
        isDmChannel: true,
      }),
    ),
    true,
  );
});

test("outside a DM the same reply is still suppressed by the channel mute", () => {
  const event = makeEvent([replyTag(PARENT_ID), pTag(PUBKEY)]);
  assert.equal(
    shouldNotifyForEvent(
      event,
      PUBKEY,
      opts({
        mutedChannelIds: new Set([CHANNEL_ID]),
        channelId: CHANNEL_ID,
        parentAuthorPubkey: PUBKEY,
        isDmChannel: false,
      }),
    ),
    false,
  );
});

test("a reply answering us is not high priority", () => {
  // High priority marks the whole channel, and the home badge then drops
  // top-level items there — so a stray reply could hide an approval request.
  const event = makeEvent([replyTag(PARENT_ID), pTag(PUBKEY)]);
  assert.equal(isHighPriorityEventForUser(event, PUBKEY, PUBKEY), false);
});

test("a real mention on someone else's reply is still high priority", () => {
  const event = makeEvent([replyTag(PARENT_ID), pTag(PUBKEY)]);
  assert.equal(isHighPriorityEventForUser(event, PUBKEY, OTHER_PUBKEY), true);
});

test("a reply with an unresolved parent is not high priority", () => {
  // Callers resolve the parent for exactly the replies that tag the user, so a
  // missing author means the lookup failed. High priority is persisted and
  // drops the channel's top-level items from the dock badge, so it fails
  // closed — unlike notification delivery, which fails open.
  const event = makeEvent([replyTag(PARENT_ID), pTag(PUBKEY)]);
  assert.equal(isHighPriorityEventForUser(event, PUBKEY), false);
});

test("a top-level p-tag is high priority with no parent to resolve", () => {
  assert.equal(
    isHighPriorityEventForUser(makeEvent([pTag(PUBKEY)]), PUBKEY),
    true,
  );
});

test("a broadcast reply is high priority even with an unresolved parent", () => {
  const event = makeEvent([replyTag(PARENT_ID), pTag(PUBKEY), broadcastTag()]);
  assert.equal(isHighPriorityEventForUser(event, PUBKEY), true);
});

test("a muted DM does not escalate to a parent lookup it would ignore", () => {
  const event = makeEvent([replyTag(PARENT_ID), pTag(PUBKEY)]);
  assert.equal(
    needsResolvedParentAuthor(event, PUBKEY, parentOpts({ isDmChannel: true })),
    false,
  );
});

test("a thread you were mentioned in keeps notifying, and muting it still wins", () => {
  // `isNotifiedForThread` counts a mention as following the thread: it renders
  // "Following" and removes the Follow action. This gate has to agree, or the
  // user is told they are subscribed to a thread that never notifies again and
  // the control that would have subscribed them is gone.
  const laterReply = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    pTag(OTHER_PUBKEY),
  ]);

  assert.equal(
    shouldNotifyForEvent(laterReply, PUBKEY, opts()),
    false,
    "a thread we have no relationship with stays silent",
  );

  assert.equal(
    shouldNotifyForEvent(
      laterReply,
      PUBKEY,
      opts({ mentionedRootIds: new Set([ROOT_ID]) }),
    ),
    true,
    "a reply in a thread we were mentioned in notifies",
  );

  // Same precedence isNotifiedForThread applies: mutedRootIds short-circuits
  // the whole membership test, so an explicit thread mute outranks the mention
  // that subscribed us.
  assert.equal(
    shouldNotifyForEvent(
      laterReply,
      PUBKEY,
      opts({
        mentionedRootIds: new Set([ROOT_ID]),
        mutedRootIds: new Set([ROOT_ID]),
      }),
    ),
    false,
    "muting the thread outranks the mention that subscribed us",
  );
});

test("a sender's marker settles the mention question without the parent", () => {
  const muted = { mutedChannelIds: new Set(["c1"]), channelId: "c1" };

  // Marked as addressing: a plain reply. Does not pierce the channel mute,
  // and — this is the payoff — needs no parent lookup to know that.
  const addressed = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    [...pTag(PUBKEY), "", "reply"],
  ]);
  assert.equal(
    shouldNotifyForEvent(addressed, PUBKEY, opts(muted)),
    false,
    "an addressing tag does not pierce a channel mute",
  );
  assert.equal(
    needsResolvedParentAuthor(addressed, PUBKEY, { cachedParentAuthor: null }),
    false,
    "the marker removes the round trip",
  );
  assert.equal(hasAuthoredMentionForEvent(addressed, PUBKEY), false);
  assert.equal(isHighPriorityEventForUser(addressed, PUBKEY, null), false);

  // Marked as a mention: pierces the mute even though the parent is ours.
  // This is the case the parent lookup cannot decide — passing PUBKEY as the
  // parent author would previously have forced "reply".
  const typed = makeEvent([
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    [...pTag(PUBKEY), "", "mention"],
  ]);
  assert.equal(
    shouldNotifyForEvent(typed, PUBKEY, {
      ...opts(muted),
      parentAuthorPubkey: PUBKEY,
    }),
    true,
    "a typed mention pierces the mute even when it answers our own message",
  );
  assert.equal(hasAuthoredMentionForEvent(typed, PUBKEY, PUBKEY), true);
  assert.equal(isHighPriorityEventForUser(typed, PUBKEY, PUBKEY), true);

  // Unmarked stays exactly as it was: ask the parent.
  const bare = makeEvent([rootTag(ROOT_ID), replyTag(PARENT_ID), pTag(PUBKEY)]);
  assert.equal(
    needsResolvedParentAuthor(bare, PUBKEY, { cachedParentAuthor: null }),
    true,
    "an unmarked tag still needs the parent",
  );
  assert.equal(hasAuthoredMentionForEvent(bare, PUBKEY, PUBKEY), false);
  assert.equal(hasAuthoredMentionForEvent(bare, PUBKEY, OTHER_PUBKEY), true);
});
