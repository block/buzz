import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function getTimelineMetrics(page: import("@playwright/test").Page) {
  return page.getByTestId("message-timeline").evaluate((element) => {
    const timeline = element as HTMLDivElement;

    return {
      clientHeight: timeline.clientHeight,
      scrollHeight: timeline.scrollHeight,
      scrollTop: timeline.scrollTop,
    };
  });
}

async function getFirstVisibleMessage(page: import("@playwright/test").Page) {
  return page.getByTestId("message-timeline").evaluate((element) => {
    const timeline = element as HTMLDivElement;
    const timelineRect = timeline.getBoundingClientRect();
    const messages = Array.from(
      timeline.querySelectorAll<HTMLElement>("[data-message-id]"),
    );

    for (const message of messages) {
      const rect = message.getBoundingClientRect();
      if (rect.bottom <= timelineRect.top || rect.top >= timelineRect.bottom) {
        continue;
      }

      return {
        id: message.dataset.messageId ?? "",
        text: message.textContent?.replace(/\s+/g, " ").slice(0, 80) ?? "",
        top: rect.top - timelineRect.top,
      };
    }

    return null;
  });
}

async function getMessagePosition(
  page: import("@playwright/test").Page,
  messageId: string,
) {
  return page.getByTestId("message-timeline").evaluate((element, id) => {
    const timeline = element as HTMLDivElement;
    const message = timeline.querySelector<HTMLElement>(
      `[data-message-id="${CSS.escape(id)}"]`,
    );
    if (!message) {
      return null;
    }

    return {
      id,
      top:
        message.getBoundingClientRect().top -
        timeline.getBoundingClientRect().top,
    };
  }, messageId);
}

test("preserves user scroll while older channel history loads", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__ === "function",
  );

  await page.evaluate(() => {
    for (let index = 0; index < 40; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `visible current ${index}\nsecond line ${index}`,
      });
    }
    window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__?.({
      channelName: "general",
      count: 250,
      lineCount: 3,
    });
  });

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline).toContainText("visible current 39");

  // Initial load should receive enough history to make the page scrollable.
  // Delay only the next history request, so the test isolates pagination while
  // the user is actively scrolling.
  await page.evaluate(() => {
    window.__BUZZ_E2E__ = {
      ...window.__BUZZ_E2E__,
      mock: {
        ...window.__BUZZ_E2E__?.mock,
        historyDelayMs: 1_000,
      },
    };
  });

  await page.waitForFunction(() => {
    const element = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLDivElement | null;
    return element && element.scrollHeight > element.clientHeight + 1000;
  });

  // Move away from the bottom before jumping near the top; otherwise the
  // timeline's sticky-bottom guard can intentionally pin the first upward jump.
  const beforeFetch = await getTimelineMetrics(page);
  await timeline.evaluate((element) => {
    const timelineElement = element as HTMLDivElement;
    timelineElement.scrollTop = timelineElement.scrollHeight;
    timelineElement.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(50);

  const nearTop = await timeline.evaluate((element) => {
    const timelineElement = element as HTMLDivElement;
    timelineElement.scrollTop = 180;
    timelineElement.dispatchEvent(new Event("scroll", { bubbles: true }));
    return timelineElement.scrollTop;
  });
  expect(nearTop).toBeLessThan(260);

  await page.waitForTimeout(100);
  const duringFetch = await timeline.evaluate((element) => {
    const timelineElement = element as HTMLDivElement;
    timelineElement.scrollTop = timelineElement.scrollTop + 160;
    timelineElement.dispatchEvent(new Event("scroll", { bubbles: true }));
    return timelineElement.scrollTop;
  });
  expect(duringFetch).toBeGreaterThan(nearTop);
  const anchorDuringFetch = await getFirstVisibleMessage(page);
  expect(anchorDuringFetch).not.toBeNull();

  await expect
    .poll(
      async () => {
        const [anchor, metrics] = await Promise.all([
          getMessagePosition(page, anchorDuringFetch?.id ?? ""),
          getTimelineMetrics(page),
        ]);
        if (metrics.scrollHeight <= beforeFetch.scrollHeight + 1000) {
          return Number.POSITIVE_INFINITY;
        }
        return anchor
          ? Math.abs(anchor.top - (anchorDuringFetch?.top ?? 0))
          : Number.POSITIVE_INFINITY;
      },
      {
        timeout: 3_000,
      },
    )
    .toBeLessThanOrEqual(2);
});

