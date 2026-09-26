import { expect, test } from "@playwright/test";
import type { Locator, Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Mock-mode current-user pubkey (DEFAULT_MOCK_IDENTITY); custom channel
// sections persist under buzz-channel-sections.v1:<pubkey>.
const MOCK_PUBKEY = "deadbeef".repeat(8);
const SECTIONS_KEY = `buzz-channel-sections.v1:${MOCK_PUBKEY}`;
const SECTION_TOP = { id: "sec-top", name: "Priority", order: 0 };
const SECTION_BOTTOM = { id: "sec-bottom", name: "Archive", order: 1 };

async function seedChannelSections(page: Page) {
  await page.addInitScript(
    ({ key, sections }) => {
      window.localStorage.setItem(
        key,
        JSON.stringify({ version: 1, sections, assignments: {} }),
      );
    },
    { key: SECTIONS_KEY, sections: [SECTION_TOP, SECTION_BOTTOM] },
  );
}

// dnd-kit marks each section's wrapping row with aria-roledescription
// ="sortable" and spreads the drag listeners there, so the row is the handle.
function sectionRows(page: Page) {
  return page.locator('[aria-roledescription="sortable"]');
}

// Returns each row's own section name, so an unexpected row shows up as its
// text instead of being collapsed into a catch-all bucket.
async function sectionOrder(page: Page) {
  return sectionRows(page).evaluateAll(
    (rows, names) =>
      rows.map((row) => {
        const text = row.textContent?.trim() ?? "";
        return names.find((name) => text.startsWith(name)) ?? text;
      }),
    [SECTION_TOP.name, SECTION_BOTTOM.name],
  );
}

/**
 * The press the macOS webview reports for tap-to-click: a `pointerdown` with no
 * button held, whose `pointerup` arrives hundreds of milliseconds later or not
 * before the next input at all. `page.mouse.down()` cannot produce it — it
 * always reports a held button — so it is dispatched directly, with real
 * viewport coordinates so dnd-kit measures activation distance from the row
 * rather than from the origin.
 */
async function pressWithNoButtonHeld(target: Locator) {
  const box = await target.boundingBox();
  if (!box) throw new Error("press target is not laid out");
  const clientX = box.x + box.width / 2;
  const clientY = box.y + box.height / 2;
  await target.dispatchEvent("pointerdown", {
    button: 0,
    buttons: 0,
    isPrimary: true,
    pointerId: 1,
    pointerType: "mouse",
    clientX,
    clientY,
  });
  return { clientX, clientY };
}

test("a press reporting no button held does not arm a drag or eat the next click", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");

  const random = page.getByTestId("channel-random");
  const general = page.getByTestId("channel-general");
  await expect(random).toBeVisible();
  // The channel the click has to reach must not already be the selected one, or
  // the assertion at the end would hold with the click never landing.
  await expect(general).not.toHaveAttribute("data-active", "true");
  const target = await general.boundingBox();
  if (!target) throw new Error("channel rows are not laid out");

  const { clientX, clientY } = await pressWithNoButtonHeld(random);
  await page.mouse.move(clientX, clientY);

  // Travel far past the 6px activation distance with no release in between:
  // this is the move that used to start a drag out of a finished click.
  await page.mouse.move(
    target.x + target.width / 2,
    target.y + target.height / 2,
    { steps: 10 },
  );

  // An ordinary click, which dnd-kit swallows from its capture-phase blocker
  // whenever a drag is live.
  await page.mouse.down();
  await page.mouse.up();

  await expect(general).toHaveAttribute("data-active", "true");
});

test("a real press still drags right after one was refused", async ({
  page,
}) => {
  await seedChannelSections(page);
  await installMockBridge(page);
  await page.goto("/");

  const rows = sectionRows(page);
  await expect(rows).toHaveCount(2);
  const top = rows.filter({ hasText: SECTION_TOP.name });
  const bottom = rows.filter({ hasText: SECTION_BOTTOM.name });
  expect(await sectionOrder(page)).toEqual([
    SECTION_TOP.name,
    SECTION_BOTTOM.name,
  ]);

  const from = await top.boundingBox();
  const to = await bottom.boundingBox();
  if (!from || !to) throw new Error("section rows are not laid out");
  const toX = to.x + to.width / 2;
  const toY = to.y + to.height / 2;

  // Refuse a press, then drag for real from the same row. Nothing may be left
  // armed or half-torn-down by the refusal.
  const { clientX, clientY } = await pressWithNoButtonHeld(top);
  await page.mouse.move(clientX, clientY);
  await page.mouse.move(toX, toY, { steps: 10 });
  expect(await sectionOrder(page)).toEqual([
    SECTION_TOP.name,
    SECTION_BOTTOM.name,
  ]);

  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2 + 14);
  await page.mouse.move(toX, toY, { steps: 10 });
  await page.mouse.up();

  // The drop committed, so the order flipped: refusing button-free presses
  // costs nothing a deliberate drag needs.
  await expect
    .poll(() => sectionOrder(page))
    .toEqual([SECTION_BOTTOM.name, SECTION_TOP.name]);
});
