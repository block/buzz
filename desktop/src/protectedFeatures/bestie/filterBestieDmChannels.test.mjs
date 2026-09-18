import assert from "node:assert/strict";
import test from "node:test";

import { filterBestieDmChannels } from "./filterBestieDmChannels.ts";

const SELF = "AA";
const BESTIE = "BB";
const OTHER = "CC";

function makeDm(id, participantPubkeys) {
  return {
    archivedAt: null,
    channelType: "dm",
    description: "",
    id,
    isMember: true,
    lastMessageAt: null,
    memberCount: participantPubkeys.length,
    memberPubkeys: participantPubkeys,
    name: id,
    participantPubkeys,
    participants: participantPubkeys,
    purpose: null,
    topic: null,
    ttlDeadline: null,
    ttlSeconds: null,
    visibility: "private",
  };
}

test("filters only the one-to-one DM with the designated Bestie", () => {
  const bestieDm = makeDm("bestie", [BESTIE, SELF]);
  const otherDm = makeDm("other", [SELF, OTHER]);
  const groupWithBestie = makeDm("group", [SELF, BESTIE, OTHER]);

  assert.deepEqual(
    filterBestieDmChannels(
      [bestieDm, otherDm, groupWithBestie],
      SELF.toLowerCase(),
      BESTIE.toLowerCase(),
    ).map(({ id }) => id),
    ["other", "group"],
  );
});

test("preserves the DM list until identity and assignment are available", () => {
  const channels = [makeDm("bestie", [SELF, BESTIE])];

  assert.equal(filterBestieDmChannels(channels, undefined, BESTIE), channels);
  assert.equal(filterBestieDmChannels(channels, SELF, null), channels);
});
