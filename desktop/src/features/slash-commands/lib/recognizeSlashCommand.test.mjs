import assert from "node:assert/strict";
import test from "node:test";

import { recognizeSlashCommand } from "./recognizeSlashCommand.ts";
import { tokenizeSlashArgs } from "./tokenizeSlashArgs.ts";

const COMMANDS = [
  { name: "away" },
  { name: "status" },
  { name: "remove", aliases: ["kick"] },
  { name: "shrug" },
  { name: "leave", aliases: ["part"] },
];

const recognizeRows = [
  ["/away", "away", ""],
  ["/AWAY", "away", ""],
  ["  /Away  ", "away", ""],
  ["\n/away", "away", ""],
  ["/status :tada: shipped", "status", ":tada: shipped"],
  ["/status\n  multi\nline", "status", "multi\nline"],
  ["/shrug   spaced   out  ", "shrug", "spaced   out"],
  ["/kick @sam", "remove", "@sam"],
  ["/KICK @sam", "remove", "@sam"],
  ["/part", "leave", ""],
  ["/remove @sam @alex", "remove", "@sam @alex"],
];

for (const [draft, name, args] of recognizeRows) {
  test(`recognizes ${JSON.stringify(draft)}`, () => {
    const result = recognizeSlashCommand(draft, COMMANDS);
    assert.equal(result?.command.name, name);
    assert.equal(result?.args, args);
  });
}

const passThroughRows = [
  ["@Eva /status", "mention first: the agent's command"],
  ["@Eva Smith /away", "multi-word mention first"],
  ["/review hello", "unknown command goes to the agent"],
  ["/usr/bin/foo", "path-like"],
  ["//away", "double slash"],
  ["/away-now", "name continues past a registered one"],
  ["/awayx", "prefix of the token only"],
  ["/awa", "partial name"],
  ["/", "bare slash"],
  ["/ away", "slash then space"],
  ["hello /away", "not the first token"],
  ["", "empty"],
  ["   ", "blank"],
];

for (const [draft, why] of passThroughRows) {
  test(`passes through ${JSON.stringify(draft)} (${why})`, () => {
    assert.equal(recognizeSlashCommand(draft, COMMANDS), null);
  });
}

test("only registered commands are recognized: an empty registry runs nothing", () => {
  assert.equal(recognizeSlashCommand("/away", []), null);
});

const tokenizeRows = [
  ["", []],
  ["   ", []],
  ["one", ["one"]],
  ["one two  three", ["one", "two", "three"]],
  ["  lead trail  ", ["lead", "trail"]],
  ['"two words" next', ["two words", "next"]],
  ['a "b c" d', ["a", "b c", "d"]],
  ['"" x', ["", "x"]],
  ['say "he said \\"hi\\""', ["say", 'he said "hi"']],
  ['"back\\\\slash"', ["back\\slash"]],
  ['"unterminated quote', ["unterminated quote"]],
  ["it's fine", ["it's", "fine"]],
  ["tab\tand\nnewline", ["tab", "and", "newline"]],
  ["@sam #general", ["@sam", "#general"]],
];

for (const [input, expected] of tokenizeRows) {
  test(`tokenizes ${JSON.stringify(input)}`, () => {
    assert.deepEqual(tokenizeSlashArgs(input), expected);
  });
}
