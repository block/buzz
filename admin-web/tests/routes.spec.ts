import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.route("**/api/admin/v1/**", async (route) => {
    await route.fulfill({ contentType: "application/json", body: "[]" });
  });
});

for (const [path, heading] of [
  ["/reports", "Open reports"],
  ["/feedback", "Feedback"],
]) {
  test(`${path} supports a deep link and empty state`, async ({ page }) => {
    await page.goto(path);
    await expect(page.getByRole("heading", { name: heading })).toBeVisible();
    await expect(page.getByText("No records.")).toBeVisible();
  });
}

test("forbidden reads have an explicit state", async ({ page }) => {
  await page.route("**/api/admin/v1/reports?**", (route) =>
    route.fulfill({
      status: 403,
      contentType: "application/json",
      body: JSON.stringify({
        error: { code: "forbidden", message: "request is not authorized" },
      }),
    }),
  );
  await page.goto("/reports");
  await expect(
    page.getByRole("heading", { name: "Access denied" }),
  ).toBeVisible();
});
