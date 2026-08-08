import assert from "node:assert/strict";
import test from "node:test";

import {
  filterChannelsByQuery,
  sortChannelsMembersFirst,
} from "./channelPickerOrdering.ts";

function channel(name, { isMember = false, description = "" } = {}) {
  return { name, description, isMember };
}

test("sortChannelsMembersFirst: joined channels come first", () => {
  const sorted = sortChannelsMembersFirst([
    channel("zulu", { isMember: false }),
    channel("alpha", { isMember: false }),
    channel("yankee", { isMember: true }),
  ]);
  assert.deepEqual(
    sorted.map((entry) => entry.name),
    ["yankee", "alpha", "zulu"],
  );
});

test("sortChannelsMembersFirst: alphabetical within each membership group", () => {
  const sorted = sortChannelsMembersFirst([
    channel("bravo", { isMember: true }),
    channel("Alpha", { isMember: true }),
    channel("delta", { isMember: false }),
    channel("Charlie", { isMember: false }),
  ]);
  assert.deepEqual(
    sorted.map((entry) => entry.name),
    ["Alpha", "bravo", "Charlie", "delta"],
  );
});

test("sortChannelsMembersFirst: does not mutate the input", () => {
  const input = [channel("b"), channel("a")];
  sortChannelsMembersFirst(input);
  assert.deepEqual(
    input.map((entry) => entry.name),
    ["b", "a"],
  );
});

test("filterChannelsByQuery: empty query returns the incoming order", () => {
  const channels = [channel("bravo"), channel("alpha")];
  assert.equal(filterChannelsByQuery(channels, ""), channels);
  assert.equal(filterChannelsByQuery(channels, "   "), channels);
});

test("filterChannelsByQuery: drops non-matching channels", () => {
  const filtered = filterChannelsByQuery(
    [channel("release-notes"), channel("general")],
    "rel",
  );
  assert.deepEqual(
    filtered.map((entry) => entry.name),
    ["release-notes"],
  );
});

test("filterChannelsByQuery: better matches rank first", () => {
  const filtered = filterChannelsByQuery(
    [channel("team-general"), channel("general")],
    "general",
  );
  assert.deepEqual(
    filtered.map((entry) => entry.name),
    ["general", "team-general"],
  );
});

test("filterChannelsByQuery: equal scores keep members-first order", () => {
  const filtered = filterChannelsByQuery(
    sortChannelsMembersFirst([
      channel("eng-platform", { isMember: false }),
      channel("eng-mobile", { isMember: true }),
    ]),
    "eng",
  );
  assert.deepEqual(
    filtered.map((entry) => entry.name),
    ["eng-mobile", "eng-platform"],
  );
});

test("filterChannelsByQuery: matches descriptions as a fallback", () => {
  const filtered = filterChannelsByQuery(
    [
      channel("watercooler", { description: "off-topic chatter" }),
      channel("general"),
    ],
    "chatter",
  );
  assert.deepEqual(
    filtered.map((entry) => entry.name),
    ["watercooler"],
  );
});
