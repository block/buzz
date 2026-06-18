import { expect, test } from "@playwright/test";

test("home page loads with Buzz heading", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("header")).toContainText("Buzz");
});

test("home page shows repositories section", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("Repositories")).toBeVisible();
});
