import assert from "node:assert/strict";
import test from "node:test";

import { WELCOME_CANVAS_CONTENT } from "./welcomeCanvas.ts";

test("welcome canvas covers purpose, agent use, a first challenge, and help", () => {
  assert.match(WELCOME_CANVAS_CONTENT, /private channel is your home base/i);
  assert.match(WELCOME_CANVAS_CONTENT, /Mention an agent/i);
  assert.match(WELCOME_CANVAS_CONTENT, /quick challenge/i);
  assert.match(WELCOME_CANVAS_CONTENT, /Buzz user guide/i);
});