// Criterion 2: abandon-to-bottom mid-fetch.
//
// This catches the live-anchor vs transaction-anchor race that made the old
// design untrustworthy. The user begins an older-history fetch, then changes
// their mind and jumps back to bottom before the fetch resolves. When the
// prepended messages finally land, the timeline must remain pinned to the
// bottom -- it must NOT teleport upward to "restore" the anchor row the user
// already abandoned.
//
// Baseline note (recorded against main): this test PASSES on main. Probe
// geometry shows the existing useLoadOlderOnScroll restore would write
// scrollTop = capturedScrollTop(180) + delta(~9736) = ~9916, but the actual
// post-prepend scrollTop is at the true bottom (~29600). The bottom-stick
// guard in useTimelineScrollManager wins the race against the older-restore
// callback when the user has returned to bottom -- by accident of ordering,
// not by design. The Virtuoso replacement must keep this user-observable
// contract while removing the race-prone two-writer architecture: any
// implementation where firstItemIndex/anchor-restore can override the
// at-bottom state will fail this test. That is exactly what we want to
// prevent regressing.
//
// Black-box assertions only:
//   (a) timeline geometry: scrollTop + clientHeight >= scrollHeight - 2
//       (still at bottom after prepend)
//   (b) the last [data-message-id] row's bottom is within 2px of the
//       timeline's bottom edge (last message stayed visible at the floor)
test("does not teleport upward when user abandons fetch by jumping to bottom", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__ === "function",
  );

  await page.evaluate(() => {
    for (let index = 0; index < 40; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `visible current ${index}\nsecond line ${index}`,
      });
    }
    window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__?.({
      channelName: "general",
      count: 250,
      lineCount: 3,
    });
  });

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const timeline = page.getByTestId("message-timeline");
  // Setup gate: confirm the channel rendered at least one message row.
  // The original draft asserted toContainText("visible current 39") --
  // i.e. the last seeded message text must be in the timeline -- which
  // accidentally encodes a non-virtualized contract. A Virtuoso-rendered
  // timeline mounts a window of rows; the LAST seeded row may not be in
  // that window if the implementation doesn't start at the bottom (which
  // is itself a separate concern, validated below by `baseline` being
  // pinned to scrollHeight).
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();

  // Pace the next history fetch so we have a deterministic window to abandon
  // it. The window must be longer than the time required for the wheel-up
  // step + the post-wheel waitForTimeout, otherwise the fetch resolves
  // mid-wheel and the "abandon mid-fetch" semantic disappears (it becomes
  // "scroll up, observe prepend land, scroll back to bottom" -- a different
  // test). 5s is comfortably longer than the wheel-up loop in practice.
  await page.evaluate(() => {
    window.__BUZZ_E2E__ = {
      ...window.__BUZZ_E2E__,
      mock: {
        ...window.__BUZZ_E2E__?.mock,
        historyDelayMs: 5_000,
      },
    };
  });

  // Wait until the timeline is scrollable before we start the dance.
  await page.waitForFunction(() => {
    const element = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLDivElement | null;
    return element ? element.scrollHeight > element.clientHeight + 1000 : false;
  });

  // Start at bottom (the timeline should already be sticky-bottom on first
  // load; assert it so the abandon step has a meaningful target to return to).
  const baseline = await getTimelineMetrics(page);
  expect(baseline.scrollTop + baseline.clientHeight).toBeGreaterThanOrEqual(
    baseline.scrollHeight - 2,
  );

  // Scroll up to trigger the older-history fetch. We drive this through
  // real wheel input rather than `scrollTop = 180; dispatchEvent("scroll")`
  // because that direct-mutation pattern only works against a naive
  // scroll container -- a virtualizer like Virtuoso can intercept or
  // re-assert its tracked scroll position, leaving the outer element's
  // scrollTop pinned to the bottom even after the write. mouse.wheel
  // dispatches a real WheelEvent that whichever element owns scrolling
  // must honor. Wheel up in 2000-delta chunks (the browser applies its
  // own delta scaling so the actual scrolled distance is typically a
  // fraction of this) until scrollTop crosses below 500 or we hit a
  // step cap.
  await timeline.hover();
  for (let attempt = 0; attempt < 32; attempt += 1) {
    const metrics = await getTimelineMetrics(page);
    if (metrics.scrollTop < 500) {
      break;
    }
    await page.mouse.wheel(0, -2000);
  }
  await page.waitForTimeout(150);

  const duringFetch = await getTimelineMetrics(page);
  // Sanity: we did move away from the bottom and the prepend has NOT yet
  // resolved (otherwise the abandon scenario doesn't exist).
  expect(duringFetch.scrollTop).toBeLessThan(500);
  expect(duringFetch.scrollHeight).toBeLessThanOrEqual(
    baseline.scrollHeight + 100,
  );

  // Abandon: jump back to bottom while the fetch is still in flight.
  // We click the in-app "Jump to latest" button rather than writing
  // scrollTop = scrollHeight directly. Same rationale as the upward
  // wheel above: a virtualizer may not honor a raw scrollTop write,
  // but the application's own scroll-to-bottom path is exactly what
  // a real user would invoke and is part of the user-observable
  // surface for both scroller implementations.
  //
  // The button triggers `scrollToBottom("smooth")` which animates the
  // scroll, so we poll for the at-bottom condition rather than waiting
  // a fixed interval. Cap at 2s; well inside our 5s historyDelayMs
  // window so the prepend is still in flight when this resolves.
  await page.getByTestId("message-scroll-to-latest").click();
  await expect
    .poll(
      async () => {
        const m = await getTimelineMetrics(page);
        return m.scrollTop + m.clientHeight >= m.scrollHeight - 2;
      },
      { timeout: 2_000 },
    )
    .toBe(true);

  const afterAbandon = await getTimelineMetrics(page);
  expect(
    afterAbandon.scrollTop + afterAbandon.clientHeight,
  ).toBeGreaterThanOrEqual(afterAbandon.scrollHeight - 2);

  // Now wait for the prepend to resolve. scrollHeight grows when older
  // messages land; we poll for that growth and then assert we are STILL
  // at the bottom (no upward teleport to the abandoned anchor).
  //
  // Timeout: 6s. The wheel-up + smooth-abandon path can burn 2-3s of
  // the 5s historyDelayMs window before this poll begins, so the
  // prepend may not land for another 2-3s. 6s leaves margin.
  await expect
    .poll(
      async () => {
        const metrics = await getTimelineMetrics(page);
        return metrics.scrollHeight > afterAbandon.scrollHeight + 1000
          ? "resolved"
          : "pending";
      },
      { timeout: 6_000 },
    )
    .toBe("resolved");

  const afterPrepend = await getTimelineMetrics(page);
  // (a) Geometry: timeline still pinned to bottom.
  expect(
    afterPrepend.scrollTop + afterPrepend.clientHeight,
  ).toBeGreaterThanOrEqual(afterPrepend.scrollHeight - 2);

  // (b) DOM: the last rendered [data-message-id] sits within 2px of the
  // timeline's bottom edge. This catches a class of bugs where the geometry
  // looks bottom-pinned but the row landed off-screen due to padding/
  // composer-spacer drift.
  const lastRowOffset = await timeline.evaluate((element) => {
    const t = element as HTMLDivElement;
    const messages = Array.from(
      t.querySelectorAll<HTMLElement>("[data-message-id]"),
    );
    if (messages.length === 0) {
      return null;
    }
    const last = messages[messages.length - 1];
    const timelineRect = t.getBoundingClientRect();
    const rowRect = last.getBoundingClientRect();
    return timelineRect.bottom - rowRect.bottom;
  });
  expect(lastRowOffset).not.toBeNull();
  // Allow up to one composer-height worth of slack here: the composer overlay
  // can legitimately float above the timeline's clientHeight floor. The
  // important regression we are catching is "last row teleported hundreds
  // of pixels up", not "last row sits 40px above the floor due to overlay".
  // If the new design uses a Footer spacer matching composer height, this
  // value should be small. We assert <= 200 to be tolerant of design choice
  // while still catching teleport-class regressions.
  expect(lastRowOffset as number).toBeLessThanOrEqual(200);
});

