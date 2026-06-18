/**
 * Scroll prepend stability invariant — real-relay smoke for the channel
 * scroll-jump regression.
 *
 * Status: **nightly / manual-gated**, NOT part of the integration project's
 * default `testMatch`. The default gate for the math/timing class of this
 * bug is the mock-bridge case in `messaging.spec.ts` (deterministic, no
 * seed dependency). This spec exists as a real-relay artifact for hand
 * verification of a fix candidate against the actual relay path. Run with:
 *
 *     pnpm -C desktop exec playwright test scroll-prepend-stability.spec.ts
 *
 * **Precondition — local relay must reconcile DB-seeded channels.** The
 * relay only emits the `kind:39000` discovery events for channels inserted
 * via `scripts/setup-desktop-test-data.sh` if it was started with
 * `BUZZ_RECONCILE_CHANNELS=1` (see `crates/buzz-relay/src/main.rs:307`).
 * If your local relay was started without that env var, the DB will have
 * the seed channels but the relay won't emit them — `assertRelaySeeded()`
 * below will hang until the timeout. Restart the relay with the env var
 * before running this spec. (CI runners always restart fresh and don't
 * hit this trap.)
 *
 * Invariant: when older history is prepended, the visible anchor row stays
 * pinned to the same viewport offset. If this test ever fails, we have
 * re-introduced a `scrollTop`-during-input write somewhere in the
 * load-older path.
 *
 * IMPORTANT — this is a half-test. It runs on headless Chromium, NOT on
 * WKWebView (the actual Tauri target on macOS). That means it catches the
 * math/timing class of bugs (stale `previousScrollTop` captured before
 * commit, wrong `delta` math, ResizeObserver stomp on the restore, etc.)
 * but it does NOT reproduce the macOS-specific stale-`scrollTop`-read
 * hazard documented in Element/Matrix `docs/scrolling.md`. A green run
 * here is necessary, not sufficient — manual macOS wheel-scroll repro is
 * still required before any fix in this area is called shipped. See
 * `RESEARCH/BUZZ_SCROLL_PREPEND_2026-06-14.md` for the full background.
 */

import { expect, test, type Page } from "@playwright/test";

import { installRelayBridge } from "../helpers/bridge";

// Initial channel history limit is 200; older batch size is 100. To force at
// least one older-fetch round-trip, we need strictly more than 200 messages
// in the channel. 240 gives us enough headroom that the test still triggers
// load-older even if a couple of messages get dropped or merged.
const SEED_MESSAGE_COUNT = 240;
// Tolerance for the anchor's vertical position before vs. after prepend.
// Sub-pixel rounding and zoom can shift things a hair; anything past this
// is a real jump.
const ANCHOR_DRIFT_TOLERANCE_PX = 4;

