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
  await page.getByRole("button", { name: "Commit", exact: true }).click();
  await expect(commitRows.first()).toBeVisible();

  // The back arrow also steps back one level (detail → project page),
  // not all the way to the projects overview.
  await commitRows
    .first()
    .getByRole("button", { name: /Add Trello board workflow details/ })
    .click();
  await expect(page.getByText("Commit from")).toBeVisible();
  await page.getByRole("button", { name: "Back to buzz" }).click();
  await expect(commitRows.first()).toBeVisible();

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
