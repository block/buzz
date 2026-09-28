import assert from "node:assert/strict";
import test from "node:test";

import {
  applyAutomaticSmartSectionAssignments,
  assignChannelsToSection,
  channelMatchesSmartRule,
  getSmartSectionRuleError,
  matchingChannelsForSection,
  manuallyUnassignChannel,
} from "./smartChannelSections.ts";

function smartSection(overrides = {}) {
  return {
    id: "smart-1",
    name: "Engineering",
    order: 0,
    smartRule: {
      pattern: "eng-",
      isRegex: false,
      caseSensitive: false,
    },
    ...overrides,
  };
}

function store(overrides = {}) {
  return {
    version: 1,
    sections: [smartSection()],
    assignments: {},
    ...overrides,
  };
}

test("literal smart rules match channel names by substring", () => {
  assert.equal(
    channelMatchesSmartRule("ENG-Platform", {
      pattern: "eng-",
      isRegex: false,
      caseSensitive: false,
    }),
    true,
  );
  assert.equal(
    channelMatchesSmartRule("ENG-Platform", {
      pattern: "eng-",
      isRegex: false,
      caseSensitive: true,
    }),
    false,
  );
});

test("regex rules honor case sensitivity and reject invalid expressions", () => {
  assert.equal(
    channelMatchesSmartRule("eng-platform", {
      pattern: "^ENG-(platform|mobile)$",
      isRegex: true,
      caseSensitive: false,
    }),
    true,
  );
  assert.equal(
    channelMatchesSmartRule("eng-platform", {
      pattern: "^ENG-(platform|mobile)$",
      isRegex: true,
      caseSensitive: true,
    }),
    false,
  );
  assert.equal(
    getSmartSectionRuleError({
      pattern: "[",
      isRegex: true,
      caseSensitive: false,
    }),
    "Enter a valid regular expression.",
  );
});

test("automatic assignment places only channels without a saved location", () => {
  const original = store({
    assignments: { manual: "other", already: "smart-1" },
    manuallyUnassignedChannelIds: ["kept-in-channels"],
  });
  const result = applyAutomaticSmartSectionAssignments(original, [
    { id: "new-match", name: "eng-infra" },
    { id: "manual", name: "eng-manual" },
    { id: "already", name: "eng-existing" },
    { id: "kept-in-channels", name: "eng-opted-out" },
    { id: "not-a-match", name: "design" },
  ]);

  assert.deepEqual(result.assignments, {
    manual: "other",
    already: "smart-1",
    "new-match": "smart-1",
  });
  assert.deepEqual(result.manuallyUnassignedChannelIds, ["kept-in-channels"]);
});

test("the first matching Smart Section by sidebar order wins", () => {
  const result = applyAutomaticSmartSectionAssignments(
    store({
      sections: [
        smartSection({ id: "second", order: 2 }),
        smartSection({ id: "first", order: 1 }),
      ],
    }),
    [{ id: "channel", name: "eng-platform" }],
  );
  assert.equal(result.assignments.channel, "first");
});

test("bulk matching returns matches regardless of current placement", () => {
  const channels = [
    { id: "one", name: "eng-one" },
    { id: "two", name: "design" },
    { id: "three", name: "ENG-three" },
  ];
  assert.deepEqual(
    matchingChannelsForSection(smartSection(), channels).map(
      (channel) => channel.id,
    ),
    ["one", "three"],
  );
});

test("manual moves override automatic placement until explicitly moved back", () => {
  const assigned = store({ assignments: { channel: "smart-1" } });
  const unassigned = manuallyUnassignChannel(assigned, "channel");
  assert.deepEqual(unassigned.assignments, {});
  assert.deepEqual(unassigned.manuallyUnassignedChannelIds, ["channel"]);

  const afterAutomaticScan = applyAutomaticSmartSectionAssignments(unassigned, [
    { id: "channel", name: "eng-platform" },
  ]);
  assert.equal(afterAutomaticScan, unassigned);

  const movedBack = assignChannelsToSection(
    afterAutomaticScan,
    ["channel"],
    "smart-1",
  );
  assert.equal(movedBack.assignments.channel, "smart-1");
  assert.equal(movedBack.manuallyUnassignedChannelIds, undefined);
});
