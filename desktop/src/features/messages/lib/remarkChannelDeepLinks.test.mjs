import assert from "node:assert/strict";
import test from "node:test";

import remarkChannelDeepLinks from "./remarkChannelDeepLinks.ts";

function run(value) {
  const tree = {
    type: "root",
    children: [{ type: "paragraph", children: [{ type: "text", value }] }],
  };
  remarkChannelDeepLinks()(tree);
  return tree.children[0].children;
}

test("turns a bare channel deep link into a custom node", () => {
  const children = run(
    "Open buzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32 now",
  );
  assert.equal(children[1].type, "channel-deep-link");
  assert.equal(
    children[1].value,
    "buzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32",
  );
});

test("peels trailing sentence punctuation", () => {
  const children = run(
    "Open buzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32.",
  );
  assert.equal(
    children[1].value,
    "buzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32",
  );
  assert.equal(children[2].value, ".");
});
