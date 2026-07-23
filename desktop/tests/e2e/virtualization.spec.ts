import { expect, test } from "@playwright/test";
import type { Locator, Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const WATERCOOLER_CHANNEL_ID = "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11";
const FORUM_THREAD_ID = "mock-forum-release-thread";
const FORUM_DEEPLINK_REPLY_ID = "mock-forum-release-deeplink";

// Mock-mode current-user pubkey (DEFAULT_MOCK_IDENTITY). Custom channel
// sections persist under buzz-channel-sections.v1:<pubkey>, so shot 6 seeds two
// sections for this key before the app boots.
const MOCK_PUBKEY = "deadbeef".repeat(8);
const SECTION_TOP = { id: "sec-top", name: "Priority", order: 0 };
const SECTION_BOTTOM = { id: "sec-bottom", name: "Archive", order: 1 };

async function seedChannelSections(page: Page) {
  await page.addInitScript(
    ({ pubkey, sections }) => {
      window.localStorage.setItem(
        `buzz-channel-sections.v1:${pubkey}`,
        JSON.stringify({ version: 1, sections, assignments: {} }),
      );
    },
    { pubkey: MOCK_PUBKEY, sections: [SECTION_TOP, SECTION_BOTTOM] },
  );
}

// dnd-kit's PointerSensor activates only after the pointer travels past its
// 6px distance constraint, so a single move never starts a drag. This walks the
// pointer down, past the activation threshold, onto the target, then releases —
// the sequence dnd-kit needs to fire onDragEnd and commit the reorder.
async function dragOver(page: Page, source: Locator, target: Locator) {
  const from = await source.boundingBox();
  const to = await target.boundingBox();
  if (!from || !to) throw new Error("drag handles not laid out");
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2 + 10);
  await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, {
    steps: 10,
  });
  await page.mouse.up();
}

