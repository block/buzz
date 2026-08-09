import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const ISSUE_COMMENTS = [
  "First issue comment",
  "Second issue comment",
  "Third issue comment",
  "Fourth issue comment",
];

async function openBuzzProject(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByTestId("projects-section-projects").click();
  const projectEntry = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  await expect(projectEntry).toBeVisible({ timeout: 10_000 });
  await projectEntry.click();
}

test("issue comments use the project activity timeline", async ({ page }) => {
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("tab", { name: "Issues", exact: true }).click();
  const issueRow = page.getByTestId("project-issue-row").first();
  await expect(issueRow).toBeVisible({ timeout: 10_000 });
  await issueRow.getByRole("button", { name: /^#/ }).click();

  const composer = page.getByTestId("project-issue-comment-composer");
  await expect(composer).toBeVisible();

  for (const comment of ISSUE_COMMENTS) {
    await composer.locator('[contenteditable="true"]').fill(comment);
    await composer.getByRole("button", { name: "Send message" }).click();
    await expect(page.getByText(comment, { exact: true })).toBeVisible({
      timeout: 10_000,
    });
  }

  const timelineRows = page.getByTestId("project-issue-comment-timeline-row");
  const earlierComments = page.getByTestId("project-issue-earlier-comments");
  const historyToggle = page.getByTestId(
    "project-issue-comment-history-toggle",
  );

  await expect(timelineRows).toHaveCount(3);
  await expect(earlierComments).toContainText("Show 1 earlier comment");
  await expect(
    timelineRows.filter({ hasText: "First issue comment" }),
  ).toHaveCount(0);
  await expect(
    timelineRows.filter({ hasText: "Fourth issue comment" }),
  ).toHaveCount(1);

  await earlierComments.click();
  await expect(timelineRows).toHaveCount(4);
  for (const comment of ISSUE_COMMENTS) {
    await expect(timelineRows.filter({ hasText: comment })).toHaveCount(1);
  }

  await historyToggle.click();
  await expect(timelineRows).toHaveCount(0);
  await expect(historyToggle).toContainText("Show 4 earlier comments");

  await historyToggle.click();
  await expect(timelineRows).toHaveCount(4);
});
