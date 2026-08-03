import assert from "node:assert/strict";
import test from "node:test";

import { mixMinusRecipients } from "./voiceRoomAudio.ts";

test("mix-minus routes a speaker to every other room participant", () => {
  assert.deepEqual(mixMinusRecipients(["medium", "high", "composer"], "high"), [
    "medium",
    "composer",
  ]);
});

test("mix-minus never feeds a participant its own voice", () => {
  assert.deepEqual(mixMinusRecipients(["medium"], "medium"), []);
});
