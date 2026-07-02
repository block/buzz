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
//
// Both handles are scrolled on-screen first: at the 1.1 default text scale the
// sidebar overflows the 720px test viewport, and the click that navigates to
// channel-general auto-scrolls the sidebar to center that row — leaving the
// section headers above the scroller's clip. boundingBox() still reports those
// off-screen positions, so raw mouse coords would land on the workspace rail
// instead of the sortable rows and the drag would never activate. Scrolling the
// handles into view keeps the coords honest; the reorder itself must still
// activate, drop, and commit for the test to pass.
async function dragOver(page: Page, source: Locator, target: Locator) {
  await source.scrollIntoViewIfNeeded();
  await target.scrollIntoViewIfNeeded();
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
    await installMockBridge(page);
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

  test("07 — load-older prepend holds the anchored row without jitter or reconcile spin", async ({
    page,
  }) => {
    // Install once: addInitScript re-runs on every navigation in this page, so
    // each page.goto in the loop below re-applies the mock bridge.
    await installMockBridge(page);

    // The deep-history channel seeds 600 messages; the initial load windows to
    // the newest 200, leaving 400 older behind the until cursor — enough that
    // every run lands a genuine prepend. Reads the first row at/below the
    // viewport top and returns scrollTop, scrollHeight, and that row's on-screen
    // VIEWPORT position in ONE settled snapshot — the position the single-writer
    // restore must hold steady across the prepend.
    //
    // Waits inside the browser for a measurement-settled frame before reading.
    // The virtualizer re-windows after a scroll: for a few rAFs the mounted rows
    // can all sit above the viewport top (their absolute offsets lag the new
    // scrollTop) until the library mounts rows at the current position. That is
    // a measurement transient, NOT the scrollTop race — scrollTop is already
    // correct on those frames. Reading on such a frame would throw "no row";
    // polling for a settled frame removes the flake without touching any
    // race-detection threshold below (scrollTop value + viewportPos stability),
    // and snapshots all three fields together so they can't skew across reads.
    const sampleAnchor = (timeline: Locator) =>
      timeline.evaluate(async (scroller) => {
        const s = scroller as HTMLElement;
        for (let frame = 0; frame < 60; frame += 1) {
          const scrollerTop = s.getBoundingClientRect().top;
          const row = Array.from(
            s.querySelectorAll<HTMLElement>("[data-message-id]"),
          ).find((r) => r.getBoundingClientRect().top - scrollerTop >= 0);
          if (row) {
            return {
              viewportPos: row.getBoundingClientRect().top - scrollerTop,
              scrollTop: s.scrollTop,
              scrollHeight: s.scrollHeight,
            };
          }
          await new Promise((resolve) => requestAnimationFrame(resolve));
        }
        throw new Error("no anchor row mounted after 60 frames");
      });

    // Determinism is the bar, not pass-once. The original defect was a RACE: a
    // second restore loop (the resize-observer restoring to the pre-fetch
    // scrollTop of 0, fired by the load-older spinner's clientHeight shift)
    // fought the anchor restore frame-by-frame; last writer won, so the anchor
    // held only ~2 of 3 runs and on its losing runs scrollTop collapsed to ~0
    // (view stuck at the top, anchor lost). A single prepend can go green on a
    // lucky scheduling order, so this drives the prepend on SIX fresh page loads
    // and asserts the anchor holds on every one — a flaky-pass fails the run.
    // Fresh navigation each iteration resets the virtualizer's measurement state,
    // matching the run-to-run conditions under which the race surfaced.
    for (let run = 0; run < 6; run += 1) {
      // Force a full document reload each iteration. Navigating straight to the
      // same hash route is a same-document hash change, not a reload, so the
      // virtualizer + paginated history would carry over and later runs would
      // exhaust the older pages — defeating the per-run fresh-prepend premise.
      await page.goto("about:blank");
      await page.goto("/#/channels/feedf00d-0000-4000-8000-000000000007");
      const timeline = page.getByTestId("message-timeline");
      await expect(timeline).toBeVisible();
      await expect(
        page.locator('[data-message-id^="mock-deep-history-"]').first(),
      ).toBeVisible();

      // Scroll up to mount mid-history rows while staying clear of the load-older
      // sentinel zone (trips within 200px of the top), then let the windowed rows
      // measure off their 80px estimate so the pre-prepend anchor reading is
      // stable. The single trigger is the deliberate scrollTop = 0 below.
      await timeline.evaluate((el) => {
        el.scrollTop = 4000;
      });
      await page.waitForTimeout(300);
      await timeline.evaluate((el) => {
        el.scrollTop = 4000;
      });
      await page.waitForTimeout(150);
      const before = await sampleAnchor(timeline);
      expect(before.scrollTop).toBeGreaterThan(200);

      // Trigger exactly one prepend. Scrolling to 150 trips the load-older
      // sentinel (its rootMargin reaches 200px past the top) with
      // previousScrollTopRef pinned near the top — the condition under which the
      // resize-observer's competing restore collapsed the anchor pre-fix. After
      // the single fetchOlder lands, the anchor restore carries scrollTop deep
      // into the content, clear of the 200px sentinel zone, so the observer does
      // NOT re-fire: one clean prepend, not the re-trigger storm that scrollTop
      // 0 produces (0 keeps the sentinel tripped across every paged window down
      // to the small exhaustion-tail page, which legitimately lands the top row
      // near the top — masking the hold signal).
      await timeline.evaluate((el) => {
        el.scrollTop = 150;
      });

      // Anchor-hold gate (the race signal): poll until the restore has carried
      // scrollTop deep into the content — past where it sat before the prepend.
      // Pre-fix, the competing resize-observer restore (firing on the spinner's
      // clientHeight shift, restoring to previousScrollTopRef ~150) won often
      // enough that scrollTop stayed pinned near the top; this poll would then
      // time out, failing the run. scrollHeight grows several frames BEFORE the
      // restore moves scrollTop, so a scrollHeight gate would read mid-cycle
      // near the top — the race lives in scrollTop, so the gate watches it.
      await expect
        .poll(async () => (await sampleAnchor(timeline)).scrollTop, {
          timeout: 10_000,
        })
        .toBeGreaterThan(before.scrollTop);

      // One settled snapshot for the remaining checks so scrollHeight and
      // viewportPos come from the same frame as the held scrollTop:
      //   (a) the scroller grew by the prepended rows' height (genuine prepend),
      //   (b) the first-visible row sits where it did before the prepend.
      const after = await sampleAnchor(timeline);
      expect(after.scrollHeight).toBeGreaterThan(before.scrollHeight + 800);
      expect(Math.abs(after.viewportPos - before.viewportPos)).toBeLessThan(
        120,
      );

      // Reconcile terminates: two equal scrollTop reads 600ms apart prove the
      // rAF loop stopped. Under the double-writer bug the library re-scheduled
      // one rAF per frame for the full 5s MAX_RECONCILE_MS valve — still churning
      // 600ms apart.
      const settled1 = await timeline.evaluate((el) => el.scrollTop);
      await page.waitForTimeout(600);
      const settled2 = await timeline.evaluate((el) => el.scrollTop);
      expect(Math.abs(settled1 - settled2)).toBeLessThan(2);
    }
  });
});
