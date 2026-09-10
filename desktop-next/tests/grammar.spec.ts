import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { waitForAnimations } from "../../desktop/tests/helpers/animations";

test("grammar shows all tables and preserves topic links", async ({ page }) => {
  await page.goto("/design/typography");
  await page
    .getByRole("navigation", { name: "Design system", exact: true })
    .getByRole("link", { name: "Logic & grammar", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Logic & grammar", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".grammar-page details")).toHaveCount(0);
  await expect(page.locator(".grammar-page table")).toHaveCount(15);
  for (const table of await page.locator(".grammar-page table").all()) {
    await expect(table).toBeVisible();
  }
  const topic = page.locator("#type-roles");
  const link = page
    .getByRole("navigation", { name: "Grammar topics" })
    .getByRole("link", { name: "The 19 typography roles", exact: true });
  await link.focus();
  await link.press("Enter");
  await expect(topic.locator("tbody tr")).toHaveCount(19);
  await expect(
    topic.getByRole("rowheader", { name: "body/label-medium", exact: true }),
  ).toBeVisible();
  await expect(
    topic.getByRole("link", { name: "type.roles.md", exact: true }),
  ).toHaveAttribute(
    "href",
    /github\.com\/squareup\/design-blockinterface\/blob\/[a-f0-9]{40}\/blockUI\/docs\/type\.roles\.md$/,
  );
  await page.reload();
  await expect(topic).toBeInViewport();
  await page.goto("/design/grammar#buzz-boundary");
  await expect(
    page.getByRole("rowheader", { name: "Buzz fonts and type", exact: true }),
  ).toBeVisible();
});

test("grammar tables fit both densities and themes", async ({
  page,
}, testInfo) => {
  await page.goto("/design/grammar");
  for (const width of [390, 1440]) {
    await page.setViewportSize({ width, height: 960 });
    for (const density of ["Normal", "Compact"]) {
      await page.getByRole("button", { name: density, exact: true }).click();
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
    }
    const audit = await new AxeBuilder({ page })
      .disableRules(["color-contrast"])
      .analyze();
    expect(audit.violations).toEqual([]);
  }
  for (const mode of ["light", "dark"]) {
    if (mode === "dark")
      await page.getByRole("button", { name: "Switch to dark mode" }).click();
    const audit = await new AxeBuilder({ page })
      .disableRules(["color-contrast"])
      .analyze();
    expect(audit.violations).toEqual([]);
    await page.locator("#type-roles").scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await page.screenshot({ path: testInfo.outputPath(`grammar-${mode}.png`) });
  }
});
