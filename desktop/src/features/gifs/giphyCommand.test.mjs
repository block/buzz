import assert from "node:assert/strict";
import test from "node:test";

import { parseGiphyCommand } from "./giphyCommand.ts";

test("parseGiphyCommand returns the query after /giphy", () => {
  assert.equal(parseGiphyCommand("/giphy ship it"), "ship it");
  assert.equal(parseGiphyCommand("  /GIPHY   Party Parrot  "), "Party Parrot");
});

test("parseGiphyCommand treats a bare /giphy as a trending browse", () => {
  assert.equal(parseGiphyCommand("/giphy"), "");
  assert.equal(parseGiphyCommand("/giphy   "), "");
});

test("parseGiphyCommand keeps the user's backslashes", () => {
  assert.equal(parseGiphyCommand("/giphy C:\\temp"), "C:\\temp");
});

test("parseGiphyCommand ignores text that is not the command", () => {
  assert.equal(parseGiphyCommand("hello /giphy cats"), null);
  assert.equal(parseGiphyCommand("/giphycats"), null);
  assert.equal(parseGiphyCommand("/gif cats"), null);
  assert.equal(parseGiphyCommand("/giphy cats\nand dogs"), null);
  assert.equal(parseGiphyCommand(""), null);
});