const REAL_BUZZ_BUGS_IMAGE_SHA =
  "ff2862080bac3d009f97cad4bb94e6efec328eaaee058a405e854acd49fc1483";
const REAL_BUZZ_BUGS_IMAGE_URL = `https://sprout-oss.stage.blox.sqprod.co/media/${REAL_BUZZ_BUGS_IMAGE_SHA}.png`;
const REAL_BUZZ_BUGS_IMAGE_TAG = [
  "imeta",
  `url ${REAL_BUZZ_BUGS_IMAGE_URL}`,
  "m image/png",
  `x ${REAL_BUZZ_BUGS_IMAGE_SHA}`,
  "size 26257",
  "dim 951x244",
  "filename image.png",
] as string[];

test("reserves real buzz-bugs imeta image height before image loads", async ({
  page,
}) => {
  await page.route("**/media/**", () => new Promise(() => {}));
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  await page.evaluate(
    ({ content, extraTags }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content,
        extraTags,
      });
    },
    {
      content: `this setting gets reverted on every update\n![image](${REAL_BUZZ_BUGS_IMAGE_URL})`,
      extraTags: [REAL_BUZZ_BUGS_IMAGE_TAG],
    },
  );

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const image = page.getByAltText("image").last();
  const rect = await image.evaluate((element) => {
    const img = element as HTMLImageElement;
    const box = img.getBoundingClientRect();
    return {
      attrHeight: img.getAttribute("height"),
      attrWidth: img.getAttribute("width"),
      height: box.height,
      offsetHeight: img.offsetHeight,
      offsetWidth: img.offsetWidth,
      width: box.width,
    };
  });
  expect(rect.attrWidth).toBe("951");
  expect(rect.attrHeight).toBe("244");
  expect(rect.offsetHeight).toBeGreaterThan(80);
});

