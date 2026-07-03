import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// =============================================================================
// Regression — a live broadcast reply must enter the authoritative channel
// window store (PR #1500 review, blocker 2)
// =============================================================================
//
// NIP-CW §Top-level Classification: a depth-1 reply carrying `["broadcast","1"]`
// is a channel-window row — it belongs on the timeline as well as in its thread.
// The relay serves it as a row (`buzz-db/src/thread.rs`: "top-level rows =
// depth 0, missing metadata, or broadcast depth-1 replies").
//
// The client's live-append path drops it from the WINDOW STORE. `appendMessage`
// (hooks.ts) routes every parented timeline event into the thread cache and
// returns BEFORE the window-store merge, gating only on `parentId !== null`
// with no broadcast check. So a live broadcast reply never reaches the
// `channel-window` store's `liveOverlay` — it survives on the timeline only via
// the incidental `useLiveChannelUpdates` write to `channel-messages`, which any
// window-store rebuild (page-zero refresh, later top-level append) erases.
//
// We assert the durable invariant — the broadcast reply IS in the window
// store's `liveOverlay` — rather than a DOM row, because the timeline render is
// masked by the second (unfiltered) subscriber. The window store is the
// authoritative source `flattenChannelWindowEvents` rebuilds from.
//
// RED at f2a551f2 (appendMessage returns before the overlay merge → liveOverlay
// omits the broadcast reply). GREEN on wren/review-live-window-fixes @ 9a533a9e
// (broadcast replies fall through into `mergeLiveChannelWindowEvent`).
//
// An ordinary (non-broadcast) depth-1 reply is emitted as a control: it is NOT
// a window row and MUST stay out of the overlay, so a naive "merge every
// parented event" fix can't false-green this spec.

const CHANNEL = "general";

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(() =>
      page.evaluate(
        (ch) =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: ch,
          }) ?? false,
        channelName,
      ),
    )
    .toBe(true);
}

async function emit(
  page: import("@playwright/test").Page,
  input: {
    content: string;
    parentEventId?: string | null;
    createdAt?: number;
    extraTags?: string[][];
  },
) {
  const event = await page.evaluate(
    (payload) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: payload.channel,
        content: payload.content,
        parentEventId: payload.parentEventId,
        createdAt: payload.createdAt,
        extraTags: payload.extraTags,
      }),
    {
      channel: CHANNEL,
      content: input.content,
      parentEventId: input.parentEventId ?? null,
      createdAt: input.createdAt,
      extraTags: input.extraTags,
    },
  );
  if (!event) throw new Error("mock message emitter is not installed");
  return event;
}

async function liveOverlayContents(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    const qc = window.__BUZZ_E2E_QUERY_CLIENT__ as unknown as {
      getQueriesData: (f: unknown) => Array<[readonly unknown[], unknown]>;
    };
    const win = qc
      .getQueriesData({ queryKey: [] })
      .find(([key]) => JSON.stringify(key).includes("channel-window"));
    const store = win?.[1] as
      | { liveOverlay?: Array<{ content?: string }> }
      | undefined;
    return (store?.liveOverlay ?? []).map((event) => event.content ?? "");
  });
}

test("a live broadcast depth-1 reply enters the authoritative channel window store", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  // Seed a top-level root into the cold window before opening the channel, so
  // there is a thread for the broadcast reply to descend from.
  const now = Math.floor(Date.now() / 1000);
  const root = await emit(page, { content: "timeline root", createdAt: now });

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText(CHANNEL);
  await expect(
    page.getByTestId("message-timeline").getByText("timeline root"),
  ).toBeVisible();

  // The live subscription must be established before we emit, or the event is
  // delivered before appendMessage is listening — that would be a cold-load
  // test, not a live-append test.
  await waitForMockLiveSubscription(page, CHANNEL);

  // LIVE broadcast depth-1 reply: parent is the root, carries ["broadcast","1"].
  await emit(page, {
    content: "broadcast to the channel",
    parentEventId: root.id,
    createdAt: now + 1,
    extraTags: [["broadcast", "1"]],
  });

  // CONTROL — an ordinary (non-broadcast) depth-1 reply: NOT a window row, MUST
  // stay out of the overlay.
  await emit(page, {
    content: "ordinary thread reply",
    parentEventId: root.id,
    createdAt: now + 2,
  });

  // The broadcast reply must land in the authoritative window-store overlay via
  // live append alone — the invariant that survives any window-store rebuild.
  await expect
    .poll(() => liveOverlayContents(page))
    .toContain("broadcast to the channel");

  // The ordinary reply is a thread reply, never a window row.
  expect(await liveOverlayContents(page)).not.toContain(
    "ordinary thread reply",
  );
});
