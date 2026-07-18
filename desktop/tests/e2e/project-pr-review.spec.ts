import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/project-pr-review";
const REVIEWER_AGENT_PUBKEY = "a".repeat(64);

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

async function openBuzzProject(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByRole("button", { name: "Repositories", exact: true }).click();
  const projectEntry = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  await expect(projectEntry).toBeVisible({ timeout: 10_000 });
  await projectEntry.click();
}

test("PR creator/owner can toggle draft, request reviews, and approve", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_EVENT_KINDS__ = [1631];
  });
  await installMockBridge(page);
  await openBuzzProject(page);

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
  await page.getByTestId("project-reviewer-search").fill("bob");
  await page
    .getByTestId(`project-reviewer-result-${TEST_IDENTITIES.bob.pubkey}`)
    .click();
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

  await page.getByRole("button", { name: "Merge", exact: true }).click();
  await expect(page.getByTestId("merge-pull-request-confirm")).toBeVisible();
  await page.getByTestId("merge-pull-request-confirm-button").click();
  await expect(page.getByText("Merged feature into main.")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_SIGNED_EVENTS__?.filter(
            (event) => event.kind === 1631,
          ).length ?? 0,
      ),
    )
    .toBe(1);
  await expect(
    page.getByRole("button", {
      name: "Publish merged status",
      exact: true,
    }),
  ).toBeVisible();
  await page
    .getByRole("button", {
      name: "Publish merged status",
      exact: true,
    })
    .click();
  await expect(
    page.getByText("Published merged pull request status."),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_SIGNED_EVENTS__?.filter(
            (event) => event.kind === 1631,
          ).length ?? 0,
      ),
    )
    .toBe(1);
  const mergedEvent = await page.evaluate(() =>
    window.__BUZZ_E2E_SIGNED_EVENTS__
      ?.filter((event) => event.kind === 1631)
      .at(-1),
  );
  expect(mergedEvent?.tags).toContainEqual([
    "merge-commit",
    "abcdef0123456789abcdef0123456789abcdef01",
  ]);
  expect(mergedEvent?.tags.some((tag) => tag[0] === "e")).toBe(true);
  const mergeCommandCount = await page.evaluate(
    () =>
      window.__BUZZ_E2E_COMMANDS__?.filter(
        (command) => command === "merge_project_pull_request",
      ).length ?? 0,
  );
  expect(mergeCommandCount).toBe(1);
});

test("managed agent repository owner can merge", async ({ page }) => {
  await enableProjectsFeature(page);
  await page.addInitScript((owner) => {
    window.__BUZZ_E2E_PROJECT_OWNER_OVERRIDE__ = owner;
  }, TEST_IDENTITIES.alice.pubkey);
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.alice.pubkey,
        name: "Brain",
      },
      {
        pubkey: REVIEWER_AGENT_PUBKEY,
        name: "Reviewer Bot",
      },
    ],
  });
  await openBuzzProject(page);

  await page.getByRole("tab", { name: "PRs" }).click();
  const agentRow = page
    .getByTestId("project-pull-request-row")
    .filter({ hasText: "Brain" })
    .first();
  await expect(agentRow).toBeVisible({ timeout: 10_000 });
  await agentRow.getByRole("button", { name: /^#/ }).click();
  await page.getByRole("button", { name: "Request", exact: true }).click();
  await page.getByTestId("project-reviewer-search").fill("Reviewer Bot");
  await page
    .getByTestId(`project-reviewer-result-${REVIEWER_AGENT_PUBKEY}`)
    .click();
  await expect(page.getByText("Review requested.")).toBeVisible();
  const reviewRequestPayload = await page.evaluate(() =>
    window.__BUZZ_E2E_COMMAND_PAYLOADS__?.find(
      (entry) => entry.command === "sign_project_pull_request_review_request",
    ),
  );
  expect(reviewRequestPayload?.payload).toMatchObject({
    input: {
      reviewers: [REVIEWER_AGENT_PUBKEY],
      targetOwner: TEST_IDENTITIES.alice.pubkey,
    },
  });
  await page.getByRole("button", { name: "Merge", exact: true }).click();
  await page.getByTestId("merge-pull-request-confirm-button").click();
  await expect(page.getByText("Merged feature into main.")).toBeVisible();

  const mergePayload = await page.evaluate(() =>
    window.__BUZZ_E2E_COMMAND_PAYLOADS__?.find(
      (entry) => entry.command === "merge_project_pull_request",
    ),
  );
  expect(mergePayload?.payload).toMatchObject({
    input: {
      targetOwner: TEST_IDENTITIES.alice.pubkey,
    },
  });
});