async function createChannel(page: Page, channelName: string): Promise<string> {
  await page.getByRole("button", { name: "Create a channel" }).click();
  await page.getByTestId("create-channel-name").fill(channelName);
  await page.getByTestId("create-channel-submit").click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  return await page.evaluate(async (name) => {
    const tauriWindow = window as Window & {
      __TAURI_INTERNALS__?: {
        invoke: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
    };
    const invoke = tauriWindow.__TAURI_INTERNALS__?.invoke;
    if (!invoke) throw new Error("Tauri invoke unavailable");
    const channels = (await invoke("get_channels")) as Array<{
      id: string;
      name: string;
    }>;
    const channel = channels.find((c) => c.name === name);
    if (!channel) throw new Error(`Channel not found: ${name}`);
    return channel.id;
  }, channelName);
}

async function seedChannelMessages(
  page: Page,
  channelId: string,
  count: number,
  prefix: string,
) {
  // Sends `count` messages in small batches via the Tauri bridge. This is
  // slow (each call hits the relay and waits for ack) but it's the only
  // path that produces real signed events the load-older fetch can return.
  // Batches keep the JS event loop from starving the renderer.
  const BATCH_SIZE = 20;
  for (let start = 0; start < count; start += BATCH_SIZE) {
    const end = Math.min(start + BATCH_SIZE, count);
    await page.evaluate(
      async ({ channelId, prefix, start, end }) => {
        const tauriWindow = window as Window & {
          __TAURI_INTERNALS__?: {
            invoke: (
              command: string,
              payload?: Record<string, unknown>,
            ) => Promise<unknown>;
          };
        };
        const invoke = tauriWindow.__TAURI_INTERNALS__?.invoke;
        if (!invoke) throw new Error("Tauri invoke unavailable");
        for (let i = start; i < end; i += 1) {
          await invoke("send_channel_message", {
            channelId,
            content: `${prefix} seed ${i.toString().padStart(4, "0")}`,
            kind: null,
            mediaTags: null,
            mentionPubkeys: null,
            parentEventId: null,
          });
        }
      },
      { channelId, prefix, start, end },
    );
  }
}

/**
 * Reads, for each currently rendered message row, its `data-message-id`
 * plus the vertical offset of its top edge relative to the timeline
 * container's top edge.
 */
async function snapshotRenderedAnchors(page: Page) {
  return page.evaluate(() => {
    const timeline = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLElement | null;
    if (!timeline) return null;
    const containerRect = timeline.getBoundingClientRect();
    const rows = Array.from(
      timeline.querySelectorAll<HTMLElement>("[data-message-id]"),
    );
    return {
      messageCount: rows.length,
      firstMessageId: rows[0]?.dataset.messageId ?? null,
      metrics: {
        clientHeight: timeline.clientHeight,
        scrollHeight: timeline.scrollHeight,
        scrollTop: timeline.scrollTop,
      },
      anchors: rows.map((row) => {
        const rect = row.getBoundingClientRect();
        return {
          bottom: rect.bottom - containerRect.top,
          id: row.dataset.messageId ?? "",
          top: rect.top - containerRect.top,
          visible:
            rect.bottom > containerRect.top && rect.top < containerRect.bottom,
        };
      }),
    };
  });
}

async function getTimelineMetrics(page: Page) {
  return page.getByTestId("message-timeline").evaluate((el) => {
    const t = el as HTMLDivElement;
    return {
      clientHeight: t.clientHeight,
      scrollHeight: t.scrollHeight,
      scrollTop: t.scrollTop,
    };
  });
}

/**
 * Scrolls the timeline up using wheel events (not `scrollTop=` writes —
 * we don't want the harness to bypass the same code path the user uses).
 * Stops once `messageCount` rendered rows are above the viewport — i.e.
 * we've scrolled far enough that we're near the top sentinel and the
 * older-fetch rootMargin (200px) will fire.
 *
 * Returns the anchor we want to track: a message id that is currently
 * visible roughly one-third down the viewport. We pick mid-viewport
 * (not the top) so that even if the prepend goes wrong by a little, the
 * anchor doesn't accidentally scroll off-screen and become unfindable.
 */
async function scrollUpAndPickAnchor(page: Page) {
  const timeline = page.getByTestId("message-timeline");
  await timeline.hover();
  // Wheel up in large but realistic chunks (~1 screenful per tick).
  for (let attempts = 0; attempts < 60; attempts += 1) {
    const metrics = await getTimelineMetrics(page);
    if (metrics.scrollTop < 400) break;
    await page.mouse.wheel(0, -600);
    await page.waitForTimeout(40);
  }

  const anchor = await page.evaluate(() => {
    const timeline = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLElement | null;
    if (!timeline) return null;
    const rect = timeline.getBoundingClientRect();
    const preferredTop = rect.height / 3;
    let best: { distance: number; id: string; top: number } | null = null;

    for (const row of Array.from(
      timeline.querySelectorAll<HTMLElement>("[data-message-id]"),
    )) {
      const rowRect = row.getBoundingClientRect();
      if (rowRect.bottom <= rect.top || rowRect.top >= rect.bottom) {
        continue;
      }
      const top = rowRect.top - rect.top;
      const distance = Math.abs(top - preferredTop);
      if (!best || distance < best.distance) {
        best = { distance, id: row.dataset.messageId ?? "", top };
      }
    }

    return best ? { id: best.id, top: best.top } : null;
  });
  if (!anchor) {
    const snap = await snapshotRenderedAnchors(page);
    throw new Error(
      `Could not find an anchor message row after scrolling up. Snapshot: ${JSON.stringify(
        snap,
      )}`,
    );
  }
  return anchor;
}

async function waitForPrependCommit(
  page: Page,
  previousFirstMessageId: string,
  previousCount: number,
  timeoutMs = 5_000,
) {
  // Polls until the rendered message snapshot shows BOTH a new first-message
  // id AND a larger row count. Either alone is not enough: a count bump with
  // the same first id can happen on bottom appends; a first-id change with no
  // count bump can happen if a message gets re-keyed.
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const snap = await snapshotRenderedAnchors(page);
    if (
      snap &&
      snap.firstMessageId !== previousFirstMessageId &&
      snap.messageCount > previousCount
    ) {
      return snap;
    }
    await page.waitForTimeout(50);
  }
  throw new Error("Prepend did not commit within the expected window.");
}

