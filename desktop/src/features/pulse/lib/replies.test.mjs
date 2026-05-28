import assert from "node:assert/strict";
import test from "node:test";

import { getReplyParent } from "./replies.ts";

function note(tags) {
  return {
    id: "note",
    pubkey: "a".repeat(64),
    createdAt: 0,
    content: "",
    tags,
  };
}

const A = "a".repeat(64);
const B = "b".repeat(64);
const C = "c".repeat(64);

test("getReplyParent prefers the last marked reply tag", () => {
  assert.equal(
    getReplyParent(
      note([
        ["e", A, "", "reply"],
        ["e", B, "", "reply"],
      ]),
    ),
    B,
  );
});

test("getReplyParent falls back to the last unmarked e tag", () => {
  assert.equal(
    getReplyParent(
      note([
        ["e", A],
        ["e", B],
      ]),
    ),
    B,
  );
});

test("getReplyParent uses a root marker when no closer parent exists", () => {
  assert.equal(getReplyParent(note([["e", A, "", "root"]])), A);
});

test("getReplyParent uses reply before later root", () => {
  assert.equal(
    getReplyParent(
      note([
        ["e", A, "", "reply"],
        ["e", B, "", "root"],
      ]),
    ),
    A,
  );
});

test("getReplyParent keeps reply precedence over root and unmarked fallbacks", () => {
  assert.equal(
    getReplyParent(
      note([
        ["e", A, "", "root"],
        ["e", B],
        ["e", C, "", "reply"],
      ]),
    ),
    C,
  );
});

test("getReplyParent returns null without e tags", () => {
  assert.equal(getReplyParent(note([])), null);
});
