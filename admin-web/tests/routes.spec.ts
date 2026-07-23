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
        tags: [],
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
  const card = page.locator(".feedback-record");
  await expect(card.locator(".record-provenance")).toContainText(
    "design.buzz.xyz",
  );
  await card.locator(".feedback-main-link").click();
  await expect(page).toHaveURL(`/feedback/${id}`);
  await expect(
    page.getByRole("heading", { name: "Feedback detail" }),
  ).toBeVisible();
  await expect(
    page.getByText("end of feedback", { exact: false }),
  ).toBeVisible();
});

test("feedback can be searched and filtered by community and time", async ({
  page,
}) => {
  const recent = new Date(Date.now() - 60 * 60 * 1000).toISOString();
  const old = new Date(Date.now() - 10 * 24 * 60 * 60 * 1000).toISOString();
  await page.route("**/api/admin/v1/feedback", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify([
        {
          id: "recent",
          communityId: "one",
          communityHost: "design.buzz.xyz",
          submitterPubkey: "21".repeat(32),
          category: "bug",
          bodySummary: "Composer freezes after sleep",
          receivedAt: recent,
        },
        {
          id: "old",
          communityId: "two",
          communityHost: "engineering.buzz.xyz",
          submitterPubkey: "22".repeat(32),
          category: "praise",
          bodySummary: "Calls are much more reliable",
          receivedAt: old,
        },
      ]),
    }),
  );

  await page.goto("/feedback");
  await expect(page.getByText("2 of 2 submissions")).toBeVisible();
  await page.getByRole("searchbox", { name: "Search feedback" }).fill("calls");
  await expect(page.getByText("Calls are much more reliable")).toBeVisible();
  await expect(page.getByText("Composer freezes after sleep")).toHaveCount(0);

  await page.getByRole("searchbox", { name: "Search feedback" }).fill("");
  await page.getByLabel("Community").selectOption("design.buzz.xyz");
  await expect(page.getByText("Composer freezes after sleep")).toBeVisible();
  await expect(page.getByText("Calls are much more reliable")).toHaveCount(0);

  await page.getByLabel("Community").selectOption("all");
  await page.getByLabel("Received").selectOption("day");
  await expect(page.getByText("Composer freezes after sleep")).toBeVisible();
  await expect(page.getByText("Calls are much more reliable")).toHaveCount(0);
});

test("feedback status is stored locally by feedback id", async ({ page }) => {
  await page.route("**/api/admin/v1/feedback", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify([
        {
          id: "feedback-one",
          communityId: "one",
          communityHost: "design.buzz.xyz",
          submitterPubkey: "21".repeat(32),
          category: "bug",
          bodySummary: "Composer freezes after sleep",
          receivedAt: new Date().toISOString(),
        },
      ]),
    }),
  );

  await page.goto("/feedback");
  await page.getByRole("checkbox", { name: "Acted on" }).check();
  await page.reload();
  await expect(page.getByRole("checkbox", { name: "Acted on" })).toBeChecked();
  await page.getByLabel("Status").selectOption("acted-on");
  await expect(page.getByText("Composer freezes after sleep")).toBeVisible();
  await page.getByLabel("Status").selectOption("pending");
  await expect(page.getByText("No matching feedback.")).toBeVisible();
});

test("feedback attachments render from imeta without raw markdown", async ({
  page,
}) => {
  const id = "feedback-with-attachments";
  const imageUrl = `https://design.buzz.xyz/media/${"a".repeat(64)}.png`;
  const fileUrl = `https://design.buzz.xyz/media/${"b".repeat(64)}.txt`;
  await page.route(`**/api/admin/v1/feedback/${id}`, (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        id,
        communityId: "one",
        communityHost: "design.buzz.xyz",
        eventId: "31".repeat(32),
        submitterPubkey: "21".repeat(32),
        category: "bug",
        body: `Composer froze.\n![image](${imageUrl})\n[diagnostics.txt](${fileUrl})`,
        tags: [
          [
            "imeta",
            `url ${imageUrl}`,
            "m image/png",
            `x ${"a".repeat(64)}`,
            "size 48213",
            "dim 1280x720",
            "filename screenshot.png",
          ],
          [
            "imeta",
            `url ${fileUrl}`,
            "m text/plain",
            `x ${"b".repeat(64)}`,
            "size 391",
            "filename diagnostics.txt",
          ],
        ],
        eventCreatedAt: "2026-07-17T17:25:00Z",
        receivedAt: "2026-07-17T17:30:00Z",
      }),
    }),
  );

  await page.goto(`/feedback/${id}`);
  await expect(
    page.getByText("Composer froze.", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("![image]", { exact: false })).toHaveCount(0);
  await expect(
    page.getByRole("img", { name: "screenshot.png" }),
  ).toHaveAttribute(
    "src",
    `/api/admin/v1/feedback/${id}/attachments/${"a".repeat(64)}`,
  );
  await expect(
    page.getByRole("link", { name: /diagnostics.txt/ }),
  ).toHaveAttribute(
    "href",
    `/api/admin/v1/feedback/${id}/attachments/${"b".repeat(64)}`,
  );
  const fileHeight = await page
    .locator(".file-attachment")
    .evaluate((element) => element.getBoundingClientRect().height);
  expect(fileHeight).toBeLessThan(100);
});
