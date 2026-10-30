import assert from "node:assert/strict";
import test from "node:test";

import {
  selectForwardPeopleTargets,
  selectForwardTargets,
} from "./ForwardMessageDialog.tsx";

const SOURCE_CHANNEL = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
const RELAY_SELF = "f".repeat(64);
const ME = "a".repeat(64);
const ALICE = "b".repeat(64);
const BOB = "c".repeat(64);

function channel(overrides = {}) {
  return {
    id: "7c9e6679-7425-40de-944b-e07fc1f90ae7",
    name: "general",
    channelType: "stream",
    visibility: "public",
    description: "",
    isMember: true,
    archivedAt: null,
    participantPubkeys: [],
    memberPubkeys: [],
    ...overrides,
  };
}

const targets = (channels, query = "", extra = {}) =>
  selectForwardTargets({
    channels,
    query,
    sourceChannelId: SOURCE_CHANNEL,
    ...extra,
  });
const names = (channels) => channels.map((c) => c.name);

test("the source channel is not offered as a forward destination", () => {
  const channels = [
    channel({ id: SOURCE_CHANNEL, name: "here" }),
    channel({ name: "there" }),
  ];
  assert.deepEqual(names(targets(channels)), ["there"]);
  // The id comparison is case-insensitive; a tag-normalized id must still
  // match a channel list that kept the relay's casing.
  assert.deepEqual(
    names(
      selectForwardTargets({
        channels,
        query: "",
        sourceChannelId: SOURCE_CHANNEL.toUpperCase(),
      }),
    ),
    ["there"],
  );
});

test("destinations a forward could not reach are not offered", () => {
  const channels = [
    channel({ id: "a", name: "forum-post", channelType: "forum" }),
    channel({ id: "b", name: "left", isMember: false }),
    channel({ id: "c", name: "archived", archivedAt: "2026-01-01T00:00:00Z" }),
    channel({ id: "d", name: "open" }),
    channel({ id: "e", name: "alice", channelType: "dm" }),
  ];
  assert.deepEqual(names(targets(channels)), ["alice", "open"]);
});

test("the moderation DM with the relay is not offered as a destination", () => {
  // The viewer cannot write into it, so offering it only produces a rejected
  // send and a silently lost forward.
  const channels = [
    channel({
      id: "mod",
      name: "relay",
      channelType: "dm",
      participantPubkeys: [ME, RELAY_SELF],
    }),
    channel({
      id: "dm",
      name: "alice",
      channelType: "dm",
      participantPubkeys: [ME, ALICE],
    }),
  ];

  assert.deepEqual(
    names(
      targets(channels, "", { currentPubkey: ME, relaySelfPubkey: RELAY_SELF }),
    ),
    ["alice"],
  );
  // Without a known relay identity the predicate fails open, as everywhere
  // else: an unreachable relay must not hide ordinary DMs.
  assert.deepEqual(names(targets(channels, "", { currentPubkey: ME })), [
    "alice",
    "relay",
  ]);
});

test("a hidden huddle backing channel is not offered as a destination", () => {
  const channels = [
    channel({ id: "huddle-backing", name: "huddle" }),
    channel({ id: "open", name: "open" }),
  ];
  assert.deepEqual(
    names(
      targets(channels, "", { hiddenChannelIds: new Set(["huddle-backing"]) }),
    ),
    ["open"],
  );
  // A revealed backing channel is not in the hidden set, so it stays offered.
  assert.deepEqual(
    names(targets(channels, "", { hiddenChannelIds: new Set() })),
    ["huddle", "open"],
  );
});

test("search narrows destinations and ranks the better name match first", () => {
  const channels = [
    channel({ id: "a", name: "design-review" }),
    channel({ id: "b", name: "design" }),
    channel({ id: "c", name: "random", description: "design chatter" }),
    channel({ id: "d", name: "ops" }),
  ];

  const matches = names(targets(channels, "design"));
  assert.deepEqual(matches, ["design", "design-review", "random"]);
  assert.deepEqual(names(targets(channels, "  DESIGN  ")), matches);
  assert.deepEqual(targets(channels, "nothing-matches"), []);
});

test("equally scored destinations are ordered by name, not by list order", () => {
  const channels = [
    channel({ id: "a", name: "zulu" }),
    channel({ id: "b", name: "alpha" }),
  ];
  assert.deepEqual(names(targets(channels)), ["alpha", "zulu"]);
});

function person(overrides = {}) {
  return {
    pubkey: ALICE,
    displayName: "Alice",
    avatarUrl: null,
    nip05Handle: null,
    ownerPubkey: null,
    isAgent: false,
    ...overrides,
  };
}

const people = (candidates, query = "al", extra = {}) =>
  selectForwardPeopleTargets({
    candidates,
    currentPubkey: ME,
    query,
    ...extra,
  }).map((candidate) => candidate.pubkey);

test("a person with no shared channel is still a forward destination", () => {
  assert.deepEqual(people([person()]), [ALICE]);
});

test("people a forward must not be offered to are filtered out", () => {
  assert.deepEqual(
    people([
      person({ pubkey: ME, displayName: "Alfred" }),
      person({ pubkey: BOB, displayName: "Albot", isAgent: true }),
      person(),
    ]),
    [ALICE],
  );
  // Someone already reachable through an offered DM channel appears as that
  // channel, not a second time as a person.
  assert.deepEqual(
    people([person()], "al", {
      excludedPubkeys: new Set([ALICE]),
    }),
    [],
  );
  assert.deepEqual(
    people([person()], "al", { isArchived: (pubkey) => pubkey === ALICE }),
    [],
  );
  // A DM with the relay identity is the moderation channel; the viewer cannot
  // write into it, so the relay is not a person to forward to.
  assert.deepEqual(
    people([person({ pubkey: RELAY_SELF, displayName: "Alpha relay" })], "al", {
      relaySelfPubkey: RELAY_SELF.toUpperCase(),
    }),
    [],
  );
});

test("people are ranked by match quality and capped", () => {
  assert.deepEqual(
    people([
      person({ pubkey: BOB, displayName: "Not Alice" }),
      person({ displayName: "Alice" }),
    ]),
    [ALICE, BOB],
  );
  // The directory can return more matches than a picker should scroll.
  assert.equal(
    people(
      Array.from({ length: 24 }, (_, index) =>
        person({
          pubkey: index.toString(16).padStart(64, "0"),
          displayName: `Al ${index}`,
        }),
      ),
    ).length,
    10,
  );
  // Nothing is offered before the viewer types: the unsearched view is their
  // own conversations, not the relay directory.
  assert.deepEqual(people([person()], ""), []);
  assert.deepEqual(people([person({ displayName: "Zoe" })]), []);
});
