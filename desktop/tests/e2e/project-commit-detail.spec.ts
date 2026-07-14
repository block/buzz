import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/project-commit-detail";

// The projects surface is a preview feature — opt in before the app mounts.
// Must run before installMockBridge so React reads the override on mount.
async function enableProjectsFeature(page: import("@playwright/test").Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "buzz-feature-overrides-v1",
      JSON.stringify({ projects: true }),
    );
  });
}

test("commit detail opens from the commits feed with a diff", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  // The preview server is a static file server without SPA fallback, so
  // enter at "/" and navigate via the sidebar.
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();

  // The overview no longer lists repository cards — switch to the
  // Repositories filter to reveal the project cards/rows.
  await page.getByRole("button", { name: "Repositories", exact: true }).click();

  // Open the first mock project (dtag "buzz" from the e2e bridge fixture).
  const projectEntry = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  await expect(projectEntry).toBeVisible({ timeout: 10_000 });
  await projectEntry.click();

  await page.getByRole("tab", { name: "Commits" }).click();
  const commitRows = page.getByTestId("project-activity-feed-item");
  await expect(commitRows.first()).toBeVisible({ timeout: 10_000 });

  // GitHub-style feed: commits grouped under a date header with a hash cell.
  await expect(page.getByText(/^Commits on /).first()).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/02-commits-feed.png`,
  });

  // Open the newest commit via its subject button.
  await commitRows
    .first()
    .getByRole("button", { name: /Add Trello board workflow details/ })
    .click();

  // Detail header: author line, subject, and hash.
  await expect(page.getByText("Commit from")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Add Trello board workflow details" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Copy commit hash" }),
  ).toBeVisible();

  // Diff from the mocked get_project_repo_diff renders changed files.
  await expect(page.getByText("2 changed files")).toBeVisible({
    timeout: 10_000,
  });
  await expect(
    page.getByText("WorkspaceTabs({ selectedCommitHash })"),
  ).toBeVisible();

  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/01-commit-detail.png`,
  });

  // Breadcrumb category segment steps back to the commits feed.
  await page
    .getByRole("navigation", { name: "Project breadcrumb" })
    .getByRole("button", { name: "Commits", exact: true })
    .click();
  await expect(commitRows.first()).toBeVisible();

  // The commits feed itself gets a grayed sub-tab crumb.
  await expect(
    page.getByRole("navigation", { name: "Project breadcrumb" }),
  ).toContainText("Commits");

  // The project-name segment goes to the project home (Overview tab).
  await commitRows
    .first()
    .getByRole("button", { name: /Add Trello board workflow details/ })
    .click();
  await expect(page.getByText("Commit from")).toBeVisible();
  await page
    .getByRole("navigation", { name: "Project breadcrumb" })
    .getByRole("button", { name: "buzz", exact: true })
    .click();
  await expect(page.getByRole("tab", { name: "Overview" })).toHaveAttribute(
    "aria-selected",
    "true",
  );

  // The Projects root segment leaves the project entirely.
  await page
    .getByRole("navigation", { name: "Project breadcrumb" })
    .getByRole("button", { name: "Projects", exact: true })
    .click();
  await expect(projectEntry).toBeVisible();
});

test("pull request and issue feeds share the commit row structure", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();

  // The overview no longer lists repository cards — switch to the
  // Repositories filter to reveal the project cards/rows.
  await page.getByRole("button", { name: "Repositories", exact: true }).click();

  const projectEntry = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  await expect(projectEntry).toBeVisible({ timeout: 10_000 });
  await projectEntry.click();

  // PR rows use the shared feed row: title button + #id cluster cell.
  await page.getByRole("tab", { name: "PRs" }).click();
  const prRows = page.getByTestId("project-pull-request-row");
  await expect(prRows.first()).toBeVisible({ timeout: 10_000 });
  await expect(
    prRows.first().getByRole("button", { name: /^#/ }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ fullPage: false, path: `${SHOTS}/03-prs-feed.png` });

  // The #id cell opens the PR detail, same as clicking the title.
  await prRows.first().getByRole("button", { name: /^#/ }).click();
  await expect(
    page.getByRole("navigation", { name: "Project breadcrumb" }),
  ).toContainText("PRs");

  // Step back to the feed so the workspace tabs are available again.
  await page
    .getByRole("navigation", { name: "Project breadcrumb" })
    .getByRole("button", { name: "PRs", exact: true })
    .click();
  await expect(prRows.first()).toBeVisible();

  // Issue rows share the same structure.
  await page.getByRole("tab", { name: "Issues" }).click();
  const issueRows = page.getByTestId("project-issue-row");
  await expect(issueRows.first()).toBeVisible({ timeout: 10_000 });
  await expect(
    issueRows.first().getByRole("button", { name: /^#/ }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/04-issues-feed.png`,
  });
});