test.describe("list virtualization", () => {
  test("01 — Pulse windowed feed with sticky composer pinned mid-scroll", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("open-pulse-view").click();

    // The seeded feed overflows the viewport (30 notes), so the windowed list
    // renders a subset and the composer stays pinned. Wait for virtual rows.
    const rows = page.locator("[data-index]");
    await expect(rows.first()).toBeVisible();
    const composer = page.locator(".pulse-composer");
    await expect(composer).toBeVisible();

    // Scroll the feed mid-list, then prove the sticky composer is still pinned
    // at the top of its scroll container — this exercises the
    // translateY(start - scrollMargin) offset under a non-zero scrollTop.
    const scroller = composer.locator(
      "xpath=ancestor::*[contains(@class,'overflow-y-auto')][1]",
    );
    await scroller.evaluate((el) => {
      el.scrollTop = 600;
    });
    await expect
      .poll(async () =>
        composer.evaluate(
          (el, scrollEl) => {
            const composerTop = el.getBoundingClientRect().top;
            const scrollTop = (scrollEl as HTMLElement).getBoundingClientRect()
              .top;
            return Math.abs(composerTop - scrollTop);
          },
          await scroller.elementHandle(),
        ),
      )
      .toBeLessThan(80);
  });

  test("02 — forum deep-link lands on an offscreen reply", async ({ page }) => {
    await installMockBridge(page);
    await page.goto(
      `/#/channels/${WATERCOOLER_CHANNEL_ID}/posts/${FORUM_THREAD_ID}?replyId=${FORUM_DEEPLINK_REPLY_ID}`,
    );

    // The deep-link target is the last of 25 replies — offscreen at open. Under
    // content-visibility the row stays queryable, so scrollIntoView lands it.
    const target = page.locator(
      `[data-forum-event-id="${FORUM_DEEPLINK_REPLY_ID}"]`,
    );
    await expect(target).toBeVisible();
    await expect(target).toContainText("Deep-link target");
    // Assert the row sits within the viewport vertically — proves the scroll
    // actually moved to it rather than leaving it below the fold.
    await expect
      .poll(async () =>
        target.evaluate((el) => {
          const rect = el.getBoundingClientRect();
          return rect.top >= 0 && rect.bottom <= window.innerHeight;
        }),
      )
      .toBe(true);
  });

  test("03 — members search shows both sticky titles under content-visibility", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey:
            "554cef57437abac34522ac2c9f0490d685b72c80478cf9f7ed6f9570ee8624ea",
          name: "charlie",
          status: "stopped",
        },
      ],
    });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await page.getByTestId("channel-members-trigger").click();
    await expect(page.getByTestId("members-sidebar")).toBeVisible();

    // "a" matches member `alice` (Members section) and non-member `charlie`
    // (Not in this channel section) — both heterogeneous lists + both sticky
    // titles must stay alive under content-visibility.
    await page.getByTestId("channel-management-search-users").fill("a");
    await expect(page.getByText("Members", { exact: true })).toBeVisible();
    await expect(
      page.getByText("Not in this channel", { exact: true }),
    ).toBeVisible();
    // A member row and an add-search (non-member) row both rendered.
    await expect(
      page.getByTestId("members-sidebar-people").getByText("alice"),
    ).toBeVisible();
    await expect(
      page.locator('[data-testid^="channel-user-search-result-"]').first(),
    ).toBeVisible();
  });

  test("06 — custom-section dnd reorder commits under content-visibility", async ({
    page,
  }) => {
    await seedChannelSections(page);
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // dnd-kit marks each section's wrapping row with role="button" +
    // aria-roledescription="sortable" and spreads the drag listeners there, so
    // the row itself is the handle. Scoping to that attribute reads the live
    // section order and excludes the inner disclosure button and the (hidden)
    // assign-to-section context-menu items that reuse the same names.
    const headers = page.locator('[aria-roledescription="sortable"]');
    const topHeader = headers.filter({ hasText: "Priority" });
    const bottomHeader = headers.filter({ hasText: "Archive" });
    await expect(topHeader).toBeVisible();
    await expect(bottomHeader).toBeVisible();
    await expect(headers).toHaveCount(2);

    const sectionOrder = async () =>
      headers.evaluateAll((rows) =>
        rows.map((row) =>
          row.textContent?.trim().startsWith("Priority")
            ? "Priority"
            : "Archive",
        ),
      );
    expect(await sectionOrder()).toEqual(["Priority", "Archive"]);

    // Drag "Priority" past "Archive" — onDragEnd commits arrayMove and persists
    // the new order. The drop must land for the order to flip.
    await dragOver(page, topHeader, bottomHeader);

    // The drop landed: order flipped. A no-op drag would leave it unchanged.
    await expect.poll(sectionOrder).toEqual(["Archive", "Priority"]);
    // Both section rows stayed committed in the DOM across the reorder — the
    // content-visibility invariant the divergence rests on (no unmount).
    await expect(headers).toHaveCount(2);
  });

  test("08 — cascading older pages never snap the viewport toward newest", async ({
    page,
  }) => {
    test.setTimeout(120_000);
    // A real CDP wheel burst is required here: assigning scrollTop does not
    // reproduce Chromium/WebKit's native wheel → scroll callback ordering. The
    // old boundary rollback moved the viewport back down before the fetch
    // committed; keep that pre-prepend reversal below the same 5px frame bar.
    // A 300ms relay delay leaves the input boundary and prepend commit as two
    // distinct phases so this assertion cannot accidentally measure only the
    // later anchor correction.
    await installMockBridge(page, {
      deepHistoryMessageCount: 1_800,
      channelWindowDelayMs: 300,
    });
    await page.goto("/#/channels/feedf00d-0000-4000-8000-000000000007");
    const timeline = page.getByTestId("message-timeline");
    await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
    // Initial bottom positioning can momentarily cross the start threshold. Let
    // any resulting page transaction settle before driving explicit crossings.
    await page.waitForTimeout(1_000);

    const sampleVisibleAnchor = (expectedId?: string) =>
      timeline.evaluate(async (scroller, anchorId) => {
        const s = scroller as HTMLElement;
        for (let frame = 0; frame < 60; frame += 1) {
          const scrollerTop = s.getBoundingClientRect().top;
          const rows = Array.from(
            s.querySelectorAll<HTMLElement>("[data-message-id]"),
          );
          const row = anchorId
            ? rows.find((candidate) => candidate.dataset.messageId === anchorId)
            : rows.find(
                (candidate) =>
                  candidate.getBoundingClientRect().top >= scrollerTop,
              );
          if (row) {
            return {
              id: row.dataset.messageId ?? "",
              top: row.getBoundingClientRect().top - scrollerTop,
              scrollHeight: s.scrollHeight,
              bottomDistance: s.scrollHeight - s.clientHeight - s.scrollTop,
            };
          }
          await new Promise((resolve) => requestAnimationFrame(resolve));
        }
        throw new Error(
          `anchor row ${anchorId ?? "at viewport top"} not mounted`,
        );
      }, expectedId);

    // Load fifteen consecutive server pages in one mounted virtualizer. This
    // is the production shape that exposed the intermittent end-cache snap:
    // variable-height rows and repeated front insertions exercise the full
    // index-shift path rather than allowing a single lucky pass.
    for (let pageIndex = 0; pageIndex < 15; pageIndex += 1) {
      // Leave the threshold first so Virtua emits a fresh start-edge crossing;
      // initial positioning can briefly report offset 0 while mounting.
      await timeline.evaluate((element) => {
        element.scrollTop = 4000;
      });
      await page.waitForTimeout(300);
      await timeline.evaluate((element) => {
        element.scrollTop = 180;
      });
      await page.waitForTimeout(150);
      const before = await sampleVisibleAnchor();
      const wheelTracePromise = timeline.evaluate(async (scroller) => {
        const s = scroller as HTMLElement;
        let previousScrollTop = s.scrollTop;
        let maxBoundaryRollback = 0;
        let minScrollTop = s.scrollTop;
        const deadline = performance.now() + 120;
        while (performance.now() < deadline) {
          maxBoundaryRollback = Math.max(
            maxBoundaryRollback,
            s.scrollTop - previousScrollTop,
          );
          previousScrollTop = s.scrollTop;
          minScrollTop = Math.min(minScrollTop, s.scrollTop);
          await new Promise((resolve) => requestAnimationFrame(resolve));
        }
        return { maxBoundaryRollback, minScrollTop };
      });
      const box = await timeline.boundingBox();
      if (!box) throw new Error("timeline has no bounding box");
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
      for (const deltaY of [-60, -30, -20, -15]) {
        await page.mouse.wheel(0, deltaY);
        await page.waitForTimeout(12);
      }
      const wheelTrace = await wheelTracePromise;
      expect(wheelTrace.minScrollTop).toBeLessThanOrEqual(350);
      expect(wheelTrace.maxBoundaryRollback).toBeLessThan(5);
      // Linux Chromium delivers CDP wheel input with more latency than macOS,
      // so the burst's final delta can land AFTER the anchor baseline sample
      // below. maxDrift then reports the reader's own last wheel event as
      // anchor drift (measured exactly 15 = the -15 delta; changing the delta
      // to -17 made the failure read 17). Gate the baseline on input settle:
      // two consecutive frames with identical scrollTop. This tightens the
      // assertion rather than diluting it — the baseline becomes honest and
      // genuine post-prepend drift still reads as drift.
      await timeline.evaluate(async (element) => {
        let prior = element.scrollTop;
        for (let frame = 0; frame < 30; frame += 1) {
          await new Promise((resolve) => requestAnimationFrame(resolve));
          if (element.scrollTop === prior) break;
          prior = element.scrollTop;
        }
      });
      const committedAnchor = await sampleVisibleAnchor(before.id);
      const motion = await timeline.evaluate(
        async (scroller, { anchorId, anchorTop, oldHeight }) => {
          const s = scroller as HTMLElement;
          let maxDrift = 0;
          let sawPrepend = false;
          let sawAnchorAfterPrepend = false;
          let finalDrift = 0;
          let stableFrames = 0;
          for (let frame = 0; frame < 180; frame += 1) {
            const row = Array.from(
              s.querySelectorAll<HTMLElement>("[data-message-id]"),
            ).find((candidate) => candidate.dataset.messageId === anchorId);
            if (s.scrollHeight > oldHeight + 800 && !sawPrepend) {
              sawPrepend = true;
            }
            if (row) {
              const top =
                row.getBoundingClientRect().top - s.getBoundingClientRect().top;
              const drift = Math.abs(top - anchorTop);
              if (sawPrepend) {
                maxDrift = Math.max(maxDrift, drift);
                sawAnchorAfterPrepend = true;
                stableFrames =
                  Math.abs(drift - finalDrift) < 0.5 ? stableFrames + 1 : 0;
                finalDrift = drift;
              }
            }
            if (sawAnchorAfterPrepend && stableFrames >= 8) break;
            await new Promise((resolve) => requestAnimationFrame(resolve));
          }
          return { maxDrift, sawPrepend };
        },
        {
          anchorId: committedAnchor.id,
          anchorTop: committedAnchor.top,
          oldHeight: before.scrollHeight,
        },
      );
      expect(motion.sawPrepend).toBe(true);
      expect(motion.maxDrift).toBeLessThan(5);

      await expect
        .poll(
          async () => timeline.evaluate((element) => element.scrollHeight),
          {
            timeout: 10_000,
          },
        )
        .toBeGreaterThan(before.scrollHeight + 800);

      // A snap to newest leaves this near zero. Keep a full viewport of history
      // below the reader after every prepend, rather than checking only the
      // final page and missing a transient cascade failure.
      const bottomDistance = await timeline.evaluate(
        (element) =>
          element.scrollHeight - element.clientHeight - element.scrollTop,
      );
      expect(bottomDistance).toBeGreaterThan(
        await timeline.evaluate((element) => element.clientHeight),
      );

      // Leave the boundary with real downward wheel input while this prepend's
      // three-second semantic-anchor watcher is still alive. The watcher belongs
      // only to the completed prepend: it must not reinterpret this deliberate
      // reader movement as row drift and pull the viewport back toward its stale
      // baseline before the next upward load.
      const exitTracePromise = timeline.evaluate(async (scroller) => {
        const s = scroller as HTMLElement;
        const startScrollTop = s.scrollTop;
        let previousScrollTop = startScrollTop;
        let maxForwardTravel = 0;
        let maxRollback = 0;
        const deadline = performance.now() + 400;
        while (performance.now() < deadline) {
          const travel = s.scrollTop - startScrollTop;
          maxForwardTravel = Math.max(maxForwardTravel, travel);
          maxRollback = Math.max(maxRollback, previousScrollTop - s.scrollTop);
          previousScrollTop = s.scrollTop;
          await new Promise((resolve) => requestAnimationFrame(resolve));
        }
        return { maxForwardTravel, maxRollback };
      });
      const exitBox = await timeline.boundingBox();
      if (!exitBox) throw new Error("timeline has no bounding box");
      await page.mouse.move(
        exitBox.x + exitBox.width / 2,
        exitBox.y + exitBox.height / 2,
      );
      for (const deltaY of [120, 100, 80]) {
        await page.mouse.wheel(0, deltaY);
        await page.waitForTimeout(12);
      }
      const exitTrace = await exitTracePromise;
      expect(exitTrace.maxForwardTravel).toBeGreaterThan(200);
      expect(exitTrace.maxRollback).toBeLessThan(5);

      // Loading more history must not return keepMounted to its old linear
      // growth. Virtua still retains every measured size for spacer geometry;
      // only the live message-row DOM stays bounded around the reader and tail.
      const mountedMessageCount = await timeline
        .locator("[data-message-id]")
        .count();
      expect(mountedMessageCount).toBeLessThan(400);
    }
  });

  test("09 — older-page render commit waits for scroller rest under continued wheel input", async ({
    page,
  }) => {
    test.setTimeout(60_000);
    // Production shape for the WKWebView dropped-write hazard: heavy
    // variable-height rows, a slow older-page fetch, and wheel input that
    // KEEPS ARRIVING through fetch resolution. Every prepend-compensation
    // mechanism is a scrollTop write, and macOS WebKit can drop those writes
    // while trackpad momentum owns the offset — so the contract under test is
    // that the fetched page's RENDER COMMIT (the scrollHeight jump) is
    // deferred until input quiesces, and that the at-rest commit then holds
    // the anchored row. Chromium cannot reproduce the dropped write itself;
    // it CAN prove the commit-at-rest scheduling that makes it unreachable.
    await installMockBridge(page, {
      deepHistoryMessageCount: 1_800,
      channelWindowDelayMs: 300,
    });
    await page.goto("/#/channels/feedf00d-0000-4000-8000-000000000007");
    const timeline = page.getByTestId("message-timeline");
    await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
    await page.waitForTimeout(1_000);

    // Mount mid-history rows clear of the load-older sentinel, then trip it.
    await timeline.evaluate((element) => {
      element.scrollTop = 4000;
    });
    await page.waitForTimeout(300);

    // In-page observer: tracks the last wheel-input timestamp, captures the
    // first at-rest anchor after input stops, and records when the prepend
    // commit (scrollHeight jump) lands relative to the last input.
    const tracePromise = timeline.evaluate(async (scroller) => {
      const s = scroller as HTMLElement;
      const baseHeight = s.scrollHeight;
      let lastInputTs = 0;
      let sawInput = false;
      let restAnchor: { id: string; top: number } | null = null;
      const onWheel = () => {
        lastInputTs = performance.now();
        sawInput = true;
        // Input after a lull invalidates any anchor captured during it —
        // the commit must be measured against the FINAL at-rest position.
        restAnchor = null;
      };
      s.addEventListener("wheel", onWheel, { passive: true });
      let commit: { ts: number; gapSinceInput: number } | null = null;
      let sawSpinnerDuringHold = false;
      let anchorDriftAfterCommit: number | null = null;
      const deadline = performance.now() + 8_000;
      while (performance.now() < deadline) {
        const now = performance.now();
        if (commit === null) {
          if (
            document.querySelector(
              '[data-testid="message-timeline-fetching-older"]',
            ) !== null
          ) {
            sawSpinnerDuringHold = true;
          }
          // First frame at rest (input quiet for 60ms — shorter than the
          // gate's own window, so this reading always precedes admission):
          // capture the row the at-rest commit must hold.
          if (restAnchor === null && sawInput && now - lastInputTs >= 60) {
            const scrollerTop = s.getBoundingClientRect().top;
            const row = Array.from(
              s.querySelectorAll<HTMLElement>("[data-message-id]"),
            ).find(
              (candidate) =>
                candidate.getBoundingClientRect().top - scrollerTop >= 0,
            );
            if (row?.dataset.messageId) {
              restAnchor = {
                id: row.dataset.messageId,
                top: row.getBoundingClientRect().top - scrollerTop,
              };
            }
          }
          if (s.scrollHeight > baseHeight + 800) {
            commit = { ts: now, gapSinceInput: now - lastInputTs };
          }
        } else if (restAnchor !== null) {
          const anchor = restAnchor;
          const scrollerTop = s.getBoundingClientRect().top;
          const row = Array.from(
            s.querySelectorAll<HTMLElement>("[data-message-id]"),
          ).find((candidate) => candidate.dataset.messageId === anchor.id);
          if (row) {
            anchorDriftAfterCommit = Math.max(
              anchorDriftAfterCommit ?? 0,
              Math.abs(
                row.getBoundingClientRect().top - scrollerTop - anchor.top,
              ),
            );
          }
          // Watch a settle window after the commit, then finish.
          if (now - commit.ts > 700) break;
        }
        await new Promise((resolve) => requestAnimationFrame(resolve));
      }
      s.removeEventListener("wheel", onWheel);
      return {
        commit,
        capturedRestAnchor: restAnchor !== null,
        sawSpinnerDuringHold,
        anchorDriftAfterCommit,
      };
    });

    // Trip the boundary, then keep real wheel input flowing DOWN (away from
    // the boundary) through and well past the 300ms fetch resolution — the
    // mid-gesture window in which the ungated build commits the page.
    await timeline.evaluate((element) => {
      element.scrollTop = 150;
    });
    const box = await timeline.boundingBox();
    if (!box) throw new Error("timeline has no bounding box");
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    for (let burst = 0; burst < 30; burst += 1) {
      await page.mouse.wheel(0, 30);
      await page.waitForTimeout(40);
    }

    const trace = await tracePromise;
    // The page must eventually commit — the gate defers, never strands.
    expect(trace.commit).not.toBeNull();
    // The commit landed only after input quiesced. On the ungated build the
    // deferred snapshot flushes as soon as the fetch resolves — between wheel
    // bursts, a gap far below the quiet window — so this line is the red/green
    // signal for the settle gate.
    expect(trace.commit?.gapSinceInput ?? 0).toBeGreaterThanOrEqual(80);
    // The reader saw the fetching affordance while the page was held.
    expect(trace.sawSpinnerDuringHold).toBe(true);
    // The at-rest commit held the anchored row (writes land at rest).
    expect(trace.capturedRestAnchor).toBe(true);
    expect(trace.anchorDriftAfterCommit ?? 0).toBeLessThan(5);
  });
});

