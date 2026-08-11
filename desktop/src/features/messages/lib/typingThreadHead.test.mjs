import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { getTypingThreadHeadId } from "./threading.ts";

describe("getTypingThreadHeadId", () => {
  it("returns null for channel-scoped typing (no e tags)", () => {
    assert.equal(getTypingThreadHeadId([["h", "chan-1"]]), null);
  });

  it("uses parent when reply has no separate root (depth-1)", () => {
    assert.equal(
      getTypingThreadHeadId([
        ["h", "chan-1"],
        ["e", "root-1", "", "reply"],
      ]),
      "root-1",
    );
  });

  it("prefers root over nested parent so open-thread indicators stay lit", () => {
    assert.equal(
      getTypingThreadHeadId([
        ["h", "chan-1"],
        ["e", "root-1", "", "root"],
        ["e", "nested-reply", "", "reply"],
      ]),
      "root-1",
    );
  });
});
