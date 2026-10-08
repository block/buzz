import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  RouterContextProvider,
  createMemoryHistory,
  createRootRoute,
  createRouter,
} from "@tanstack/react-router";
import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { ThoughtActivity } from "./ThoughtActivity.tsx";

function thoughtTree(router, queryClient, text) {
  // Markdown reads router and query context, so mount minimal providers.
  return React.createElement(
    RouterContextProvider,
    { router },
    React.createElement(
      QueryClientProvider,
      { client: queryClient },
      React.createElement(ThoughtActivity, {
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
      }),
    ),
  );
}

test("ThoughtActivity: a mounted empty thought becomes expandable when text streams in", async () => {
  const router = createRouter({
    routeTree: createRootRoute(),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  const queryClient = new QueryClient({
    defaultOptions: { queries: { enabled: false } },
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(thoughtTree(router, queryClient, "")));
    assert.ok(container.textContent.includes("No readable reasoning tokens"));
    assert.equal(container.querySelector("details"), null);

    await act(async () =>
      root.render(
        thoughtTree(router, queryClient, "Weighing the two options."),
      ),
    );
    assert.ok(container.querySelector("details"), "filled thought must expand");
    assert.ok(container.textContent.includes("Weighing the two options."));
    assert.ok(!container.textContent.includes("No readable reasoning tokens"));
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});
