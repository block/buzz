import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { ProjectChannelRequestDetails } from "./ProjectChannelRequestDialog.tsx";

function request(overrides = {}) {
  return {
    homeChannelId: "11111111-1111-4111-8111-111111111111",
    name: "release-planning",
    visibility: "private",
    ...overrides,
  };
}

test("owner review shows an agent-requested temporary channel lifetime and cleanup consequence", () => {
  const html = renderToStaticMarkup(
    React.createElement(ProjectChannelRequestDetails, {
      request: request({ ttlSeconds: 90_000 }),
    }),
  );

  assert.match(html, />Lifetime</);
  assert.match(html, /Temporary · 1d1h/);
  assert.match(
    html,
    /Cleans up automatically after that period of inactivity\./,
  );
});

test("owner review omits temporary-channel lifetime when none was requested", () => {
  const html = renderToStaticMarkup(
    React.createElement(ProjectChannelRequestDetails, { request: request() }),
  );

  assert.doesNotMatch(html, />Lifetime</);
  assert.doesNotMatch(html, /Temporary/);
  assert.doesNotMatch(html, /Cleans up automatically/);
});
