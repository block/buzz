import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import {
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
  it("shows three avatars until overflow requires a +N chip", () => {
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
      ["a", "b"],
    );
  });

  it("uses the preview channel as the advertised navigation target", () => {
    assert.equal(preferredUnreadTarget([preview("dm")], "nearer"), "dm");
    assert.equal(preferredUnreadTarget([], "nearer"), "nearer");
  });

  it("announces the DM target and honest channel count", () => {
    assert.equal(
      unreadDmAccessibleLabel({
        count: 2,
        dmPreviews: [preview("dm", "Alice")],
        position: "bottom",
      }),
      "Go to unread direct message from Alice. 2 unread channels below.",
    );
    assert.equal(
      unreadDmAccessibleLabel({
        count: 1,
        dmPreviews: [],
        position: "top",
      }),
      "1 unread channel above",
    );
  });

  it("renders a decorative channel-keyed stack, overflow, and immediate fallback", () => {
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
        testId: "more-unread",
      }),
    );

    assert.match(
      markup,
      /aria-label="Go to unread direct message from Alice\. 5 unread channels below\."/,
    );
    assert.match(markup, /<span aria-hidden="true"/);
    assert.match(markup, /data-testid="sidebar-unread-dm-avatar-dm-one"/);
    assert.match(markup, /data-testid="sidebar-unread-dm-avatar-dm-two"/);
    assert.doesNotMatch(markup, /sidebar-unread-dm-avatar-dm-three/);
    assert.match(markup, />\+2<\/span>/);
  });

  it("keeps channel-based previews distinct for repeated participants", () => {
    const previews = [preview("dm-one", "Alice"), preview("dm-two", "Alice")];
    assert.deepEqual(
      visibleUnreadDmPreviews(previews).map(({ channelId }) => channelId),
      ["dm-one", "dm-two"],
    );
  });
});
