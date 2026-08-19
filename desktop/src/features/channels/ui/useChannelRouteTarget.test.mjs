import assert from "node:assert/strict";
import test from "node:test";

import { isThreadRouteTargetReady } from "./useChannelRouteTarget.ts";

const ROOT = { id: "root", parentId: null, rootId: null, tags: [] };
const PARENT = { id: "parent", parentId: "root", rootId: "root", tags: [] };
const LEAF = { id: "leaf", parentId: "parent", rootId: "root", tags: [] };

test("thread route readiness waits for the routed message and every ancestor", () => {
  assert.equal(isThreadRouteTargetReady("leaf", null, new Map()), false);
  assert.equal(
    isThreadRouteTargetReady("leaf", LEAF, new Map([["leaf", LEAF]])),
    false,
  );
  assert.equal(
    isThreadRouteTargetReady(
      "leaf",
      LEAF,
      new Map([
        ["root", ROOT],
        ["parent", PARENT],
        ["leaf", LEAF],
      ]),
    ),
    true,
  );
});

test("root routes are ready as soon as the root message arrives", () => {
  assert.equal(
    isThreadRouteTargetReady("root", ROOT, new Map([["root", ROOT]])),
    true,
  );
  assert.equal(isThreadRouteTargetReady(null, null, new Map()), true);
});
