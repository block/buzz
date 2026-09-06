import { expect, test } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const ISSUE_COMMENTS = [
  "First issue comment",
  "Second issue comment",
  "Third issue comment",
  "Fourth issue comment",
];
const ACTION_COMMENT = "Test: verify the Windows installer launches.";

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

test("issue comments keep technical evidence collapsed and surface human actions", async ({
  page,
}) => {
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("tab", { name: "Issues", exact: true }).click();
  const issueRow = page.getByTestId("project-issue-row").first();
  await expect(issueRow).toBeVisible({ timeout: 10_000 });
  await issueRow.getByRole("button", { name: /^ISS-/ }).click();

  const composer = page.getByTestId("project-issue-comment-composer");
  await expect(composer).toBeVisible();

  for (const comment of ISSUE_COMMENTS) {
    await composer.locator('[contenteditable="true"]').fill(comment);
    await composer.getByRole("button", { name: "Send message" }).click();
    await expect(composer.locator('[contenteditable="true"]')).toBeEmpty();
  }
  await composer.locator('[contenteditable="true"]').fill(ACTION_COMMENT);
  await composer.getByRole("button", { name: "Send message" }).click();
  await expect(page.getByText(ACTION_COMMENT, { exact: true })).toBeVisible({
    timeout: 10_000,
  });

  const timelineRows = page.getByTestId("project-issue-comment-timeline-row");
  const technicalEvidence = page.getByTestId(
    "project-issue-technical-evidence-toggle",
  );
  const historyToggle = page.getByTestId(
    "project-issue-comment-history-toggle",
  );

  await expect(timelineRows).toHaveCount(4);
  const actionRow = timelineRows.filter({ hasText: ACTION_COMMENT });
  await expect(actionRow).toHaveCount(1);
  await expect(actionRow).toContainText("Action required");
  await expect(technicalEvidence).toContainText(
    "Show 1 earlier technical evidence comment",
  );
  await expect(
    timelineRows.filter({ hasText: "First issue comment" }),
  ).toHaveCount(0);

  await technicalEvidence.click();
  await expect(timelineRows).toHaveCount(5);
  for (const comment of ISSUE_COMMENTS) {
    await expect(timelineRows.filter({ hasText: comment })).toHaveCount(1);
  }

  await historyToggle.click();
  await expect(timelineRows).toHaveCount(0);
  await expect(historyToggle).toContainText("Show 5 earlier comments");

  await historyToggle.click();
  await expect(timelineRows).toHaveCount(5);

  const actionEvent = await page.evaluate(() =>
    window.__BUZZ_E2E_SIGNED_EVENTS__?.find(
      (event) =>
        event.kind === 1 &&
        event.content === "Test: verify the Windows installer launches.",
    ),
  );
  expect(actionEvent?.tags).toContainEqual(["t", "action-required"]);
  expect(actionEvent?.tags.some((tag) => tag[0] === "p")).toBe(true);
});

test("channel issue detail has a back path to the scoped issue list", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();
  await page.getByTestId("channel-issues-trigger").click();

  const panel = page.getByTestId("channel-issues-auxiliary-pane");
  await expect(panel).toBeVisible();
  const issueRow = panel.getByTestId("project-issue-row").first();
  await expect(issueRow).toBeVisible({ timeout: 10_000 });
  await issueRow.getByRole("button", { name: /^ISS-/ }).click();
  await expect(
    panel.getByRole("button", { name: "Back to issues" }),
  ).toBeVisible();

  await panel.getByRole("button", { name: "Back to issues" }).click();
  await expect(panel.getByTestId("project-issue-row").first()).toBeVisible();
});

test("channel issues preserve an open thread and create in the linked repository", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "general",
          }) ?? false,
      ),
    )
    .toBe(true);
  await page.evaluate(
    ({ parentEventId, pubkey }) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: "Issue context reply",
        parentEventId,
        pubkey,
      }),
    {
      parentEventId: "mock-general-welcome",
      pubkey: TEST_IDENTITIES.alice.pubkey,
    },
  );

  await page.getByTestId("message-thread-summary").first().click();
  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();

  await page.getByTestId("channel-issues-trigger").click();
  const issuesPanel = page.getByTestId("channel-issues-auxiliary-pane");
  await expect(issuesPanel).toBeVisible();
  await expect(threadPanel).toBeVisible();

  await issuesPanel.getByRole("button", { name: "Create issue" }).click();
  const dialog = page.getByRole("dialog", { name: "Create an issue" });
  await expect(dialog).toBeVisible();
  await dialog.getByLabel("Title").fill("Create from channel issues");
  await dialog.getByLabel("Description").fill("Created in the channel.");
  await dialog.getByRole("button", { name: "Create issue" }).click();
  await expect(dialog).toHaveCount(0);

  const createdIssue = await page.evaluate(() =>
    window.__BUZZ_E2E_SIGNED_EVENTS__?.find(
      (event) =>
        event.kind === 1621 && event.content === "Created in the channel.",
    ),
  );
  expect(createdIssue?.tags).toContainEqual(["a", expect.any(String)]);

  await issuesPanel.getByRole("button", { name: "Close issues" }).click();
  await expect(issuesPanel).toHaveCount(0);
  await expect(threadPanel).toBeVisible();

  await threadPanel.getByRole("button", { name: "Close panel" }).click();
  await expect(threadPanel).toHaveCount(0);
});

test("issue assignees can be assigned and unassigned", async ({ page }) => {
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("tab", { name: "Issues", exact: true }).click();
  const issueRow = page.getByTestId("project-issue-row").first();
  await expect(issueRow).toBeVisible({ timeout: 10_000 });
  await issueRow.getByRole("button", { name: /^ISS-/ }).click();

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
  await expect(page.getByText("Issue unassigned.")).toBeVisible();
  await expect(unassign).toHaveCount(0, { timeout: 10_000 });
});

test("issue status can be changed from the detail rail", async ({ page }) => {
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("tab", { name: "Issues", exact: true }).click();
  const issueRow = page.getByTestId("project-issue-row").first();
  await expect(issueRow).toBeVisible({ timeout: 10_000 });
  await issueRow.getByRole("button", { name: /^ISS-/ }).click();

  const trigger = page.getByTestId("project-issue-status-trigger");
  await expect(trigger).toBeVisible({ timeout: 10_000 });

  // Label-driven states have no status event behind them and must not be
  // offered as something the user can publish.
  await trigger.click();
  await expect(
    page.getByTestId("project-issue-status-option-resolved"),
  ).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "In Progress" })).toHaveCount(
    0,
  );
  await page.getByTestId("project-issue-status-option-resolved").click();

  await expect(page.getByText("Status set to Done.")).toBeVisible({
    timeout: 10_000,
  });
  // The rail reflects the new status without a reload.
  await expect(trigger).toContainText("Done", { timeout: 10_000 });

  await page.keyboard.press("Escape");
  await trigger.click();
  await expect(
    page.getByTestId("project-issue-status-option-closed"),
  ).toBeVisible();
  await page.getByTestId("project-issue-status-option-closed").click();
  await expect(trigger).toContainText("Closed", { timeout: 10_000 });
});
