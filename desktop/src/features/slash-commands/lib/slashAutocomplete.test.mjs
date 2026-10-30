import assert from "node:assert/strict";
import test from "node:test";

import { composeSlashCommands } from "./composeSlashCommands.ts";
import { detectSlashQuery } from "./detectSlashQuery.ts";
import { filterSlashCommands } from "./filterSlashCommands.ts";
import { resolveChannelLinkIds } from "./resolveChannelLinkIds.ts";

const def = (name, extra = {}) => ({
  name,
  description: name,
  usage: `/${name}`,
  context: "any",
  run: () => {},
  ...extra,
});

const COMMANDS = [
  def("topic"),
  def("status"),
  def("shrug"),
  def("remove", { aliases: ["kick"] }),
  def("rename"),
  def("remind"),
  def("archive", { isVisible: () => false }),
];

const names = (list) => list.map((command) => command.name);

const filterRows = [
  ["", ["remind", "remove", "rename", "shrug", "status", "topic"]],
  ["re", ["remind", "remove", "rename"]],
  ["REM", ["remind", "remove"]],
  ["s", ["shrug", "status"]],
  ["shr", ["shrug"]],
  ["k", []],
  ["kick", []],
  ["hrug", []],
  ["archive", []],
  ["zzz", []],
];

for (const [query, expected] of filterRows) {
  test(`lists ${JSON.stringify(query)} -> ${expected.join(",") || "nothing"}`, () => {
    assert.deepEqual(names(filterSlashCommands(COMMANDS, query)), expected);
  });
}

test("filtering does not reorder or mutate the registry", () => {
  const before = names(COMMANDS);
  filterSlashCommands(COMMANDS, "");
  assert.deepEqual(names(COMMANDS), before);
});

test("a command shows again when its visibility predicate flips", () => {
  let allowed = false;
  const gated = [def("archive", { isVisible: () => allowed })];
  assert.deepEqual(filterSlashCommands(gated, "ar"), []);
  allowed = true;
  assert.deepEqual(names(filterSlashCommands(gated, "ar")), ["archive"]);
});

const detectRows = [
  ["/", 1, { query: "", startIndex: 0, endIndex: 1 }],
  ["/sh", 3, { query: "sh", startIndex: 0, endIndex: 3 }],
  ["/sh", 2, { query: "sh", startIndex: 0, endIndex: 3 }],
  ["  /sh", 5, { query: "sh", startIndex: 2, endIndex: 5 }],
  ["\n/sh", 4, { query: "sh", startIndex: 1, endIndex: 4 }],
  ["/Away-now", 9, { query: "Away-now", startIndex: 0, endIndex: 9 }],
  ["/sh hello", 3, { query: "sh", startIndex: 0, endIndex: 3 }],
  ["/sh hello", 9, null],
  ["/sh hello", 4, null],
  ["/sh", 0, null],
  ["//away", 6, null],
  ["/usr/bin", 8, null],
  ["hello /sh", 9, null],
  ["@Eva /sh", 8, null],
  ["", 0, null],
  ["hello", 5, null],
];

for (const [text, cursor, expected] of detectRows) {
  test(`detects ${JSON.stringify(text)} @${cursor}`, () => {
    assert.deepEqual(detectSlashQuery(text, cursor), expected);
  });
}

test("composing ignores duplicate names and keeps channel commands", () => {
  const groups = [
    [
      def("topic", { context: "channel" }),
      def("mute"),
      def("who", { aliases: ["members"] }),
    ],
    [def("mute"), def("members"), def("kick", { aliases: ["who"] })],
  ];
  assert.deepEqual(names(composeSlashCommands(groups, { enabled: true })), [
    "topic",
    "mute",
    "who",
  ]);
  assert.deepEqual(composeSlashCommands(groups, { enabled: false }), []);
});

test("autocomplete hides channel commands in a DM", () => {
  const commands = [def("topic", { context: "channel" }), def("mute")];
  assert.deepEqual(names(filterSlashCommands(commands, "")), ["mute", "topic"]);
  assert.deepEqual(names(filterSlashCommands(commands, "", { isDm: true })), [
    "mute",
  ]);
});

test("channel-link ids resolve non-DM channels case-insensitively, once, in order", () => {
  const channels = [
    { id: "1", name: "general", channelType: "stream" },
    { id: "2", name: "Random", channelType: "stream" },
    { id: "3", name: "alice-bob", channelType: "dm" },
  ];
  assert.deepEqual(
    resolveChannelLinkIds(
      "join #random and (#GENERAL) #random #alice-bob #nope",
      channels,
    ),
    ["2", "1"],
  );
  assert.deepEqual(resolveChannelLinkIds("no#links here", channels), []);
});
