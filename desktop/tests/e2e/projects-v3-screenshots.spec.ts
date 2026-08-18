import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/projects-v3-screenshots";

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

// Walks the Projects v3 workspace through its headline states so PR
// screenshots capture distinct pixels per feature (overview box, tab-strip
// plus, issue detail with inline copy link + avatar timeline, PR detail).
test("projects v3 workspace screenshot states", async ({ page }) => {
  await installMockBridge(page);
  await openBuzzProject(page);
  await expect(
    page.getByRole("button", { name: "Open Discussion" }),
  ).toHaveCount(0);

  // Single-box workspace: tabs sit above the pickers, which only appear for
  // the README overview and Files.
  const overviewTab = page.getByRole("tab", { name: "Overview" });
  const filesTab = page.getByRole("tab", { name: "Files" });
  const channelsTab = page.getByRole("tab", { name: "Channels" });
  const contributorsTab = page.getByRole("tab", { name: "Contributors" });
  const tabMenu = page.getByTestId("project-workspace-tab-menu");
  const workspacePanel = page.getByTestId("project-workspace-panel");
  const selectionRow = page.getByTestId("project-repository-selection-row");
  await expect(filesTab).toBeVisible();
  await expect(selectionRow).toBeVisible();
  await expect(
    workspacePanel.getByTestId("project-repository-selection-row"),
  ).toBeVisible();
  await expect(
    workspacePanel.getByTestId("project-workspace-tab-menu"),
  ).toHaveCount(0);
  const overviewBounds = await overviewTab.boundingBox();
  const channelsBounds = await channelsTab.boundingBox();
  const contributorsBounds = await contributorsTab.boundingBox();
  const tabMenuBounds = await tabMenu.boundingBox();
  const workspaceBounds = await workspacePanel.boundingBox();
  const selectionBounds = await selectionRow.boundingBox();
  expect(overviewBounds).not.toBeNull();
  expect(channelsBounds).not.toBeNull();
  expect(contributorsBounds).not.toBeNull();
  expect(tabMenuBounds).not.toBeNull();
  expect(workspaceBounds).not.toBeNull();
  expect(selectionBounds).not.toBeNull();
  expect(channelsBounds?.x).toBeLessThan(contributorsBounds?.x ?? 0);
  expect(overviewBounds?.x).toBe(workspaceBounds?.x);
  expect(tabMenuBounds?.y).toBeLessThan(workspaceBounds?.y ?? 0);
  expect(overviewBounds?.y).toBeLessThan(selectionBounds?.y ?? 0);
  await expect(
    workspacePanel.getByRole("heading", { name: "README", exact: true }),
  ).toBeVisible();
  await expect(
    workspacePanel.getByTestId("project-section-header-icon"),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/01-workspace-overview.png` });

  await filesTab.click();
  await expect(selectionRow).toBeVisible();
  await expect(
    workspacePanel.getByRole("heading", { name: "Files", exact: true }),
  ).toBeVisible();
  await page.getByRole("tab", { name: "Commits" }).click();
  await expect(selectionRow).toHaveCount(0);
  await expect(
    workspacePanel.getByRole("heading", { name: "Commits", exact: true }),
  ).toBeVisible();
  const commitRow = page.getByTestId("project-activity-feed-item").first();
  const commitDate = commitRow.getByTestId("project-commit-row-date");
  const commitHash = commitRow.getByTitle(/^View commit /);
  await expect(commitDate).toBeVisible();
  const commitDateBounds = await commitDate.boundingBox();
  const commitHashBounds = await commitHash.boundingBox();
  expect(commitDateBounds).not.toBeNull();
  expect(commitHashBounds).not.toBeNull();
  expect(commitDateBounds?.x).toBeGreaterThan(commitHashBounds?.x ?? 0);
  await contributorsTab.click();
  const contributorRow = page.getByTestId("project-contributor-row").first();
  await expect(
    page.getByTestId("project-contributor-row-date").first(),
  ).toBeVisible();
  const contributorBounds = await contributorRow.boundingBox();
  expect(contributorBounds).not.toBeNull();

  // Issues tab: the create action lives in the section header.
  await page.getByRole("tab", { name: "Issues", exact: true }).click();
  await expect(selectionRow).toHaveCount(0);
  const newIssueButton = workspacePanel.getByRole("button", {
    name: "New issue",
  });
  await expect(newIssueButton).toBeVisible();
  await expect(tabMenu.getByRole("button", { name: "New issue" })).toHaveCount(
    0,
  );
  const issuesHeading = workspacePanel.getByRole("heading", {
    name: "Issues",
    exact: true,
  });
  const issuesHeadingBounds = await issuesHeading.boundingBox();
  const newIssueBounds = await newIssueButton.boundingBox();
  expect(issuesHeadingBounds).not.toBeNull();
  expect(newIssueBounds).not.toBeNull();
  expect(newIssueBounds?.x).toBeGreaterThan(issuesHeadingBounds?.x ?? 0);
  const issueRow = page.getByTestId("project-issue-row").first();
  await expect(issueRow).toBeVisible({ timeout: 10_000 });
  const issueDate = issueRow.getByTestId("project-issue-row-date");
  const issueId = issueRow.getByTitle("View issue");
  await expect(issueDate).toBeVisible();
  const issueDateBounds = await issueDate.boundingBox();
  const issueIdBounds = await issueId.boundingBox();
  const issueRowBounds = await issueRow.boundingBox();
  expect(issueDateBounds).not.toBeNull();
  expect(issueIdBounds).not.toBeNull();
  expect(issueRowBounds).not.toBeNull();
  expect(contributorBounds?.height).toBe(issueRowBounds?.height);
  expect(issueDateBounds?.x).toBeGreaterThan(issueIdBounds?.x ?? 0);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/02-issues-tab.png` });

  // Issue detail: inline copy link after the title, assignees rail, and the
  // avatar comment timeline (seed comments so the timeline renders).
  await issueRow.getByRole("button", { name: /^#/ }).click();
  const composer = page.getByTestId("project-issue-comment-composer");
  await expect(composer).toBeVisible();
  await expect(tabMenu).toHaveCount(0);
  for (const comment of [
    "The palette needs the most work — starting there.",
    "Agreed. Typography pass can land separately.",
  ]) {
    await composer.locator('[contenteditable="true"]').fill(comment);
    await composer.getByRole("button", { name: "Send message" }).click();
    await expect(page.getByText(comment, { exact: true })).toBeVisible({
      timeout: 10_000,
    });
  }
  await expect(page.getByTestId("project-issue-copy-link")).toBeVisible();
  await expect(
    page.getByTestId("project-issue-comment-timeline-row").first(),
  ).toBeVisible();
  // Let the "Comment posted." toast dismiss so it doesn't photobomb.
  await expect(page.getByText("Comment posted.")).toHaveCount(0, {
    timeout: 10_000,
  });
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/03-issue-detail.png` });

  // PR list: section header owns creation; detail keeps its entity header.
  await page
    .getByRole("navigation", { name: "Project breadcrumb" })
    .getByRole("button", { name: "Issues", exact: true })
    .click();
  await expect(tabMenu).toBeVisible();
  await page.getByRole("tab", { name: "Pull Request", exact: true }).click();
  await expect(
    workspacePanel.getByRole("heading", {
      name: "Pull Requests",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    workspacePanel.getByRole("button", { name: "New pull request" }),
  ).toBeVisible();
  await expect(
    tabMenu.getByRole("button", { name: "New pull request" }),
  ).toHaveCount(0);
  const prRow = page.getByTestId("project-pull-request-row").first();
  await expect(prRow).toBeVisible({ timeout: 10_000 });
  const prDate = prRow.getByTestId("project-pull-request-row-date");
  const prId = prRow.getByTitle("View pull request");
  await expect(prDate).toBeVisible();
  const prDateBounds = await prDate.boundingBox();
  const prIdBounds = await prId.boundingBox();
  expect(prDateBounds).not.toBeNull();
  expect(prIdBounds).not.toBeNull();
  expect(prDateBounds?.x).toBeGreaterThan(prIdBounds?.x ?? 0);
  await prRow.getByRole("button", { name: /^#/ }).click();
  await expect(
    page.getByTestId("project-pull-request-copy-link"),
  ).toBeVisible();
  await expect(tabMenu).toHaveCount(0);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/04-pr-detail.png` });
});

test("projects v3 work-item list metadata", async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("buzz.projects.viewMode", "list");
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();

  await page.getByTestId("projects-section-prs").click();
  const pullRequestRow = page.getByTestId(/^projects-pr-row-/).first();
  await expect(pullRequestRow).toBeVisible();
  await expect(pullRequestRow).toContainText("opened this in");
  await expect(pullRequestRow).toContainText(/from\s*feature\/mock-2-0/);
  await expect(pullRequestRow).not.toContainText(/#[0-9a-f]{8}/);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/05-pr-list-metadata.png` });

  await page.getByTestId("projects-section-issues").click();
  const issueRow = page.getByTestId(/^projects-issue-row-/).first();
  await expect(issueRow).toBeVisible();
  await expect(issueRow).toContainText(/opened this in\s*relay-tools/);
  await expect(issueRow).not.toContainText(/#[0-9a-f]{8}/);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/06-issue-list-metadata.png` });
});
