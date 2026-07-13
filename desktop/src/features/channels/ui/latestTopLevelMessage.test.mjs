import assert from "node:assert/strict";
import test from "node:test";

import { latestTopLevelMessage } from "./latestTopLevelMessage.ts";

const message = (id, tags = []) => ({ id, tags });

test("latestTopLevelMessage skips newer thread replies", () => {
  const root = message("root");
  const reply = message("reply", [["e", "root", "", "reply"]]);

  assert.equal(latestTopLevelMessage([root, reply]), root);
});

test("latestTopLevelMessage handles empty and reply-only timelines", () => {
  const reply = message("reply", [["e", "root", "", "reply"]]);

  assert.equal(latestTopLevelMessage(undefined), null);
  assert.equal(latestTopLevelMessage([]), null);
  assert.equal(latestTopLevelMessage([reply]), null);
});
