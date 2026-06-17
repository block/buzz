import { expect, test } from "@playwright/test";
import type { Locator, Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Screenshot capture for the desktop list-virtualization pass (PR #1089). Each
// shot is gated by an assertion so a geometry regression fails the run rather
// than silently producing a misleading image. The shots are the empirical gate
// that closes the correctness residual flagged in review: the absolute-position
// / scrollMargin geometry under live dynamic measurement, and the
// content-visibility "rows stay committed" claim on the dnd-coupled surface.
// Artifacts land in test-results/virtualization/.
const SHOTS = "test-results/virtualization";

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

test.describe("list virtualization screenshots", () => {
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

    await page.screenshot({ path: `${SHOTS}/01-pulse-sticky-composer.png` });
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

    await page.screenshot({ path: `${SHOTS}/02-forum-deeplink-offscreen.png` });
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

    await page.screenshot({
      path: `${SHOTS}/03-members-both-sticky-titles.png`,
    });
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

    await page.screenshot({ path: `${SHOTS}/06a-sections-before-reorder.png` });

    // Drag "Priority" past "Archive" — onDragEnd commits arrayMove and persists
    // the new order. The drop must land for the order to flip.
    await dragOver(page, topHeader, bottomHeader);

    // The drop landed: order flipped. A no-op drag would leave it unchanged.
    await expect.poll(sectionOrder).toEqual(["Archive", "Priority"]);
    // Both section rows stayed committed in the DOM across the reorder — the
    // content-visibility invariant the divergence rests on (no unmount).
    await expect(headers).toHaveCount(2);

    await page.screenshot({ path: `${SHOTS}/06b-sections-after-reorder.png` });
  });
});