test("thread-heavy history mounts every loaded row", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  // Seed summaries on 120 loaded roots. Every loaded row should be realized
  // immediately so first-pass scrolling never encounters Virtua's hidden
  // pre-measurement state.
  await page.evaluate(() => {
    for (let index = 480; index < 600; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "deep-history",
        content: `summary-only reply ${index}`,
        parentEventId: `mock-deep-history-${index}`,
      });
    }
  });

  await page.getByTestId("channel-deep-history").click();
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();

  // Emit after opening so the live summary path updates every loaded root,
  // independent of the relay page's summary cap.
  await page.evaluate(() => {
    for (let index = 480; index < 600; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "deep-history",
        content: `live summary-only reply ${index}`,
        parentEventId: `mock-deep-history-${index}`,
      });
    }
  });
  await page.waitForTimeout(500);

  await page.waitForTimeout(300);

  const loadedRows = timeline.locator("[data-message-id]");
  // The mock channel's current loaded window contains 50 roots; all of them
  // must already exist and be painted before the first scroll gesture.
  await expect(loadedRows).toHaveCount(50);
  expect(
    await loadedRows.evaluateAll((rows) =>
      rows.every((row) => getComputedStyle(row).visibility === "visible"),
    ),
  ).toBe(true);
});

test("channel switches settle the last row above the composer", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  await page.evaluate(() => {
    for (const [channelName, prefix] of [
      ["general", "switch-general"],
      ["engineering", "switch-engineering"],
    ] as const) {
      for (let index = 0; index < 60; index += 1) {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName,
          content: `${prefix}-${index}`,
          createdAt: 1_700_000_000 + index,
        });
      }
    }
  });

  for (const channelName of ["general", "engineering", "general"]) {
    await page.getByTestId(`channel-${channelName}`).click();
    await expect(page.getByTestId("chat-title")).toHaveText(channelName);
    const timeline = page.getByTestId("message-timeline");
    const composer = page.getByTestId("channel-composer-overlay");
    await expect
      .poll(async () =>
        timeline.evaluate(
          (element, composerElement) => {
            const rows = Array.from(
              element.querySelectorAll<HTMLElement>("[data-message-id]"),
            );
            const lastRow = rows.at(-1);
            if (!lastRow) return Number.POSITIVE_INFINITY;
            return (
              lastRow.getBoundingClientRect().bottom -
              (composerElement as HTMLElement).getBoundingClientRect().top
            );
          },
          await composer.elementHandle(),
        ),
      )
      .toBeLessThanOrEqual(1);
  }
});

