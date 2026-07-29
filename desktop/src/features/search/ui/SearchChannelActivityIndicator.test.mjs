import assert from "node:assert/strict";
import { describe, it } from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { SearchChannelActivityIndicator } from "./SearchChannelActivityIndicator.tsx";

const activeWorking = {
  agentCount: 1,
  agentNames: ["Honey"],
  agentPubkeys: ["honey-pubkey"],
  anchorAt: Date.now(),
  channelId: "recent-channel",
};

describe("SearchChannelActivityIndicator", () => {
  it("replaces an active recent timestamp with the shared working spinner", () => {
    const html = renderToStaticMarkup(
      React.createElement(SearchChannelActivityIndicator, {
        channelName: "recent-channel",
        summary: activeWorking,
        timestampLabel: "2m ago",
      }),
    );

    assert.match(html, /lucide-loader-circle/);
    assert.match(html, /motion-safe:animate-spin/);
    assert.match(html, /\binline-flex\b/);
    assert.doesNotMatch(html, /(?:class="| )hidden(?: |")/);
    assert.match(html, /aria-label="Honey working"/);
    assert.doesNotMatch(html, /2m ago/);
  });

  it("keeps the activity timestamp when the recent channel is idle", () => {
    const html = renderToStaticMarkup(
      React.createElement(SearchChannelActivityIndicator, {
        channelName: "recent-channel",
        timestampLabel: "2m ago",
      }),
    );

    assert.match(html, />2m ago</);
    assert.doesNotMatch(html, /lucide-loader-circle/);
  });
});
