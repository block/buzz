import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { ChannelIntroBlock } from "./ChannelIntroBlock.tsx";

test("ChannelIntroBlock preserves multi-paragraph description whitespace", () => {
  const description = "First paragraph.\n\nSecond paragraph.\nThird line.";
  const html = renderToStaticMarkup(
    React.createElement(ChannelIntroBlock, {
      intro: {
        channelKindLabel: "regular channel",
        channelName: "test",
        description,
      },
    }),
  );

  assert.match(html, /whitespace-pre-line/);
  assert.match(html, /First paragraph\.\n\nSecond paragraph\.\nThird line\./);
});
