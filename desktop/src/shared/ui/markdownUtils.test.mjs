import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  markdownPropsAreEqual,
  shallowArrayEqual,
  shallowRecordEqual,
} from "./markdownUtils.ts";

const base = {
  content: "hello",
  className: "prose",
  customEmoji: [],
  hardLineBreaks: true,
  interactive: true,
  blockCode: false,
  mediaInset: false,
  agentMentionPubkeysByName: { bot: "aa" },
  mentionPubkeysByName: { alice: "bb" },
  mentionNames: ["alice"],
  channelNames: ["general"],
  imetaByUrl: new Map(),
  configNudgeAuthorPubkey: null,
  searchQuery: "",
  snapshotSharedBy: undefined,
  videoReviewContext: undefined,
  messageId: "evt-1",
  linkPreviewsSuppressed: false,
  linkPreviewTags: [["r", "https://example.com"]],
  leadingInlineContent: undefined,
  onRemoveLinkPreviewsForEveryone: undefined,
};

/** A different value for every prop the renderer reads. */
const changed = {
  content: "goodbye",
  className: "prose-sm",
  customEmoji: [],
  hardLineBreaks: false,
  interactive: false,
  blockCode: true,
  mediaInset: true,
  agentMentionPubkeysByName: { bot: "cc" },
  mentionPubkeysByName: { alice: "cc" },
  mentionNames: ["bob"],
  channelNames: ["random"],
  imetaByUrl: new Map(),
  configNudgeAuthorPubkey: "dd",
  searchQuery: "term",
  snapshotSharedBy: "alice",
  videoReviewContext: {},
  messageId: "evt-2",
  linkPreviewsSuppressed: true,
  linkPreviewTags: [["r", "https://other.example"]],
  leadingInlineContent: "prefix",
  onRemoveLinkPreviewsForEveryone: async () => {},
};

describe("markdownPropsAreEqual", () => {
  it("treats identical props as equal", () => {
    assert.equal(markdownPropsAreEqual(base, { ...base }), true);
  });

  for (const key of Object.keys(changed)) {
    it(`observes a change to ${key}`, () => {
      assert.equal(
        markdownPropsAreEqual(base, { ...base, [key]: changed[key] }),
        false,
      );
    });
  }

  it("compares mention maps by value, not identity", () => {
    assert.equal(
      markdownPropsAreEqual(base, {
        ...base,
        mentionPubkeysByName: { ...base.mentionPubkeysByName },
      }),
      true,
    );
  });

  it("compares name lists by value, not identity", () => {
    assert.equal(
      markdownPropsAreEqual(base, {
        ...base,
        mentionNames: [...base.mentionNames],
      }),
      true,
    );
  });
});

describe("shallowArrayEqual", () => {
  it("handles missing sides", () => {
    assert.equal(shallowArrayEqual(undefined, undefined), true);
    assert.equal(shallowArrayEqual(["a"], undefined), false);
  });

  it("compares element-wise", () => {
    assert.equal(shallowArrayEqual(["a", "b"], ["a", "b"]), true);
    assert.equal(shallowArrayEqual(["a", "b"], ["b", "a"]), false);
    assert.equal(shallowArrayEqual(["a"], ["a", "b"]), false);
  });
});

describe("shallowRecordEqual", () => {
  it("handles missing sides", () => {
    assert.equal(shallowRecordEqual(undefined, undefined), true);
    assert.equal(shallowRecordEqual({ a: "1" }, undefined), false);
  });

  it("compares entry-wise", () => {
    assert.equal(shallowRecordEqual({ a: "1" }, { a: "1" }), true);
    assert.equal(shallowRecordEqual({ a: "1" }, { a: "2" }), false);
    assert.equal(shallowRecordEqual({ a: "1" }, { a: "1", b: "2" }), false);
  });
});
