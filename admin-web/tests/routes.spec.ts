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

test("report rows render the relay response contract", async ({ page }) => {
  await page.route("**/api/admin/v1/reports?**", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify([
        {
          id: "0e6caad8-1e18-4cd7-84fa-7264103f0a08",
          communityId: "6d474feb-c50a-44e4-a0b5-f30532df49bc",
          communityHost: "design.buzz.xyz",
          reporterPubkey: "21".repeat(32),
          targetKind: "event",
          target: "12".repeat(32),
          reportType: "spam",
          status: "open",
          createdAt: "2026-07-17T17:30:00Z",
        },
      ]),
    }),
  );
  await page.goto("/reports");
  await expect(page.getByText("design.buzz.xyz")).toBeVisible();
  await expect(page.getByText("spam")).toBeVisible();
  await expect(page.getByText("Unknown date")).toHaveCount(0);
});

test("feedback cards open the complete submission", async ({ page }) => {
  const id = "feed0000-0000-4000-8000-000000000001";
  const fullBody = `${"Long feedback ".repeat(30)}end of feedback`;
  await page.route(`**/api/admin/v1/feedback/${id}`, (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        id,
        communityId: "6d474feb-c50a-44e4-a0b5-f30532df49bc",
        communityHost: "design.buzz.xyz",
        eventId: "31".repeat(32),
        submitterPubkey: "21".repeat(32),
        category: "needs-work",
        body: fullBody,
        eventCreatedAt: "2026-07-17T17:25:00Z",
        receivedAt: "2026-07-17T17:30:00Z",
      }),
    }),
  );
  await page.route("**/api/admin/v1/feedback", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify([
        {
          id,
          communityId: "6d474feb-c50a-44e4-a0b5-f30532df49bc",
          communityHost: "design.buzz.xyz",
          submitterPubkey: "21".repeat(32),
          category: "needs-work",
          bodySummary: `${fullBody.slice(0, 240)}…`,
          receivedAt: "2026-07-17T17:30:00Z",
        },
      ]),
    }),
  );

  await page.goto("/feedback");
  await expect(page.getByText("design.buzz.xyz")).toBeVisible();
  await page.locator(`a[href="/feedback/${id}"]`).click();
  await expect(page).toHaveURL(`/feedback/${id}`);
  await expect(
    page.getByRole("heading", { name: "Feedback detail" }),
  ).toBeVisible();
  await expect(
    page.getByText("end of feedback", { exact: false }),
  ).toBeVisible();
});
