import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  RouterContextProvider,
  createMemoryHistory,
  createRootRoute,
  createRouter,
} from "@tanstack/react-router";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { ThoughtActivity } from "./ThoughtActivity.tsx";

function renderThought(text) {
  // Markdown reads router and query context, so mount minimal providers.
  const router = createRouter({
    routeTree: createRootRoute(),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  const thought = React.createElement(ThoughtActivity, {
    agentAvatarUrl: null,
    agentName: "Test Agent",
    agentPubkey: "pubkey123",
    item: {
      agentPubkey: "pubkey123",
      sessionId: "session-001",
      turnId: null,
      id: "thought:1",
      type: "thought",
      renderClass: "thought",
      title: "Thinking",
      text,
      timestamp: "2026-10-02T14:27:48.000Z",
    },
  });
  return renderToStaticMarkup(
    React.createElement(
      RouterContextProvider,
      { router },
      React.createElement(
        QueryClientProvider,
        {
          client: new QueryClient({
            defaultOptions: { queries: { enabled: false } },
          }),
        },
        thought,
      ),
    ),
  );
}

for (const text of ["", "  \n"]) {
  test(`ThoughtActivity: textless thought (${JSON.stringify(text)}) shows a non-expandable notice`, () => {
    const html = renderThought(text);
    assert.ok(html.includes("Thinking"));
    assert.ok(html.includes("No readable reasoning tokens"));
    assert.ok(!html.includes("<details"), "empty thought must not expand");
  });
}
