import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { getTypingScopeId } from "./useChannelTyping.ts";

const ROOT = "aaaa".repeat(16);
const PARENT = "bbbb".repeat(16);
const CHANNEL = "11111111-1111-1111-1111-111111111111";

describe("getTypingScopeId", () => {
  it("returns null for channel-scoped typing (no e tags)", () => {
    assert.equal(getTypingScopeId({ tags: [["h", CHANNEL]] }), null);
  });

  it("uses the reply parent for direct replies to the thread head", () => {
    // Direct replies tag only the parent (root === parent, so no root tag).
    assert.equal(
      getTypingScopeId({
        tags: [
          ["h", CHANNEL],
          ["e", ROOT, "", "reply"],
        ],
      }),
      ROOT,
    );
  });

  it("prefers the thread root over the reply parent for nested replies", () => {
    // Nested replies tag both root and their immediate parent; thread
    // surfaces (ingress badge, open-thread composer) key on the root.
    assert.equal(
      getTypingScopeId({
        tags: [
          ["h", CHANNEL],
          ["e", ROOT, "", "root"],
          ["e", PARENT, "", "reply"],
        ],
      }),
      ROOT,
    );
  });
});
