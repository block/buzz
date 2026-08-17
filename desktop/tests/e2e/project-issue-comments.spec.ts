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

test("issue detail can open agent chat or seed a channel question", async ({
  page,
}) => {
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("tab", { name: "Tasks", exact: true }).click();
  const issueRow = page.getByTestId("project-issue-row").first();
  await expect(issueRow).toBeVisible({ timeout: 10_000 });
  await issueRow.getByRole("button", { name: /^#/ }).click();

  const communication = page.getByTestId(
    "project-context-communication-actions",
  );
  await expect(communication).toBeVisible();
  await page.getByTestId("project-context-chat-agent").click();
  await expect(page.getByTestId("project-agent-chat-panel")).toBeVisible();
  await expect(page.getByTestId("projects-agent-selection-item")).toHaveCount(
    1,
  );
  await page.getByRole("button", { name: "Close agent chat" }).click();
  await expect(
    page.getByTestId("project-right-panel-repository-tab"),
  ).toHaveAttribute("aria-pressed", "false");

  await page.getByTestId("project-right-panel-repository-tab").click();
  await page.getByTestId("project-context-discuss").click();
  await expect(
    page.getByTestId("project-context-channel-choices"),
  ).toBeVisible();
  await page.getByTestId("project-context-related-channel").first().click();
  await expect(page.getByTestId("message-input")).toContainText(
    "Let's talk about this task:",
  );
});

test("issue comments use the project activity timeline", async ({ page }) => {
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("tab", { name: "Tasks", exact: true }).click();
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

test("issue assignees can be assigned and unassigned", async ({ page }) => {
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("tab", { name: "Tasks", exact: true }).click();
  const issueRow = page.getByTestId("project-issue-row").first();
  await expect(issueRow).toBeVisible({ timeout: 10_000 });
  await issueRow.getByRole("button", { name: /^#/ }).click();

  const issueHeader = page
    .getByTestId("project-issue-detail")
    .locator("header")
    .first();
  await expect(issueHeader).toContainText("Task created");
  await expect(issueHeader).not.toContainText("alice");
  await expect(issueHeader.locator("img")).toHaveCount(0);
  const contextAssignment = page.getByTestId("project-context-task-assignment");
  await expect(contextAssignment).toBeVisible();
  await expect(contextAssignment.locator("img")).toHaveCount(0);
  const selfAssign = page.getByTestId("project-context-issue-self-assign");
  await expect(selfAssign).toBeVisible();
  await expect(page.getByTestId("project-context-issue-assign")).toBeVisible();
  const createTask = page
    .getByTestId("project-repository-actions-panel")
    .getByRole("button", { name: "Create task", exact: true });
  await expect(createTask).toBeVisible();
  await expect(selfAssign.locator("svg")).toHaveCount(1);
  const actionGeometry = await Promise.all(
    [selfAssign, createTask].map((action) =>
      action.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        return {
          height: bounds.height,
          left: bounds.left,
          width: bounds.width,
        };
      }),
    ),
  );
  expect(actionGeometry[0]).toEqual(actionGeometry[1]);
  await selfAssign.click();
  await expect(contextAssignment).toContainText("Assigned to me");
  await expect(page.getByTestId("project-detail-section").first()).toHaveCSS(
    "border-top-width",
    "0px",
  );

  await page.getByTestId("project-issue-assign").click();
  const candidate = page
    .locator('[data-testid^="project-assignee-result-"]')
    .first();
  await expect(candidate).toBeVisible();
  const candidateTestId = await candidate.getAttribute("data-testid");
  const assignee = candidateTestId?.replace("project-assignee-result-", "");
  if (!assignee) throw new Error("Assignee result is missing its pubkey.");
  expect(assignee).toMatch(/^[0-9a-f]{64}$/);
  await candidate.click();

  const unassign = page.getByTestId(`project-issue-unassign-${assignee}`);
  await expect(unassign).toBeVisible({ timeout: 10_000 });
  await unassign.click();
  await expect(page.getByText("Task unassigned.")).toBeVisible();
  await expect(unassign).toHaveCount(0, { timeout: 10_000 });
});
