import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import {
  canPreviewUnreadDm,
  MoreUnreadButton,
  preferredUnreadTarget,
  unreadDmAccessibleLabel,
  visibleUnreadDmPreviews,
} from "./MoreUnreadButton.tsx";

function preview(channelId, label = channelId, avatarUrl = null) {
  return {
    accessibleLabel: label,
    avatarUrl,
    channelId,
    label,
  };
}

describe("MoreUnreadButton model", () => {
  it("previews only honest one-to-one DM identities", () => {
    assert.equal(canPreviewUnreadDm(2, 1), true);
    assert.equal(canPreviewUnreadDm(3, 2), false);
    assert.equal(canPreviewUnreadDm(2, 0), false);
  });

  it("caps the stack at three avatars without a numeric overflow chip", () => {
    assert.deepEqual(
      visibleUnreadDmPreviews([preview("a"), preview("b"), preview("c")]).map(
        ({ channelId }) => channelId,
      ),
      ["a", "b", "c"],
    );
    assert.deepEqual(
      visibleUnreadDmPreviews([
        preview("a"),
        preview("b"),
        preview("c"),
        preview("d"),
      ]).map(({ channelId }) => channelId),
      ["a", "b", "c"],
    );
  });

  it("selects the first ordered DM independently from avatar eligibility", () => {
    const dmChannelIds = new Set(["near-group", "far-previewable"]);

    assert.equal(
      preferredUnreadTarget(
        ["near-group", "far-previewable", "channel"],
        dmChannelIds,
      ),
      "near-group",
    );
    assert.equal(
      preferredUnreadTarget(
        ["near-channel", "near-group", "far-previewable"],
        dmChannelIds,
      ),
      "near-group",
    );
    assert.equal(
      preferredUnreadTarget(["near-channel", "far-channel"], dmChannelIds),
      "near-channel",
    );
    assert.equal(preferredUnreadTarget([], dmChannelIds), undefined);
  });

  it("announces a DM identity only when it matches the navigation target", () => {
    assert.equal(
      unreadDmAccessibleLabel({
        count: 2,
        dmPreviews: [preview("dm", "Alice")],
        position: "bottom",
        targetChannelId: "dm",
      }),
      "Go to unread direct message from Alice. 2 new messages below.",
    );
    assert.equal(
      unreadDmAccessibleLabel({
        count: 2,
        dmPreviews: [preview("dm", "Alice")],
        label: "2 new activity",
        position: "bottom",
        targetChannelId: "dm",
      }),
      "Go to unread direct message from Alice. 2 new activity below.",
    );
    assert.equal(
      unreadDmAccessibleLabel({
        count: 2,
        dmPreviews: [preview("far-previewable", "Alice")],
        position: "bottom",
        targetChannelId: "near-group",
      }),
      "2 new messages below",
    );
    assert.equal(
      unreadDmAccessibleLabel({
        count: 1,
        dmPreviews: [],
        position: "top",
      }),
      "1 new message above",
    );
  });

  it("renders a decorative right-to-left stack with immediate fallback", () => {
    const markup = renderToStaticMarkup(
      MoreUnreadButton({
        count: 5,
        dmPreviews: [
          preview("dm-one", "Alice", "https://example.com/alice.png"),
          preview("dm-two", "Alice"),
          preview("dm-three", "Group DM"),
          preview("dm-four", "Dana"),
        ],
        onClick() {},
        position: "bottom",
        targetChannelId: "dm-one",
        testId: "more-unread",
      }),
    );

    assert.match(markup, /class="[^"]*overflow-hidden[^"]*"/);
    assert.match(
      markup,
      /<span class="min-w-0 truncate">5 new messages<\/span>/,
    );
    assert.match(
      markup,
      /aria-label="Go to unread direct message from Alice\. 5 new messages below\."/,
    );
    assert.doesNotMatch(markup, />Next<\/span>/);
    assert.match(markup, />·<\/span>/);
    assert.match(markup, /<span aria-hidden="true"/);
    assert.match(markup, /data-testid="sidebar-unread-dm-avatar-dm-one"/);
    assert.match(markup, /data-testid="sidebar-unread-dm-avatar-dm-two"/);
    assert.match(markup, /data-testid="sidebar-unread-dm-avatar-dm-three"/);
    assert.doesNotMatch(markup, /sidebar-unread-dm-avatar-dm-four/);
    assert.doesNotMatch(markup, />\+\d+<\/span>/);
    const stackOrder = [...markup.matchAll(/style="z-index:(\d+)"/g)].map(
      ([, zIndex]) => Number(zIndex),
    );
    assert.deepEqual(stackOrder, [3, 2, 1]);
  });

  it("keeps channel-based previews distinct for repeated participants", () => {
    const previews = [preview("dm-one", "Alice"), preview("dm-two", "Alice")];
    assert.deepEqual(
      visibleUnreadDmPreviews(previews).map(({ channelId }) => channelId),
      ["dm-one", "dm-two"],
    );
  });
});
