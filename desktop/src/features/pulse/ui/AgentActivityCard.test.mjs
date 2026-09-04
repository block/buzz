import assert from "node:assert/strict";
import test from "node:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import {
  RouterContextProvider,
  createRouter,
  createRootRoute,
  createMemoryHistory,
} from "@tanstack/react-router";
import { renderToStaticMarkup } from "react-dom/server";
import { AgentActivityCard } from "./AgentActivityCard.tsx";

function render(status) {
  const pubkey = "a".repeat(64);
  const router = createRouter({
    routeTree: createRootRoute(),
    history: createMemoryHistory({ initialEntries: ["/"] }),
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
        React.createElement(AgentActivityCard, {
          agentStatus: status,
          profile: { displayName: "Policy-only Scout", avatarUrl: null },
          group: {
            pubkey,
            latestAt: 1700000000,
            earliestAt: 1700000000,
            notes: [
              {
                id: "note",
                pubkey,
                content: "Still discoverable",
                createdAt: 1700000000,
              },
            ],
          },
        }),
      ),
    ),
  );
}

test("Pulse does not render unknown policy-only discovery as an offline dot", () => {
  const html = render("unknown");
  assert.match(html, /Policy-only Scout/);
  assert.match(html, /Still discoverable/);
  assert.doesNotMatch(html, /aria-label="Agent (offline|online|away)"/);
  assert.doesNotMatch(html, /bg-zinc-400/);
});

for (const status of ["online", "away", "offline"]) {
  test(`Pulse retains explicit ${status} liveness evidence`, () => {
    assert.match(render(status), new RegExp(`aria-label="Agent ${status}"`));
  });
}