// Criterion 3: target-after-backfill via deep-link.
//
// When the user opens a deep-link URL like /channels/<id>?messageId=<targetId>
// for a message that lives in older history (not in the initial render
// window), the timeline must scroll the target into view and apply the
// highlight treatment. This validates the target-resolution path independent
// of the user manually scrolling up to find the message.
//
// Black-box assertions only (per Mari's refinement):
//   (a) target row's [data-message-id] is in the DOM
//   (b) target row's bounding rect is inside the timeline's bounding rect
//       AND within ~half a viewport of the timeline's vertical center
//   (c) target row's className includes the highlight token
//       ("route-target-highlight-fade") -- this is the user-visible
//       highlight effect, applied via the `highlighted` prop in MessageRow
//
// No `onTargetReached` probe is used; visible-centered-highlighted is the
// stable user-observable contract.
test("deep-link to a message in older history scrolls and highlights it", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__ === "function",
  );

  // Seed the channel with a small live window plus a large prepended
  // history block. Capture the prepended event ids so we can pick a target
  // that sits well outside the initial-render window.
  const prependedIds: string[] = await page.evaluate(() => {
    for (let index = 0; index < 40; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `visible current ${index}\nsecond line ${index}`,
      });
    }
    const events = window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__?.({
      channelName: "general",
      count: 250,
      lineCount: 3,
    });
    return (events ?? []).map((event) => event.id);
  });
  expect(prependedIds.length).toBeGreaterThanOrEqual(100);

  // Pick a target from the OLDER half of the prepended block. The initial
  // history slice on channel open is limited to ~50 events; anything in
  // the older half is guaranteed to be outside the first render window.
  // prependedIds are emitted in chronological order (older first), so the
  // first quarter is reliably old.
  const targetId = prependedIds[Math.floor(prependedIds.length / 8)];
  expect(targetId).toBeTruthy();

  // Open the channel via the sidebar click (same pattern as criterion-1).
  // The dev server has no SPA fallback, so a direct page.goto into a deep
  // /channels/<id>?messageId=<id> URL 404s, and a synthetic popstate isn't
  // enough to make Tanstack Router's matcher pick up the new route. Going
  // through the live UI navigation puts us inside the channel route with
  // the router properly mounted; then we only need to update the search
  // param to test the target-resolution contract.
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const timeline = page.getByTestId("message-timeline");
  await expect(timeline).toContainText("visible current 39");

  // Now push ?messageId=<targetId> onto the existing /channels/<id> URL.
  // This is the contract under test: when the route's messageId search
  // param changes (which happens both on deep-link arrival and on in-app
  // navigation like clicking a reply quote), the timeline must resolve the
  // target, scroll to it, and apply the highlight -- even when the target
  // is in not-yet-loaded older history.
  //
  // The app uses a hash history (createHashHistory), so the router's
  // location -- pathname AND search params -- lives inside the URL hash
  // fragment (#/channels/<id>?messageId=...), NOT in window.location.search.
  // We must therefore rewrite the hash, not the top-level query string, or
  // Tanstack Router never sees the param and targetMessageId stays null.
  await page.evaluate((targetId) => {
    const hash = window.location.hash.replace(/^#/, "") || "/";
    const [path, query = ""] = hash.split("?");
    const params = new URLSearchParams(query);
    params.set("messageId", targetId);
    const nextHash = `#${path}?${params.toString()}`;
    window.history.pushState({}, "", `${window.location.pathname}${nextHash}`);
    window.dispatchEvent(new HashChangeEvent("hashchange"));
    window.dispatchEvent(new PopStateEvent("popstate"));
  }, targetId);

  // Wait for the target row to appear in the DOM. Note: data-message-id
  // values are nostr event ids (lowercase hex), so no CSS-escape is needed
  // for the Playwright locator. The `evaluate` block below runs in
  // browser context and can use CSS.escape directly.
  const targetRow = timeline.locator(`[data-message-id="${targetId}"]`);
  await expect(targetRow).toBeVisible({ timeout: 5_000 });

  // (b) Geometry: target row sits inside the timeline viewport AND near
  // the vertical center. "Near center" is defined as within one row-height
  // worth of slack of half the timeline's clientHeight -- generous enough
  // to tolerate centering implementation choices but tight enough to catch
  // "row is technically visible but at the top edge" regressions.
  const placement = await timeline.evaluate((timelineEl, id) => {
    const t = timelineEl as HTMLDivElement;
    const row = t.querySelector<HTMLElement>(
      `[data-message-id="${CSS.escape(id)}"]`,
    );
    if (!row) {
      return null;
    }
    const tRect = t.getBoundingClientRect();
    const rRect = row.getBoundingClientRect();
    return {
      rowTopRelative: rRect.top - tRect.top,
      rowBottomRelative: rRect.bottom - tRect.top,
      timelineHeight: tRect.height,
      rowHeight: rRect.height,
      className: row.className,
    };
  }, targetId);
  expect(placement).not.toBeNull();
  const p = placement as NonNullable<typeof placement>;

  // Row is inside the timeline viewport vertically.
  expect(p.rowTopRelative).toBeGreaterThanOrEqual(0);
  expect(p.rowBottomRelative).toBeLessThanOrEqual(p.timelineHeight + 1);

  // Row's center is within half-a-viewport of the timeline's vertical center.
  // This catches "scrolled but at the very top/bottom" regressions while
  // tolerating different centering strategies (smooth scrollIntoView with
  // block: "center" vs Virtuoso scrollToIndex({ align: "center" })).
  const rowCenter = (p.rowTopRelative + p.rowBottomRelative) / 2;
  const timelineCenter = p.timelineHeight / 2;
  expect(Math.abs(rowCenter - timelineCenter)).toBeLessThanOrEqual(
    p.timelineHeight / 2,
  );

  // (c) Highlight: row's className contains the route-target-highlight
  // animation token. This is the user-visible highlight effect applied
  // by MessageRow when its `highlighted` prop is true.
  expect(p.className).toContain("route-target-highlight-fade");
});

