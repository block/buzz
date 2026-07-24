import assert from "node:assert/strict";
import test from "node:test";

import remarkGroupMentions from "./remarkGroupMentions.ts";
import remarkMentions from "./remarkMentions.ts";

test("group marker handle produces one distinct group mention node", () => {
  const tree = {
    type: "root",
    children: [
      {
        type: "paragraph",
        children: [{ type: "text", value: "Hello @ios-team" }],
      },
    ],
  };

  remarkGroupMentions({ groupHandles: ["ios-team"] })(tree);
  remarkMentions({ mentionNames: ["ios-team"] })(tree);

  const children = tree.children[0].children;
  assert.equal(children.length, 2);
  assert.equal(children[1].type, "groupMention");
  assert.equal(children[1].data.hName, "group-mention");
  assert.deepEqual(children[1].data.hChildren, [
    { type: "text", value: "@ios-team" },
  ]);
});
