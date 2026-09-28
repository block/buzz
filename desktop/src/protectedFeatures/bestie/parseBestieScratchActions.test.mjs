import assert from "node:assert/strict";
import test from "node:test";

import { parseBestieScratchActionsFromMessage } from "./parseBestieScratchActions.ts";

test("parseBestieScratchActionsFromMessage reads fence", () => {
  const actions = parseBestieScratchActionsFromMessage(`
Thanks — parked.

\`\`\`bestie-scratch
{"op":"add","title":"Idea","body":"later"}
\`\`\`
`);
  assert.equal(actions.length, 1);
  assert.equal(actions[0].op, "add");
  assert.equal(actions[0].note.title, "Idea");
});