test("offscreen rich-row resize preserves the viewport-center anchor", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );
  await page.evaluate(() => {
    for (let index = 0; index < 240; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: [
          `rich resize row ${index}`,
          "long wrapped text ".repeat((index % 8) + 1),
        ].join("\n"),
        createdAt: 1_700_700_000 + index,
      });
    }
  });

  await page.getByTestId("channel-general").click();
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();

  const result = await timeline.evaluate(async (element) => {
    const scroller = element as HTMLDivElement;
    scroller.scrollTop = scroller.scrollHeight / 2;
    scroller.dispatchEvent(new Event("scroll", { bubbles: true }));
    await new Promise((resolve) => setTimeout(resolve, 250));

    const scrollerRect = scroller.getBoundingClientRect();
    const rows = Array.from(
      scroller.querySelectorAll<HTMLElement>("[data-message-id]"),
    );
    const visibleRows = rows.filter((row) => {
      const rect = row.getBoundingClientRect();
      return rect.bottom > scrollerRect.top && rect.top < scrollerRect.bottom;
    });
    const viewportCenter = scrollerRect.top + scroller.clientHeight / 2;
    const anchor = visibleRows.reduce<HTMLElement | null>((nearest, row) => {
      if (!nearest) return row;
      const rowRect = row.getBoundingClientRect();
      const nearestRect = nearest.getBoundingClientRect();
      const rowDistance = Math.abs(
        (rowRect.top + rowRect.bottom) / 2 - viewportCenter,
      );
      const nearestDistance = Math.abs(
        (nearestRect.top + nearestRect.bottom) / 2 - viewportCenter,
      );
      return rowDistance < nearestDistance ? row : nearest;
    }, null);
    if (!anchor) throw new Error("no visible center anchor");
    const rowAbove = rows
      .filter((row) => row.getBoundingClientRect().bottom <= scrollerRect.top)
      .at(-1);
    if (!rowAbove) throw new Error("no mounted offscreen rich row above");

    const anchorRect = anchor.getBoundingClientRect();
    const anchorCenterOffset =
      (anchorRect.top + anchorRect.bottom) / 2 - viewportCenter;
    const scrollTop = scroller.scrollTop;
    rowAbove.style.minHeight = `${rowAbove.getBoundingClientRect().height + 240}px`;
    await new Promise((resolve) => setTimeout(resolve, 500));

    const nextScrollerRect = scroller.getBoundingClientRect();
    const nextAnchorRect = anchor.getBoundingClientRect();
    const nextViewportCenter = nextScrollerRect.top + scroller.clientHeight / 2;
    return {
      anchorDrift:
        (nextAnchorRect.top + nextAnchorRect.bottom) / 2 -
        nextViewportCenter -
        anchorCenterOffset,
      scrollCorrection: scroller.scrollTop - scrollTop,
    };
  });

  expect(Math.abs(result.anchorDrift)).toBeLessThanOrEqual(2);
  expect(result.scrollCorrection).toBeGreaterThanOrEqual(238);
});

