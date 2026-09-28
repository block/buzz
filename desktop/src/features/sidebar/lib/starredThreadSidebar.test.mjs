import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  countUnreadForStarredThreadRoot,
  formatSidebarUnreadCount,
  shouldSuppressChannelActiveForStarredThread,
  threadFeedConversationId,
} from "./starredThreadSidebar.ts";

function feedItem({ id, rootId = null, parentId = null }) {
  const tags = [];
  if (rootId) tags.push(["e", rootId, "", "root"]);
  if (parentId || rootId) tags.push(["e", parentId ?? rootId, "", "reply"]);
  return { id, tags };
}

describe("threadFeedConversationId", () => {
  it("uses root, then parent, then event id", () => {
    assert.equal(
      threadFeedConversationId(
        feedItem({ id: "reply-1", rootId: "root-a", parentId: "root-a" }),
      ),
      "root-a",
    );
    assert.equal(
      threadFeedConversationId(feedItem({ id: "root-a" })),
      "root-a",
    );
  });
});

describe("countUnreadForStarredThreadRoot", () => {
  it("counts feed items whose conversation id is the thread root", () => {
    const items = [
      feedItem({ id: "reply-1", rootId: "root-a", parentId: "root-a" }),
      feedItem({ id: "reply-2", rootId: "root-a", parentId: "reply-1" }),
      feedItem({ id: "reply-other", rootId: "root-b", parentId: "root-b" }),
      feedItem({ id: "root-a" }),
    ];
    assert.equal(countUnreadForStarredThreadRoot(items, "root-a"), 3);
    assert.equal(countUnreadForStarredThreadRoot(items, "root-b"), 1);
    assert.equal(countUnreadForStarredThreadRoot(items, "missing"), 0);
  });

  it("ignores blank root ids", () => {
    assert.equal(
      countUnreadForStarredThreadRoot([feedItem({ id: "root-a" })], "   "),
      0,
    );
  });
});

describe("shouldSuppressChannelActiveForStarredThread", () => {
  const starred = new Set(["root-starred"]);
  const isThreadStarred = (rootId) => starred.has(rootId);

  it("suppresses when the active channel's open thread is starred", () => {
    assert.equal(
      shouldSuppressChannelActiveForStarredThread({
        isChannelActive: true,
        selectedThreadRootId: "root-starred",
        isThreadStarred,
      }),
      true,
    );
  });

  it("does not suppress for unstarred threads or inactive channels", () => {
    assert.equal(
      shouldSuppressChannelActiveForStarredThread({
        isChannelActive: true,
        selectedThreadRootId: "root-other",
        isThreadStarred,
      }),
      false,
    );
    assert.equal(
      shouldSuppressChannelActiveForStarredThread({
        isChannelActive: false,
        selectedThreadRootId: "root-starred",
        isThreadStarred,
      }),
      false,
    );
    assert.equal(
      shouldSuppressChannelActiveForStarredThread({
        isChannelActive: true,
        selectedThreadRootId: null,
        isThreadStarred,
      }),
      false,
    );
  });
});

describe("formatSidebarUnreadCount", () => {
  it("caps at 99+", () => {
    assert.equal(formatSidebarUnreadCount(1), "1");
    assert.equal(formatSidebarUnreadCount(99), "99");
    assert.equal(formatSidebarUnreadCount(100), "99+");
  });
});
