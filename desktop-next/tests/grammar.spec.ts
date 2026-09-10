import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { waitForAnimations } from "../../desktop/tests/helpers/animations";

test("grammar navigation opens focused topics and preserves deep links", async ({
  page,
}) => {
  await page.goto("/design/typography");
  await page
    .getByRole("navigation", { name: "Design system", exact: true })
    .getByRole("link", { name: "Logic & grammar", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Logic & grammar", exact: true }),
  ).toBeVisible();
  const topic = page.locator("#type-roles");
  const link = page
    .getByRole("navigation", { name: "Grammar topics" })
    .getByRole("link", { name: "The 19 typography roles", exact: true });
  await link.focus();
  await link.press("Enter");
  await expect(topic).toHaveAttribute("open", "");
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
  await topic.locator("summary").press("Space");
  await expect(topic).not.toHaveAttribute("open", "");
  // A second click on the same hash must reopen a manually closed topic.
  await link.click();
  await expect(topic).toHaveAttribute("open", "");
  await page.reload();
  await expect(topic).toHaveAttribute("open", "");
  await expect(topic).toBeInViewport();
  await page.goto("/design/grammar#buzz-boundary");
  await expect(page.locator("#buzz-boundary")).toHaveAttribute("open", "");
  await expect(
    page.getByRole("rowheader", { name: "Buzz fonts and type", exact: true }),
  ).toBeVisible();
});

test("grammar tables fit both densities and themes with accessible disclosures", async ({
  page,
}, testInfo) => {
  await page.goto("/design/grammar");
  for (const width of [390, 1440]) {
    await page.setViewportSize({ width, height: 960 });
    for (const density of ["Normal", "Compact"]) {
      await page.getByRole("button", { name: density, exact: true }).click();
      for (const section of await page.locator(".grammar-section").all()) {
        if (
          !(await section.evaluate((el) => (el as HTMLDetailsElement).open))
        ) {
          await section.locator("summary").click();
        }
      }
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
    }
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