test("live tail arrivals keep a bottom-pinned virtual timeline settled", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  await page.evaluate(() => {
    for (let index = 0; index < 60; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `live-follow seed ${index}\nsecond line ${index}`,
        createdAt: 1_700_000_000 + index,
      });
    }
  });
  await page.getByTestId("channel-general").click();
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline).toContainText("live-follow seed 59");
  await expect
    .poll(() =>
      timeline.evaluate(
        (element) =>
          element.scrollHeight - element.clientHeight - element.scrollTop,
      ),
    )
    .toBeLessThanOrEqual(1);

  await page.evaluate(() => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content:
        "remote live-follow sentinel\nline two\nline three\nline four\nline five",
      createdAt: 1_800_000_000,
    });
  });

  await expect(page.getByText("remote live-follow sentinel")).toBeVisible();
  await expect
    .poll(() =>
      timeline.evaluate(
        (element) =>
          element.scrollHeight - element.clientHeight - element.scrollTop,
      ),
    )
    .toBeLessThanOrEqual(1);
});

test("live tail arrivals stay buffered while reading and release on jump", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );
  await page.getByTestId("channel-deep-history").click();

  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
  await timeline.evaluate((element) => {
    element.scrollTop = Math.max(500, element.scrollHeight / 2);
    element.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect(page.getByTestId("message-scroll-to-latest")).toBeVisible();
  const frozenHeight = await timeline.evaluate(
    (element) => element.scrollHeight,
  );

  await page.evaluate(() => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "deep-history",
      content: "buffered live tail sentinel",
      createdAt: 1_900_000_000,
    });
  });

  await expect(page.getByText("buffered live tail sentinel")).toHaveCount(0);
  await expect(page.getByTestId("message-scroll-to-latest")).toContainText("1");
  expect(await timeline.evaluate((element) => element.scrollHeight)).toBe(
    frozenHeight,
  );

  await page.getByTestId("message-scroll-to-latest").click();
  await expect(page.getByText("buffered live tail sentinel")).toBeVisible();
});
