import assert from "node:assert/strict";
import test from "node:test";
import {
  buildReplyTags,
  buildThreadReferenceTags,
  getThreadReference,
  getReplyContextId,
} from "./threading.ts";

for (const parent of ["root", "response"]) {
  test(`signed/optimistic reply builders use root with context for ${parent}`, () => {
    for (const tags of [
      buildReplyTags("channel", "author", parent, "root", ["person"]),
      buildThreadReferenceTags("channel", parent, "root"),
    ]) {
      assert.deepEqual(getThreadReference(tags), {
        rootId: "root",
        parentId: "root",
      });
      assert.equal(getReplyContextId(tags), parent === "root" ? null : parent);
      assert.ok(tags.some((tag) => tag[0] === "h" && tag[1] === "channel"));
    }
    assert.ok(
      buildReplyTags("channel", "author", parent, "root", ["person"]).some(
        (tag) => tag[0] === "p" && tag[1] === "person",
      ),
    );
  });
}
test("historical nested replies retain their response context", () => {
  assert.equal(
    getReplyContextId([
      ["e", "root", "", "root"],
      ["e", "parent", "", "reply"],
    ]),
    "parent",
  );
  assert.equal(getReplyContextId([["e", "root", "", "reply"]]), null);
});
