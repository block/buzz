import { expect, type Locator, type Page, test } from "@playwright/test";

const URL = "/design/components/workspace";
const GROUP = ".bento-workspace .dv-groupview";
const panel = (page: Page, name: string) =>
  page
    .locator(GROUP)
    .filter({ has: page.getByRole("heading", { name, exact: true }) });
async function bounds(locator: Locator) {
  const value = await locator.boundingBox();
  if (!value) throw new Error("Missing panel geometry");
  return value;
}
async function start(page: Page, name: string) {
  const h = await bounds(panel(page, name).locator(".panel-header"));
  await page.mouse.move(h.x + 50, h.y + h.height / 2);
  await page.mouse.down();
  await page.mouse.move(h.x + 65, h.y + h.height / 2, { steps: 3 });
}
async function aim(page: Page, name: string, edge: string) {
  const b = await bounds(panel(page, name).locator(".dv-content-container"));
  await page.mouse.move(
    edge === "left"
      ? b.x + 8
      : edge === "right"
        ? b.x + b.width - 8
        : b.x + b.width / 2,
    edge === "top"
      ? b.y + 8
      : edge === "bottom"
        ? b.y + b.height - 8
        : b.y + b.height / 2,
    { steps: 20 },
  );
}

for (const edge of ["left", "right"]) {
  test(`content can split ${edge} while navigation stays put`, async ({
    page,
  }) => {
    await page.goto(URL);
    await expect(page.locator(GROUP)).toHaveCount(4);
    const nav = await bounds(panel(page, "Navigation"));
    await start(page, "Three");
    await aim(page, "One", edge);
    const overlay = page.locator(`.bento-dock-theme .dv-drop-target-${edge}`);
    await expect(overlay).toBeVisible();
    const color = await overlay.evaluate((e) => ({
      actual: getComputedStyle(e).backgroundColor,
      expected: getComputedStyle(e).getPropertyValue("--purple-3"),
    }));
    // Resolve the ramp through the browser, not through a copied palette.
    expect(
      await overlay.evaluate((e) => {
        const probe = document.createElement("div");
        probe.style.background = "var(--purple-3)";
        e.parentElement?.append(probe);
        const expected = getComputedStyle(probe).backgroundColor;
        probe.remove();
        return getComputedStyle(e).backgroundColor === expected;
      }),
    ).toBe(true);
    expect(color.actual).not.toBe("rgba(0, 0, 0, 0)");
    await page.mouse.up();
    const one = await bounds(panel(page, "One"));
    const three = await bounds(panel(page, "Three"));
    expect(edge === "left" ? three.x < one.x : three.x > one.x).toBe(true);
    const afterNav = await bounds(panel(page, "Navigation"));
    expect(afterNav.x).toBeCloseTo(nav.x, 0);
    expect(afterNav.y).toBeCloseTo(nav.y, 0);
    for (const p of await page.locator(GROUP).all()) {
      const b = await bounds(p);
      expect(b.width).toBeGreaterThanOrEqual(178);
      expect(b.height).toBeGreaterThanOrEqual(118);
    }
  });
}

test("navigation cannot be dragged, split above/left, or bypassed at the workspace edge", async ({
  page,
}) => {
  await page.goto(URL);
  await expect(page.locator(GROUP)).toHaveCount(4);
  const original = await page
    .locator(GROUP)
    .evaluateAll((els) => els.map((e) => e.getBoundingClientRect().toJSON()));
  await start(page, "Navigation");
  await aim(page, "One", "right");
  await expect(
    page.locator(".bento-dock-theme .dv-drop-target-anchor"),
  ).toHaveCount(0);
  await page.mouse.up();
  for (const edge of ["top", "left", "center"]) {
    await start(page, "Three");
    await aim(page, "Navigation", edge);
    await expect(
      page.locator(".bento-dock-theme .dv-drop-target-anchor"),
    ).toHaveCount(0);
    await page.mouse.up();
  }
  expect(
    await page
      .locator(GROUP)
      .evaluateAll((els) => els.map((e) => e.getBoundingClientRect().toJSON())),
  ).toEqual(original);
});

test("a panel fits below navigation, but a too-small split has no highlight or commit", async ({
  page,
}) => {
  await page.goto(URL);
  await expect(page.locator(GROUP)).toHaveCount(4);
  const nav = await bounds(panel(page, "Navigation"));
  await start(page, "Three");
  await aim(page, "Navigation", "bottom");
  await expect(
    page.locator(".bento-dock-theme .dv-drop-target-bottom"),
  ).toBeVisible();
  await page.mouse.up();
  const lower = await bounds(panel(page, "Three"));
  expect(lower.y).toBeGreaterThan(nav.y);
  expect(lower.x).toBeCloseTo(nav.x, 0);

  const original = await page
    .locator(GROUP)
    .evaluateAll((els) => els.map((e) => e.getBoundingClientRect().toJSON()));
  // Navigation column is deliberately narrow; it cannot become two usable columns.
  await start(page, "Two");
  await aim(page, "Navigation", "right");
  await expect(
    page.locator(".bento-dock-theme .dv-drop-target-anchor"),
  ).toHaveCount(0);
  await page.mouse.up();
  expect(
    await page
      .locator(GROUP)
      .evaluateAll((els) => els.map((e) => e.getBoundingClientRect().toJSON())),
  ).toEqual(original);
});

test("splitting vertically is available after making room by resizing", async ({
  page,
}) => {
  await page.goto(URL);
  await expect(page.locator(GROUP)).toHaveCount(4);
  const sash = await bounds(
    page
      .locator(".bento-workspace .dv-vertical > .dv-sash-container > .dv-sash")
      .first(),
  );
  await page.mouse.move(sash.x + sash.width / 2, sash.y + 2);
  await page.mouse.down();
  await page.mouse.move(sash.x + sash.width / 2, sash.y + 100, { steps: 12 });
  await page.mouse.up();
  const one = await bounds(panel(page, "One"));
  expect(one.height).toBeGreaterThan(250);
  await start(page, "Three");
  await aim(page, "One", "top");
  await expect(
    page.locator(".bento-dock-theme .dv-drop-target-top"),
  ).toBeVisible();
  await page.mouse.up();
  expect((await bounds(panel(page, "Three"))).y).toBeLessThan(
    (await bounds(panel(page, "One"))).y,
  );
});

test("shrinking a rearranged workspace leaves navigation first and rejects cramped splits", async ({
  page,
}) => {
  await page.goto(URL);
  await start(page, "Three");
  await aim(page, "Navigation", "bottom");
  await page.mouse.up();
  await page.setViewportSize({ width: 768, height: 900 });
  await expect(page.locator(GROUP)).toHaveCount(4);
  const nav = await bounds(panel(page, "Navigation"));
  const lower = await bounds(panel(page, "Three"));
  expect(lower.x).toBeCloseTo(nav.x, 0);
  expect(lower.y).toBeGreaterThan(nav.y);
  await start(page, "Two");
  await aim(page, "Navigation", "right");
  await expect(
    page.locator(".bento-dock-theme .dv-drop-target-anchor"),
  ).toHaveCount(0);
  await page.mouse.up();
});
