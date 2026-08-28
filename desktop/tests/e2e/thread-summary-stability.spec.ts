import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Regression: entering a channel whose thread-summary facepiles hydrate late
 * (profiles resolving after first paint) must not reflow the timeline.
 *
 * The summary button used to be `inline-flex`, so it participated in its
 * wrapper's line box via its baseline. When the participant avatar's content
 * popped in (Radix AvatarFallback delay / avatar image landing), the button's
 * baseline moved and the line box collapsed by ~3px per summary row — every
 * row above visibly shifted a beat after the channel painted.
 *
 * The spec pins both the wrapper's height and the on-screen position of the
 * message row above the summary from first paint through profile hydration,
 * with the users-batch response deliberately delayed past first paint.
 */

const ALICE_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";
const BOB_PUBKEY =
  "bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260";

test("thread-summary rows keep their geometry while profiles hydrate", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await installMockBridge(page, { usersBatchDelayMs: 700 });
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  // Seed a root with a reply BEFORE entering, so the summary row is part of
  // the channel's first paint.
  const rootId = await page.evaluate(
    ({ alice, bob }) => {
      const root = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: "Stability root message",
        createdAt: 1_700_900_000,
        pubkey: alice,
      });
      if (!root) throw new Error("seed failed");
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: "Stability reply",
        parentEventId: root.id,
        createdAt: 1_700_900_100,
        pubkey: bob,
      });
      return root.id;
    },
    { alice: ALICE_PUBKEY, bob: BOB_PUBKEY },
  );

  // Sample geometry every frame from before entry until well past profile
  // hydration: the summary wrapper's height and the root row's viewport top.
  await page.evaluate((root) => {
    const samples: Array<{
      t: number;
      wrapperH: number;
      rootTop: number;
      avatars: number;
    }> = [];
    (window as unknown as { __STABILITY__: typeof samples }).__STABILITY__ =
      samples;
    const tick = () => {
      const summary = document.querySelector<HTMLElement>(
        `[data-testid="message-thread-summary"][data-thread-head-id="${root}"]`,
      );
      const rootRow = document.querySelector<HTMLElement>(
        `[data-message-id="${root}"]`,
      );
      if (summary?.parentElement && rootRow) {
        samples.push({
          t: Math.round(performance.now()),
          wrapperH:
            Math.round(
              summary.parentElement.getBoundingClientRect().height * 10,
            ) / 10,
          rootTop: Math.round(rootRow.getBoundingClientRect().top * 10) / 10,
          avatars: summary.querySelectorAll(
            '[data-testid="message-thread-summary-participant"] [data-testid], [data-testid="message-thread-summary-participant"] img, [data-testid="message-thread-summary-participant"] span',
          ).length,
        });
      }
      if (samples.length < 600) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, rootId);

  await page.getByTestId("channel-general").click();
  await expect(
    page.locator(
      `[data-testid="message-thread-summary"][data-thread-head-id="${rootId}"]`,
    ),
  ).toBeVisible();
  // The facepile avatar renders (initials or image) — hydration complete.
  await expect(
    page
      .locator(`[data-thread-head-id="${rootId}"]`)
      .getByTestId("message-thread-summary-participant"),
  ).toBeVisible();
  await page.waitForTimeout(1_500);

  const samples = await page.evaluate(
    () =>
      (
        window as unknown as {
          __STABILITY__: Array<{
            t: number;
            wrapperH: number;
            rootTop: number;
          }>;
        }
      ).__STABILITY__,
  );
  expect(samples.length).toBeGreaterThan(20);

  // The summary wrapper must hold one height from first paint onward: the
  // avatar's content popping in (fallback text or image) must not change the
  // row's box. This is the video-visible jitter mechanism.
  const wrapperHeights = [...new Set(samples.map((s) => s.wrapperH))];
  expect(
    wrapperHeights,
    `summary wrapper height changed after first paint: ${JSON.stringify(samples.filter((s, i, arr) => i === 0 || s.wrapperH !== arr[i - 1].wrapperH))}`,
  ).toHaveLength(1);

  // Position stability is asserted from hydration onward: profile arrival
  // itself may legitimately reflow rows whose author labels were cold (the
  // persisted label cache covers that in production), but nothing may move
  // after the facepile has rendered.
  const hydratedSamples = samples.filter((s) => s.avatars > 0).slice(1);
  expect(hydratedSamples.length).toBeGreaterThan(10);
  const rootTops = [...new Set(hydratedSamples.map((s) => s.rootTop))];
  expect(
    rootTops,
    `root row moved after facepile hydration: ${JSON.stringify(hydratedSamples.filter((s, i, arr) => i === 0 || s.rootTop !== arr[i - 1].rootTop))}`,
  ).toHaveLength(1);
});
