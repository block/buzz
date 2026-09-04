import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { TooltipProvider } from "../tooltip.tsx";
import { ExternalLinkAnchor } from "./ExternalLinkAnchor.tsx";

test("external markdown links rest without an underline", () => {
  const html = renderToStaticMarkup(
    React.createElement(
      TooltipProvider,
      null,
      React.createElement(
        ExternalLinkAnchor,
        {
          anchorProps: {},
          href: "https://example.com/tasks/t_5873512f",
          isLinearLink: false,
          label: "t_5873512f",
        },
        "t_5873512f",
      ),
    ),
  );

  const className = html.match(/<a[^>]*class="([^"]+)"/)?.[1] ?? "";
  assert.match(className, /(?:^| )no-underline(?: |$)/);
  assert.match(className, /(?:^| )hover:underline(?: |$)/);
  assert.doesNotMatch(className, /(?:^| )underline(?: |$)/);
});
