import assert from "node:assert/strict";
import test from "node:test";

import { parseDmArgs } from "./parseDmArgs.ts";

const range = (pubkey, start, end) => ({ pubkey, start, end });

const rows = [
  ["no mention", "hey there", [], { recipients: [], message: "hey there" }],
  [
    "only a mention",
    "@Al",
    [range("a", 0, 3)],
    { recipients: ["a"], message: "" },
  ],
  [
    "mention then message",
    "@Al hey",
    [range("a", 0, 3)],
    { recipients: ["a"], message: "hey" },
  ],
  [
    "several leading mentions",
    "@Al   @Bo hey",
    [range("a", 0, 3), range("b", 6, 9)],
    { recipients: ["a", "b"], message: "hey" },
  ],
  [
    "a later mention is plain text",
    "@Al hi @Bo",
    [range("a", 0, 3), range("b", 7, 10)],
    { recipients: ["a"], message: "hi @Bo" },
  ],
  [
    "text before the first mention leaves no recipient",
    "hi @Al",
    [range("a", 3, 6)],
    { recipients: [], message: "hi @Al" },
  ],
  [
    "an unresolved leading name ends the recipients",
    "@Al @Ghost @Bo hi",
    [range("a", 0, 3), range("b", 11, 14)],
    { recipients: ["a"], message: "@Ghost @Bo hi" },
  ],
];

for (const [name, args, ranges, expected] of rows) {
  test(`parseDmArgs: ${name}`, () => {
    assert.deepEqual(parseDmArgs(args, ranges), expected);
  });
}
