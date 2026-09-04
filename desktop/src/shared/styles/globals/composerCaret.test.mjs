import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const composerCss = readFileSync(
  new URL("./composer.css", import.meta.url),
  "utf8",
);

const mentionChipRule = composerCss.match(
  /\.rich-text-composer \.tiptap \.mention-chip\s*\{([\s\S]*?)\n\}/,
)?.[1];

test("composer mention chips stay inline so the caret can sit after the trailing space", () => {
  assert.ok(mentionChipRule, "missing composer mention-chip override");
  assert.match(mentionChipRule, /display:\s*inline;/);
  assert.doesNotMatch(mentionChipRule, /display:\s*inline-flex/);
});
