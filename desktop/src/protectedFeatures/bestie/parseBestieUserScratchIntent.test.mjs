import assert from "node:assert/strict";
import test from "node:test";

import { parseBestieUserScratchIntent } from "./parseBestieUserScratchIntent.ts";

test("parseBestieUserScratchIntent parks single-line notes", () => {
  const intent = parseBestieUserScratchIntent("park this: call the dentist");
  assert.equal(intent?.op, "add");
  assert.equal(intent?.note.body, "call the dentist");
});

test("parseBestieUserScratchIntent parks titled multiline", () => {
  const intent = parseBestieUserScratchIntent(
    "save to scratch: Dentist\nCall before Friday",
  );
  assert.equal(intent?.op, "add");
  assert.equal(intent?.note.title, "Dentist");
  assert.equal(intent?.note.body, "Call before Friday");
});

test("parseBestieUserScratchIntent removes by title", () => {
  const intent = parseBestieUserScratchIntent('remove scratch "Dentist"');
  assert.equal(intent?.op, "remove-match");
  assert.equal(intent?.title, "Dentist");
});

test("parseBestieUserScratchIntent ignores unrelated chat", () => {
  assert.equal(parseBestieUserScratchIntent("remind me to stretch"), null);
});
