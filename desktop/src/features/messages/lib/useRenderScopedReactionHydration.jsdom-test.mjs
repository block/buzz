/**
 * Reopen recovery for deadline-held reactions, through the production hook.
 *
 * A reaction fetch that hits the relay deadline keeps its ids claimed so
 * automatic re-renders do not re-send the slow read. Reopening the channel is
 * the explicit retry: the hook's `channelId` effect releases the held ids.
 *
 * Falsifiability: deleting the release effect in
 * `useRenderScopedReactionHydration.ts` leaves the reopen with no fetch.
 */

import assert from "node:assert/strict";
import { afterEach, mock, test } from "node:test";

import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { relayClient } from "@/shared/api/relayClient";
import { channelMessagesKey } from "./messageQueryKeys.ts";
import { resetRenderScopedReactionHydration } from "./renderScopedReactions.ts";
import { useRenderScopedReactionHydration } from "./useRenderScopedReactionHydration.ts";

const CHANNEL_A = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
const CHANNEL_B = "8a1c3f52-6d0e-4b7a-9c21-0f5e7d3b2a10";
const MESSAGE_ID = "1".repeat(64);
const REACTION = {
  id: "9".repeat(64),
  pubkey: "a".repeat(64),
  kind: 7,
  created_at: 1_700_000_000,
  content: "✅",
  tags: [["e", MESSAGE_ID]],
  sig: "sig",
};

afterEach(() => {
  mock.restoreAll();
  resetRenderScopedReactionHydration();
});

function Harness({ channelId, entries }) {
  useRenderScopedReactionHydration({
    activeChannel: { id: channelId, channelType: "stream" },
    mainTimelineEntries: entries,
    threadHeadMessage: null,
    threadMessages: [],
  });
  return null;
}

async function flush() {
  // Hydration is deferred with window.setTimeout(0).
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 10));
  });
}

test("reopening a channel refetches deadline-held reactions; same-channel re-renders do not", async () => {
  const requests = [];
  let failWithDeadline = true;
  mock.method(relayClient, "fetchAuxEventsByReference", async (channelId) => {
    requests.push(channelId);
    if (channelId === CHANNEL_A && failWithDeadline) {
      throw new Error("error: query timed out");
    }
    return channelId === CHANNEL_A ? [REACTION] : [];
  });
  mock.method(console, "error", () => {});

  const queryClient = new QueryClient();
  queryClient.setQueryData(channelMessagesKey(CHANNEL_A), []);
  const container = document.createElement("div");
  const root = createRoot(container);
  const render = (channelId) =>
    act(() => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(Harness, {
            channelId,
            // Fresh array each render, as a live timeline update produces.
            entries: [{ message: { id: MESSAGE_ID }, summary: null }],
          }),
        ),
      );
    });

  await render(CHANNEL_A);
  await flush();
  assert.deepEqual(
    requests,
    [CHANNEL_A],
    "first open fetches and hits the deadline",
  );

  failWithDeadline = false;
  await render(CHANNEL_A);
  await flush();
  assert.deepEqual(
    requests,
    [CHANNEL_A],
    "same-channel re-render keeps ids held",
  );

  await render(CHANNEL_B);
  await flush();
  await render(CHANNEL_A);
  await flush();
  assert.deepEqual(
    requests,
    [CHANNEL_A, CHANNEL_B, CHANNEL_A],
    "reopening channel A refetches its held ids",
  );
  assert.ok(
    queryClient
      .getQueryData(channelMessagesKey(CHANNEL_A))
      .some((event) => event.id === REACTION.id),
    "recovered reaction lands in the channel cache",
  );

  act(() => root.unmount());
});
