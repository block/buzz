import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

test("enterprise admission gates app startup and browser login recovers it", async ({
  page,
}) => {
  await installMockBridge(page, { enterpriseIdentityRequired: true });
  await page.goto("/");
  await expect(page.getByTestId("community-apply-error")).toBeVisible();
  await expect(
    page.getByText("enterprise sign-in required", { exact: true }),
  ).toBeVisible();
  await page.getByTestId("enterprise-sign-in").click();
  await expect(page.getByTestId("community-apply-error")).toHaveCount(0);
  await expect(page.getByTestId("channel-general")).toBeVisible();
  // The browser receives readiness/login metadata only, never a JWT.
  const storage = await page.evaluate(() => JSON.stringify(localStorage));
  expect(storage).not.toContain("Bearer ");
  expect(storage).not.toContain("assertion");
});

test("OSS startup does not require work-account login", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await expect(page.getByTestId("channel-general")).toBeVisible();
  await expect(page.getByTestId("enterprise-sign-in")).toHaveCount(0);
});