test("project without a checkout can be cloned", async ({ page }) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("button", { name: "Clone", exact: true }).click();
  await expect(page.getByText("Cloned repository.")).toBeVisible();
  const commands = await page.evaluate(
    () => window.__BUZZ_E2E_COMMANDS__ ?? [],
  );
  expect(commands).toContain("clone_project_repository");
});

test("pushed local branch can open a pull request", async ({ page }) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    const commit = "1234567890abcdef1234567890abcdef12345678";
    window.__BUZZ_E2E_PROJECT_REPO_SYNC_STATUS__ = {
      local_path: "/tmp/buzz/REPOS/buzz",
      local_branch: "feature/projects-workflow",
      local_head: commit,
      local_short_head: commit.slice(0, 7),
      remote_branch: "feature/projects-workflow",
      remote_head: commit,
      remote_short_head: commit.slice(0, 7),
      merge_base: "0123456789abcdef0123456789abcdef01234567",
      ahead_count: 0,
      behind_count: 0,
      has_uncommitted_changes: false,
      has_untracked_files: false,
      can_push: false,
      push_block_reason: "Local branch is already pushed.",
      can_pull: false,
      pull_block_reason: "Local branch is up to date.",
    };
    window.__BUZZ_E2E_REJECT_PROJECT_EVENT_KINDS__ = [1619];
  });
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("button", { name: /main/ }).click();
  await page
    .getByRole("menuitemradio", { name: "feature/projects-workflow" })
    .click();
  await page
    .getByRole("button", { name: "New pull request", exact: true })
    .click();
  await page
    .getByTestId("create-pull-request-title")
    .fill("Complete the Projects git workflow");
  await page
    .getByTestId("create-pull-request-body")
    .fill("Adds the missing desktop write path.");
  await page.getByTestId("create-pull-request-submit").click();
  await expect(page.getByText("Pull request created.")).toBeVisible();

  const createdEvent = await page.evaluate(() =>
    window.__BUZZ_E2E_SIGNED_EVENTS__?.find((event) => event.kind === 1618),
  );
  expect(createdEvent?.tags).toContainEqual([
    "branch-name",
    "feature/projects-workflow",
  ]);
  expect(createdEvent?.tags).toContainEqual([
    "subject",
    "Complete the Projects git workflow",
  ]);

  await page.getByRole("tab", { name: "Overview" }).click();
  await page.evaluate(async () => {
    const status = window.__BUZZ_E2E_PROJECT_REPO_SYNC_STATUS__;
    if (!status) throw new Error("Missing mocked repository status.");
    status.local_head = "abcdef0123456789abcdef0123456789abcdef01";
    status.local_short_head = status.local_head.slice(0, 7);
    status.ahead_count = 1;
    status.can_push = true;
    status.push_block_reason = null;
    await window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
      queryKey: ["project"],
    });
  });
  await page.getByRole("button", { name: "Push 1", exact: true }).click();
  await expect(page.getByText("mock project event rejection")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_SIGNED_EVENTS__?.filter(
            (event) => event.kind === 1619,
          ).length ?? 0,
      ),
    )
    .toBe(1);
  await expect(
    page.getByRole("button", { name: "Update PR", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Update PR", exact: true }).click();
  await expect(page.getByText(/Pull request updated/)).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_SIGNED_EVENTS__?.filter(
            (event) => event.kind === 1619,
          ).length ?? 0,
      ),
    )
    .toBe(2);
  await expect(
    page.getByRole("button", { name: "Update PR", exact: true }),
  ).toHaveCount(0);

  const updateEvent = await page.evaluate(() =>
    window.__BUZZ_E2E_SIGNED_EVENTS__
      ?.filter((event) => event.kind === 1619)
      .at(-1),
  );
  expect(updateEvent?.tags).toContainEqual([
    "c",
    "abcdef0123456789abcdef0123456789abcdef01",
  ]);
  expect(updateEvent?.tags.some((tag) => tag[0] === "E")).toBe(true);
});
