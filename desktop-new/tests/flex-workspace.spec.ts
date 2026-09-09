import { expect, type Page, test } from "@playwright/test";
const URL = "/design/components/flex-workspace";

async function moveTab(
  page: Page,
  name: string,
  target: string,
  edge: "center" | "right",
) {
  const source = await page
    .getByRole("tab", { name, exact: true })
    .boundingBox();
  const destination = await page
    .getByRole("tab", { name: target, exact: true })
    .locator(
      "xpath=ancestor::*[contains(@class,'flexlayout__tabset_container')][1]",
    )
    .boundingBox();
  if (!source || !destination) throw new Error("Missing docking geometry");
  await page.mouse.move(
    source.x + source.width / 2,
    source.y + source.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    source.x + source.width / 2 + 15,
    source.y + source.height / 2,
    { steps: 4 },
  );
  await page.mouse.move(
    edge === "right"
      ? destination.x + destination.width - 12
      : destination.x + destination.width / 2,
    destination.y + destination.height / 2,
    { steps: 24 },
  );
  await page.mouse.up();
}

test("native tabs stack, split back out, and reset without losing panels", async ({
  page,
}) => {
  await page.goto(URL);
  await expect(page.getByRole("tab")).toHaveCount(4);
  await expect(page.getByRole("tablist")).toHaveCount(4);
  await moveTab(page, "Three", "One", "center");
  await expect(page.getByRole("tablist")).toHaveCount(3);
  await expect(page.getByRole("tab")).toHaveCount(4);
  await moveTab(page, "Three", "Two", "right");
  await expect(page.getByRole("tablist")).toHaveCount(4);
  await moveTab(page, "Navigation", "One", "center");
  await expect(page.getByRole("tablist")).toHaveCount(3);
  await page.getByRole("button", { name: "Reset layout" }).click();
  await expect(page.getByRole("tablist")).toHaveCount(4);
});

test("continuous resizing and theme changes preserve the arrangement", async ({
  page,
}) => {
  await page.goto(URL);
  const divider = page.getByRole("separator").first();
  const before = await divider.boundingBox();
  if (!before) throw new Error("Missing splitter");
  await page.mouse.move(before.x + before.width / 2, before.y + 100);
  await page.mouse.down();
  await page.mouse.move(before.x + 55, before.y + 100, { steps: 12 });
  expect((await divider.boundingBox())?.x).toBeGreaterThan(before.x + 30);
  await page.mouse.up();
  const after = await divider.boundingBox();
  await page.getByRole("button", { name: /dark|light/i }).click();
  await expect(page.getByRole("tab")).toHaveCount(4);
  expect(await divider.boundingBox()).toEqual(after);
});

for (const width of [375, 768, 1440]) {
  test(`comparison stays inside its frame at ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto(URL);
    await expect(page.getByRole("tab")).toHaveCount(4);
    const host = await page.locator(".flex-workspace-host").boundingBox();
    if (!host) throw new Error("Missing frame");
    for (const tab of await page.getByRole("tab").all()) {
      const box = await tab.boundingBox();
      expect(box?.x).toBeGreaterThanOrEqual(host.x);
      expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual(
        host.x + host.width + 1,
      );
    }
  });
}