// Criterion 5: search-active match enters the timeline viewport and
// carries the active-match highlight class, even when the match is far
// from the current scroll position.
//
// In a virtualized list, "active match" cannot be implemented via
// querySelector('[data-message-id=...]') + scrollIntoView, because rows
// outside the rendered window are not in the DOM. The contract this test
// pins (regardless of virtualization strategy):
//
//   For any user-driven find-bar match selection, the matched row must
//   enter the timeline viewport and carry the route-target-highlight-fade
//   className.
//
// Test method (black-box only):
//   1. Seed the channel with a generic message bulk plus two distinct
//      needles: NEEDLE-ALPHA early in the history, NEEDLE-BRAVO later.
//   2. Open the channel; user is at the bottom (most recent).
//   3. Open the find bar via Cmd/Ctrl+F. Type the ALPHA needle.
//   4. Assert: the row matching ALPHA is in the timeline viewport and
//      carries the highlight class.
//   5. Replace the query with the BRAVO needle.
//   6. Assert the same two properties on the BRAVO row.
//
// Centeredness is intentionally NOT asserted -- see the comment on
// `assertInViewportAndHighlighted` below for why edge-bias is correct
// browser behavior, not a scroll regression.
//
// On main (non-virtualized) this is expected to pass: querySelector +
// scrollIntoView in MessageTimeline.tsx:165-174 finds the row because all
// loaded messages are in the DOM. The blind spot -- which this test will
// catch on the Virtuoso branch if not handled -- is recycling: any
// implementation where rows outside the rendered window are removed from
// the DOM must route active-match selection through a key->index map,
// not querySelector.
//
// Per Mari's baseline-either-way rule, recording the contract here is
// valuable even though main happens to satisfy it by construction.
test("find-bar active match scrolls and highlights row regardless of position", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  const ALPHA = "NEEDLE-ALPHA-c7b3";
  const BRAVO = "NEEDLE-BRAVO-9f21";
  const TOTAL = 200;
  const ALPHA_INDEX = 20; // near the top after backfill
  const BRAVO_INDEX = 110; // mid-channel

  await page.evaluate(
    ({ total, alpha, bravo, alphaIndex, bravoIndex }) => {
      for (let i = 0; i < total; i += 1) {
        let body = `filler message ${i}`;
        if (i === alphaIndex) {
          body = `filler message ${i}\n${alpha}`;
        } else if (i === bravoIndex) {
          body = `filler message ${i}\n${bravo}`;
        }
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "general",
          content: body,
        });
      }
    },
    {
      total: TOTAL,
      alpha: ALPHA,
      bravo: BRAVO,
      alphaIndex: ALPHA_INDEX,
      bravoIndex: BRAVO_INDEX,
    },
  );

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const timeline = page.getByTestId("message-timeline");
  // Setup gate: confirm the channel rendered at least one message row
  // AND the user is at the bottom of the channel before opening the
  // find bar. The original draft asserted toContainText(
  //   `filler message ${TOTAL - 1}`) -- i.e. the last seeded message
  // text must be in the timeline -- which accidentally encodes a
  // non-virtualized contract (a virtualized timeline mounts only a
  // window of rows; the LAST seeded row may not be in that window).
  //
  // The at-bottom geometry assertion is load-bearing for this test:
  // the find-bar contract is "active match scrolls a non-visible row
  // into view." If the test started at a position where ALPHA was
  // already mounted/visible, the scroll-into-view path would be a
  // no-op and the test would pass vacuously. Pinning the start to
  // the bottom of the channel guarantees the matched row is far from
  // initial scroll position regardless of how the implementation
  // sizes its initial render window.
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
  await expect
    .poll(async () => {
      const m = await timeline.evaluate((el) => {
        const t = el as HTMLDivElement;
        return {
          scrollTop: t.scrollTop,
          clientHeight: t.clientHeight,
          scrollHeight: t.scrollHeight,
        };
      });
      return m.scrollTop + m.clientHeight >= m.scrollHeight - 2;
    })
    .toBe(true);

  // Open the find bar. The shortcut handler uses platform-standard
  // primary modifier (Meta on macOS, Control elsewhere). Playwright's
  // ControlOrMeta abstracts this for us.
  await page.keyboard.press("ControlOrMeta+f");
  await expect(page.getByTestId("channel-find-bar")).toBeVisible();

  const input = page.getByPlaceholder("Find in channel");

  // Poll for the row matching `needle` to settle inside the timeline
  // viewport, then return its placement + className. Polling is required
  // because the find-bar -> active-match -> scrollIntoView path is async
  // (state update, then a smooth scroll). Locator `toBeVisible` only
  // checks DOM-visible (display/visibility), not in-viewport, so it
  // can't be used as the wait condition for "the scroll completed".
  //
  // Tolerance: 1px on each edge for sub-pixel rounding. The 5s budget
  // accommodates browsers honoring smooth-scroll over long distances
  // (initial scroll position is the bottom of a 200-message channel;
  // the ALPHA row is ~180 rows up).
  const waitForRowInViewport = async (needle: string) =>
    timeline.evaluate((timelineEl, n) => {
      return new Promise<{
        rowTopRelative: number;
        rowBottomRelative: number;
        timelineHeight: number;
        className: string;
      }>((resolve, reject) => {
        const deadline = performance.now() + 5_000;
        const t = timelineEl as HTMLDivElement;
        const tick = () => {
          const rows = Array.from(
            t.querySelectorAll<HTMLElement>("[data-message-id]"),
          );
          const row = rows.find((r) => r.textContent?.includes(n)) ?? null;
          if (row) {
            const tRect = t.getBoundingClientRect();
            const rRect = row.getBoundingClientRect();
            const top = rRect.top - tRect.top;
            const bottom = rRect.bottom - tRect.top;
            const height = tRect.height;
            if (top >= -1 && bottom <= height + 1) {
              resolve({
                rowTopRelative: top,
                rowBottomRelative: bottom,
                timelineHeight: height,
                className: row.className,
              });
              return;
            }
          }
          if (performance.now() > deadline) {
            reject(
              new Error(
                `row matching "${n}" did not enter timeline viewport within 5s`,
              ),
            );
            return;
          }
          requestAnimationFrame(tick);
        };
        tick();
      });
    }, needle);

  const assertInViewportAndHighlighted = (placement: {
    rowTopRelative: number;
    rowBottomRelative: number;
    timelineHeight: number;
    className: string;
  }) => {
    // User-observable contract: row enters the timeline's own viewport and
    // carries the active-match highlight class. We intentionally do NOT
    // assert centeredness: `scrollIntoView({ block: 'center' })` legitimately
    // biases toward a list edge when the target is near the start or end
    // (e.g., the ALPHA index-20 row of a 200-message channel lands at the
    // top because the browser cannot center an item that has fewer rows
    // above it than half the viewport). Near-edge bias is correct browser
    // behavior, not a scroll-contract regression; tightening on
    // centeredness here would couple the test to scroll-anchor heuristics
    // rather than the user-observable invariant.
    expect(placement.rowTopRelative).toBeGreaterThanOrEqual(-1);
    expect(placement.rowBottomRelative).toBeLessThanOrEqual(
      placement.timelineHeight + 1,
    );
    expect(placement.className).toContain("route-target-highlight-fade");
  };

  // --- Phase 1: ALPHA ---
  await input.fill(ALPHA);
  // Sanity check: the active match should resolve and the matching row
  // should land in the DOM. visibility != in-viewport here -- we follow
  // up with `waitForRowInViewport` to enforce the placement contract.
  const alphaRow = timeline.locator(`[data-message-id]`).filter({
    hasText: ALPHA,
  });
  await expect(alphaRow).toBeVisible({ timeout: 5_000 });

  const a = await waitForRowInViewport(ALPHA);
  assertInViewportAndHighlighted(a);

  // --- Phase 2: BRAVO ---
  // Replace the query. The active-match id changes, which should drive
  // a fresh scroll + highlight on the BRAVO row.
  await input.fill(BRAVO);
  const bravoRow = timeline.locator(`[data-message-id]`).filter({
    hasText: BRAVO,
  });
  await expect(bravoRow).toBeVisible({ timeout: 5_000 });

  const b = await waitForRowInViewport(BRAVO);
  assertInViewportAndHighlighted(b);
});

