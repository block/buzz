import { expect, test } from "@playwright/test";

for (const width of [375, 768, 1280]) {
  test(`conversation headers compose one shared frame at ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/design/components/conversation-header");
    const identities = page.locator(".conversation-header-identity");
    await expect(identities).toHaveCount(4);
    // Removing PanelHeader from the composition must fail this test.
    await expect(
      page.locator(".panel-header .conversation-header-identity"),
    ).toHaveCount(4);
    await expect(page.locator(".panel-header header")).toHaveCount(0);
    for (const header of await page.locator(".panel-header").all()) {
      const contained = await header.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        return [...element.querySelectorAll("button")].every((button) => {
          const rect = button.getBoundingClientRect();
          return rect.left >= bounds.left && rect.right <= bounds.right + 1;
        });
      });
      expect(contained).toBe(true);
    }
    for (const context of await page
      .locator(".conversation-header-context")
      .all()) {
      await expect(context).toBeVisible();
    }
  });
}

test("the live conversation retains its actions inside one shared header", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  await page.getByRole("button", { name: "design", exact: true }).click();
  const header = page.locator(".session-view > .panel-header");
  await expect(header).toHaveCount(1);
  await expect(
    header.getByRole("heading", { name: "design", exact: true }),
  ).toBeVisible();
  await expect(
    header.getByRole("button", { name: "Start Session", exact: true }),
  ).toBeVisible();
  await header.getByRole("button", { name: "3 participants" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
});