test("anchor row stays pinned across older-history prepend", async ({
  browser,
}) => {
  // This is slow on purpose — seeding 240 messages over the real relay takes
  // ~10–15 seconds locally and longer on CI.
  test.slow();

  const channelName = `prepend-stability-${Date.now()}`;
  const ownerContext = await browser.newContext();
  const readerContext = await browser.newContext();
  const ownerPage = await ownerContext.newPage();
  const readerPage = await readerContext.newPage();

  try {
    // Alice creates the channel and seeds it; Tyler reads it. Splitting the
    // roles keeps the seed path off the same JS context that will run the
    // assertion, which keeps the seed-loop's render churn from polluting the
    // measurement.
    await installRelayBridge(ownerPage, "alice");
    await installRelayBridge(readerPage, "tyler");
    await ownerPage.goto("/");
    await readerPage.goto("/");

    const channelId = await createChannel(ownerPage, channelName);
    await seedChannelMessages(ownerPage, channelId, SEED_MESSAGE_COUNT, "msg");

    // Reader joins the channel.
    await readerPage.getByTestId("browse-channels").click();
    await expect(
      readerPage.getByTestId("channel-browser-dialog"),
    ).toBeVisible();
    await readerPage
      .getByTestId(`browse-channel-${channelName}`)
      .getByRole("button", { name: "Join" })
      .click();
    await expect(readerPage.getByTestId("chat-title")).toHaveText(channelName);

    // Wait for the initial history load (200 messages) to land. We need at
    // least 100 rendered rows before we even try scrolling up; in practice
    // the initial load brings 200.
    await expect
      .poll(
        async () => {
          const snap = await snapshotRenderedAnchors(readerPage);
          return snap?.messageCount ?? 0;
        },
        { timeout: 15_000 },
      )
      .toBeGreaterThan(150);

    // Give the timeline a beat to settle (initial scroll-to-bottom, layout).
    await readerPage.waitForTimeout(300);

    // Scroll up and pick an anchor message that's currently visible.
    const anchorBefore = await scrollUpAndPickAnchor(readerPage);

    // Snapshot the rendered state right before the fetch fires so we can
    // detect commit later.
    const before = await snapshotRenderedAnchors(readerPage);
    if (!before) throw new Error("Timeline disappeared before prepend.");

    // Trigger one more wheel tick to push the top sentinel into the
    // load-older rootMargin (200px above the viewport).
    await readerPage.getByTestId("message-timeline").hover();
    await readerPage.mouse.wheel(0, -800);

    // Wait for the prepend to commit.
    const after = await waitForPrependCommit(
      readerPage,
      before.firstMessageId ?? "",
      before.messageCount,
    );

    // Give the post-commit layout-effect one frame to run its restore.
    await readerPage.waitForTimeout(80);

    // Find the same anchor id in the new render and measure where it is now.
    const anchorAfter = after.anchors.find((a) => a.id === anchorBefore.id);
    expect(anchorAfter, "anchor row missing after prepend").toBeDefined();

    // Re-measure live (not from the cached `after` snapshot) — the
    // layout-effect restore may run AFTER `waitForPrependCommit` returned.
    const livePosition = await readerPage.evaluate((id) => {
      const timeline = document.querySelector(
        '[data-testid="message-timeline"]',
      ) as HTMLElement | null;
      if (!timeline) return null;
      const row = timeline.querySelector<HTMLElement>(
        `[data-message-id="${CSS.escape(id)}"]`,
      );
      if (!row) return null;
      return (
        row.getBoundingClientRect().top - timeline.getBoundingClientRect().top
      );
    }, anchorBefore.id);

    expect(
      livePosition,
      "anchor row missing after layout settle",
    ).not.toBeNull();
    const drift = Math.abs((livePosition ?? 0) - anchorBefore.top);
    expect(
      drift,
      `anchor row "${anchorBefore.id}" drifted ${drift.toFixed(2)}px ` +
        `(was ${anchorBefore.top.toFixed(2)}px, now ${(livePosition ?? 0).toFixed(2)}px). ` +
        `Tolerance is ${ANCHOR_DRIFT_TOLERANCE_PX}px. ` +
        `This means the older-history prepend moved the viewport — the ` +
        `scroll-jump bug has regressed.`,
    ).toBeLessThanOrEqual(ANCHOR_DRIFT_TOLERANCE_PX);
  } finally {
    await readerContext.close();
    await ownerContext.close();
  }
});