// Criterion 6 (composer half): expanding the composer (multi-line input)
// must not push the bottom row out of the user-visible area of the
// timeline when the user is following the bottom. On main the composer
// is rendered as an overlay (`absolute inset-x-0 bottom-0`) on top of
// the timeline, and the timeline reserves bottom padding to keep the
// last message clear of the composer. The contract this test pins
// (regardless of overlay-vs-sibling layout strategy):
//
//   While the user is at the bottom of the timeline, growing the
//   composer from one line to several lines must keep the last
//   message's bottom edge at-or-above the composer's top edge
//   (within a small tolerance). The bottom row must not slide
//   underneath the composer chrome.
//
// Test method (black-box only):
//   1. Seed the channel with enough messages to make the timeline
//      scrollable. The last message text is unique so we can address
//      it.
//   2. Open the channel. The view starts at the bottom by default.
//   3. Record the gap between the last message's bottom edge and the
//      composer's top edge. The bottom row must sit at-or-above the
//      composer top (gap >= 0).
//   4. Focus the composer and press Shift+Enter several times to grow
//      it into a multi-line state.
//   5. Wait for the composer to actually grow (its bounding rect grows
//      taller, equivalently its top moves up).
//   6. Assert the bottom-row-to-composer-top gap is still >= 0
//      (within tolerance). The timeline must have either auto-scrolled
//      to follow output or kept enough bottom padding to clear the
//      enlarged composer.
//
// Tolerance: 4px. Accounts for sub-pixel rendering and a single rAF
// of lag between composer resize and follow-output. We intentionally
// don't require equality -- the invariant is "the user can still see
// the bottom row clear of composer chrome," not "glued to a specific
// pixel."
test("composer expansion does not push bottom row out of viewport", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  // Seed enough messages that the timeline is scrollable. The bottom
  // message has a distinct text so we can resolve its row even if the
  // virtualizer recycles surrounding rows.
  const TOTAL = 60;
  const BOTTOM_NEEDLE = "BOTTOM-NEEDLE-3a91";
  await page.evaluate(
    ({ total, bottom }) => {
      for (let i = 0; i < total; i += 1) {
        const body = i === total - 1 ? bottom : `filler message ${i}`;
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "general",
          content: body,
        });
      }
    },
    { total: TOTAL, bottom: BOTTOM_NEEDLE },
  );

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const timeline = page.getByTestId("message-timeline");
  await expect(timeline).toContainText(BOTTOM_NEEDLE);

  // Geometry probe: report the gap between the bottom row's bottom edge
  // and the composer's top edge, plus the composer's own height so we
  // can verify the composer-driven growth separately.
  const probe = async () =>
    page.evaluate((needle) => {
      const t = document.querySelector<HTMLDivElement>(
        '[data-testid="message-timeline"]',
      );
      const c = document.querySelector<HTMLElement>(
        '[data-testid="message-composer"]',
      );
      if (!t || !c) {
        return { found: false as const };
      }
      const rows = Array.from(
        t.querySelectorAll<HTMLElement>("[data-message-id]"),
      );
      const bottomRow = rows.find((r) => r.textContent?.includes(needle));
      if (!bottomRow) {
        return { found: false as const };
      }
      const rRect = bottomRow.getBoundingClientRect();
      const cRect = c.getBoundingClientRect();
      return {
        found: true as const,
        // Positive or zero means the row's bottom sits at-or-above
        // the composer's top. Negative means the row has been pushed
        // underneath the composer overlay.
        gapAboveComposer: cRect.top - rRect.bottom,
        composerHeight: cRect.height,
      };
    }, BOTTOM_NEEDLE);

  const before = await probe();
  expect(before.found).toBe(true);
  if (!before.found) return;
  // Sanity: starting state has the bottom row clear of the composer.
  expect(before.gapAboveComposer).toBeGreaterThanOrEqual(-4);

  // Grow the composer. Shift+Enter inserts a hard break in the Tiptap
  // editor without sending; six line breaks plus typed text reliably
  // grows the composer past its single-line min-height. The inner
  // editor scroll container is height-capped at max-h-32, so growth
  // stops at ~one extra row of UI height after the cap is reached --
  // that's still enough to verify the contract: even modest composer
  // growth must not occlude the bottom row.
  const input = page.getByTestId("message-input");
  await input.click();
  for (let i = 0; i < 6; i += 1) {
    await page.keyboard.type(`line ${i}`);
    await page.keyboard.press("Shift+Enter");
  }
  await page.keyboard.type("line 6");

  // Wait for the composer growth to propagate through layout. The
  // composer's outer bounding height must visibly grow. This is the
  // smallest sanity guard against the test tautologically passing
  // when the composer didn't actually grow (e.g., focus failed).
  await expect
    .poll(
      async () => {
        const p = await probe();
        return p.found ? p.composerHeight : Number.NaN;
      },
      { timeout: 5_000 },
    )
    .toBeGreaterThan(before.composerHeight + 4);

  // The invariant: bottom row must still be clear of the composer
  // overlay. Either the timeline auto-scrolled to follow output, or
  // the reserved bottom padding grew with the composer. Both are
  // acceptable user-observable states.
  const after = await probe();
  expect(after.found).toBe(true);
  if (!after.found) return;
  expect(after.gapAboveComposer).toBeGreaterThanOrEqual(-4);
});

