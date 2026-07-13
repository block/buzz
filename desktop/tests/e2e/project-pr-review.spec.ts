import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/project-pr-review";

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

test("PR creator/owner can toggle draft, request reviews, and approve", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();

  // The "buzz" mock project is owned by the viewer, so status changes and
  // review requests are always permitted regardless of who authored the PR.
  const projectEntry = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  await expect(projectEntry).toBeVisible({ timeout: 10_000 });
  await projectEntry.click();

  await page.getByRole("tab", { name: "PRs" }).click();
  const prRows = page.getByTestId("project-pull-request-row");
  await expect(prRows.first()).toBeVisible({ timeout: 10_000 });

  // Pick a PR authored by alice: the viewer is not the author, so the
  // Approve button must be available alongside the owner status controls.
  const aliceRow = prRows.filter({ hasText: "alice" }).first();
  await expect(aliceRow).toBeVisible();
  await aliceRow.getByRole("button", { name: /^#/ }).click();

  const header = page.getByRole("heading", { level: 3 });
  await expect(header.first()).toBeVisible();

  // Owner viewing an open PR: draft toggle + approve are both offered.
  const convertToDraft = page.getByRole("button", {
    name: "Convert to draft",
  });
  const approve = page.getByRole("button", { name: "Approve", exact: true });
  await expect(convertToDraft).toBeVisible();
  await expect(approve).toBeVisible();

  // Request a review from bob via the reviewers dropdown.
  await page.getByRole("button", { name: "Request" }).click();
  await page.getByRole("menuitem", { name: "bob" }).click();
  await expect(page.getByText("Review requested.")).toBeVisible();
  // The requested reviewer appears in the reviewers row and the timeline.
  await expect(page.getByText("Requested a review from bob")).toBeVisible({
    timeout: 10_000,
  });

  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/01-review-requested.png`,
  });

  // Approve the PR: header flips to the approved chip and the discussion
  // gains a compact approval timeline row.
  await approve.click();
  await expect(page.getByText("Pull request approved.")).toBeVisible();
  await expect(page.getByText("approved these changes")).toBeVisible({
    timeout: 10_000,
  });
  await expect(
    page.getByRole("button", { name: "Approve", exact: true }),
  ).toHaveCount(0);

  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/02-approved.png`,
  });

  // Convert to draft: badge flips to Draft and the ready button appears.
  await convertToDraft.click();
  await expect(page.getByText("Converted to draft.")).toBeVisible();
  const readyForReview = page.getByRole("button", {
    name: "Ready for review",
  });
  await expect(readyForReview).toBeVisible({ timeout: 10_000 });
  await expect(convertToDraft).toHaveCount(0);

  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/03-draft.png`,
  });

  // And back: Ready for review restores the Open state.
  await readyForReview.click();
  await expect(page.getByText("Marked as ready for review.")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Convert to draft" }),
  ).toBeVisible({ timeout: 10_000 });
});
