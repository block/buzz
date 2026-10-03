import assert from "node:assert/strict";
import test from "node:test";

import { liveThreadReplyCacheRootId } from "./threading.ts";

const ROOT = "a".repeat(64);
const PARENT = "b".repeat(64);

test("live thread cache keeps a real thread reply under its root", () => {
  assert.equal(
    liveThreadReplyCacheRootId([
      ["e", ROOT, "", "root"],
      ["e", PARENT, "", "reply"],
    ]),
    ROOT,
  );
});

test("live thread cache skips broadcast replies so they stay on the main timeline", () => {
  assert.equal(
    liveThreadReplyCacheRootId([
      ["e", ROOT, "", "root"],
      ["e", PARENT, "", "reply"],
      ["broadcast", "1"],
    ]),
    null,
  );
});

test("live thread cache skips events that are not thread replies", () => {
  assert.equal(liveThreadReplyCacheRootId([["e", ROOT, "", "root"]]), null);
});