// Criterion 8: in-viewport content resize while scrolled up preserves the
// anchor row's position.
//
// The hook owns scroll position via `useAnchoredScroll`. When the user is
// scrolled up reading older history and a row *above* their reading row
// reflows (image decode without reserved dim metadata, link-card load,
// async embed expand, late font load, markdown that expands), the rows
// below shift on them. Before this test landed, the ResizeObserver only
// re-pinned when stuck-to-bottom; the scrolled-up case had no correction.
//
// The fix: the ResizeObserver calls the same anchor-restore primitive as
// the post-commit layout effect when anchored to a message. This test
// reproduces the scenario without touching React state — it directly
// grows a DOM row's height via a style override, which is exactly the
// kind of layout shift that previously had no correction (the messages
// array is unchanged, so the React layout effect never runs).
//
// Black-box assertions: the anchor row's top within the timeline must be
// unchanged (within 2px) before and after the synthetic above-anchor
// height growth.
test("in-viewport reflow above the anchor row does not push it down", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  // Seed enough rows that the timeline becomes scrollable with several
  // rows above whatever we anchor to.
  await page.evaluate(() => {
    for (let index = 0; index < 60; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `resize-anchor row ${index}\nsecond line ${index}\nthird line ${index}`,
      });
    }
  });

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline).toContainText("resize-anchor row 59");

  // Wait until the timeline is genuinely scrollable.
  await page.waitForFunction(() => {
    const element = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLDivElement | null;
    return element && element.scrollHeight > element.clientHeight + 800;
  });

  // Scroll to a middle position so we have rows on both sides of the anchor.
  await timeline.evaluate((element) => {
    const t = element as HTMLDivElement;
    t.scrollTop = Math.floor(t.scrollHeight / 2);
    t.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(50);

  // Capture the anchor row (top-crossing) and its baseline top within
  // the timeline. This is the row the user is reading.
  const baseline = await getFirstVisibleMessage(page);
  expect(baseline).not.toBeNull();
  if (!baseline) return;

  // Find a rendered row *above* the anchor and grow its height. This
  // mimics an in-viewport reflow (image decode, embed expansion) that
  // does NOT change the messages array, so the React layout effect
  // would not fire. The ResizeObserver is the only path that can
  // correct the resulting shift.
  const growthApplied = await timeline.evaluate((element, anchorId) => {
    const t = element as HTMLDivElement;
    const rows = Array.from(
      t.querySelectorAll<HTMLElement>("[data-message-id]"),
    );
    const anchorIndex = rows.findIndex(
      (row) => row.dataset.messageId === anchorId,
    );
    if (anchorIndex <= 0) return false;
    // Pick a row a few above the anchor so the growth is clearly above
    // the reader's eye, not at it.
    const target = rows[Math.max(0, anchorIndex - 3)];
    if (!target) return false;
    // 80px is well above the 0.5px noise floor in restoreAnchorToMessage
    // and large enough to be a visible jump if uncorrected.
    const currentHeight = target.getBoundingClientRect().height;
    target.style.minHeight = `${currentHeight + 80}px`;
    return true;
  }, baseline.id);
  expect(growthApplied).toBe(true);

  // ResizeObserver callbacks run asynchronously after layout. Poll the
  // anchor row's position; it must converge back to (or stay at) its
  // baseline top within ~2px.
  await expect
    .poll(
      async () => {
        const current = await getMessagePosition(page, baseline.id);
        return current
          ? Math.abs(current.top - baseline.top)
          : Number.POSITIVE_INFINITY;
      },
      { timeout: 3_000 },
    )
    .toBeLessThanOrEqual(2);
});
