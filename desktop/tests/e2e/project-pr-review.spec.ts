import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/project-pr-review";
const RECOVERY_SHOTS = "test-results/project-pr-conflict-recovery";
const REVIEWER_AGENT_PUBKEY = "a".repeat(64);
const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);

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

  await page.getByRole("tab", { name: "Pull Request" }).click();
  const prRows = page.getByTestId("project-pull-request-row");
  await expect(prRows.first()).toBeVisible({ timeout: 10_000 });

  // Pick a PR authored by alice: the viewer is not the author, so the
  // Approve button must be available alongside the owner status controls.
  const aliceRow = prRows.filter({ hasText: "alice" }).first();
  await expect(aliceRow).toBeVisible();
  await aliceRow.getByRole("button", { name: /^#/ }).click();

  const header = page.getByRole("heading", { level: 3 });
  await expect(header.first()).toBeVisible();

  // Owner viewing an open PR: draft toggle and both review decisions are offered.
  const convertToDraft = page.getByRole("button", {
    name: "Convert to draft",
  });
  const approve = page.getByRole("button", { name: "Approve", exact: true });
  const requestChanges = page.getByRole("button", {
    name: "Request changes",
    exact: true,
  });
  await expect(convertToDraft).toBeVisible();
  await expect(approve).toBeVisible();
  await expect(requestChanges).toBeVisible();

  // Request a review from bob via the reviewers dropdown.
  await page.getByRole("button", { name: "Request", exact: true }).click();
  await page.getByTestId("project-reviewer-search").fill("bob");
  await page
    .getByTestId(`project-reviewer-result-${TEST_IDENTITIES.bob.pubkey}`)
    .evaluate((button) => {
      button.click();
      button.click();
    });
  await expect(page.getByText("Review requested.")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_SIGNED_EVENTS__?.filter(
            (event) =>
              event.kind === 1 &&
              event.tags.some(
                (tag) => tag[0] === "t" && tag[1] === "review-request",
              ),
          ).length ?? 0,
      ),
    )
    .toBe(1);
  // The requested reviewer appears in the reviewers row and the timeline.
  await expect(page.getByText("Requested a review from bob")).toBeVisible({
    timeout: 10_000,
  });

  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/01-review-requested.png`,
  });

  // Fire opposite decisions in the same event turn. The first choice wins;
  // the shared synchronous guard must prevent the approval from publishing.
  await requestChanges.evaluate((requestChangesButton) => {
    const approveButton = [...document.querySelectorAll("button")].find(
      (button) => button.textContent?.trim() === "Approve",
    );
    requestChangesButton.click();
    approveButton?.click();
  });
  await expect(page.getByText("Changes requested.")).toBeVisible();
  await expect(
    page.getByText("requested changes", { exact: true }),
  ).toBeVisible({
    timeout: 10_000,
  });
  await expect(requestChanges).toHaveCount(0);
  const changeRequestEvent = await page.evaluate(() =>
    window.__BUZZ_E2E_SIGNED_EVENTS__
      ?.filter(
        (event) =>
          event.kind === 1 &&
          event.tags.some(
            (tag) => tag[0] === "t" && tag[1] === "changes-requested",
          ),
      )
      .at(-1),
  );
  expect(changeRequestEvent?.tags).toContainEqual(["c", expect.any(String)]);
  const rapidDecisionEvents = await page.evaluate(
    () =>
      window.__BUZZ_E2E_SIGNED_EVENTS__?.filter(
        (event) =>
          event.kind === 1 &&
          event.tags.some(
            (tag) =>
              tag[0] === "t" &&
              (tag[1] === "approval" || tag[1] === "changes-requested"),
          ),
      ) ?? [],
  );
  expect(rapidDecisionEvents).toHaveLength(1);

  // Replace the completed change request with an approval. Both decisions
  // remain tied to the current commit and their timestamps preserve order.
  await approve.click();
  await expect(page.getByText("Pull request approved.")).toBeVisible();
  await expect(page.getByText("approved these changes")).toBeVisible({
    timeout: 10_000,
  });
  await expect(
    page.getByRole("button", { name: "Approve", exact: true }),
  ).toHaveCount(0);
  const approvalEvent = await page.evaluate(() =>
    window.__BUZZ_E2E_SIGNED_EVENTS__
      ?.filter(
        (event) =>
          event.kind === 1 &&
          event.tags.some((tag) => tag[0] === "t" && tag[1] === "approval"),
      )
      .at(-1),
  );
  expect(approvalEvent?.tags).toContainEqual(["c", expect.any(String)]);
  expect(approvalEvent?.createdAt).toBeGreaterThan(
    changeRequestEvent?.createdAt ?? 0,
  );

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
  const mergePayload = await page.evaluate(() =>
    window.__BUZZ_E2E_COMMAND_PAYLOADS__?.find(
      (entry) => entry.command === "merge_project_pull_request",
    ),
  );
  expect(mergePayload?.payload).toMatchObject({
    input: {
      expectedCommit: expect.any(String),
      sourceBranch: expect.any(String),
      targetBranch: "main",
      targetOwner: DEFAULT_MOCK_PUBKEY,
    },
  });
});

test("merge conflicts offer persistent terminal recovery", async ({ page }) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await openBuzzProject(page);
  await page.evaluate(() => {
    window.__BUZZ_E2E_PROJECT_MERGE_ERROR__ = {
      code: "merge_conflict",
      message: "Pull request has merge conflicts.",
      recovery: {
        action: "open_terminal",
        sourceBranch: "feature",
        targetBranch: "main",
      },
    };
  });

  await page.getByRole("tab", { name: "Pull Request" }).click();
  const aliceRow = page
    .getByTestId("project-pull-request-row")
    .filter({ hasText: "alice" })
    .first();
  await aliceRow.getByRole("button", { name: /^#/ }).click();
  await page.getByRole("button", { name: "Merge", exact: true }).click();
  await page.getByTestId("merge-pull-request-confirm-button").click();

  const recovery = page.getByTestId("merge-conflict-recovery");
  await expect(recovery).toBeVisible();
  await expect(
    recovery.getByRole("button", { name: "Copy commands" }),
  ).toBeDisabled();
  await waitForAnimations(page);
  await recovery.screenshot({
    path: `${RECOVERY_SHOTS}/01-merge-conflict.png`,
  });
  await recovery.getByRole("button", { name: "Resolve in Terminal" }).click();
  await expect(
    page.getByText("Recovery commit fetched and terminal opened."),
  ).toBeVisible();
  await expect(
    page.getByText("Recovery commit fetched and terminal opened."),
  ).toBeHidden({ timeout: 10_000 });
  await expect(recovery).toContainText("git switch 'main'");
  await expect(recovery).toContainText("git merge 'refs/buzz/merge-recovery/");
  await expect(
    recovery.getByRole("button", { name: "Copy commands" }),
  ).toBeEnabled();
  await waitForAnimations(page);
  await recovery.screenshot({
    path: `${RECOVERY_SHOTS}/02-merge-conflict-prepared.png`,
  });

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_COMMAND_PAYLOADS__?.find(
            (entry) => entry.command === "open_project_merge_recovery_terminal",
          ) ?? null,
      ),
    )
    .toMatchObject({
      command: "open_project_merge_recovery_terminal",
      payload: {
        input: {
          expectedCommit: expect.any(String),
          sourceBranch: "feature",
          targetBranch: "main",
        },
      },
    });
});

test("reviewer can leave a commit-scoped inline diff comment", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("tab", { name: "Pull Request" }).click();
  const aliceRow = page
    .getByTestId("project-pull-request-row")
    .filter({ hasText: "alice" })
    .first();
  await aliceRow.getByRole("button", { name: /^#/ }).click();
  await page.getByRole("tab", { name: /Files changed/ }).click();

  const diffLine = page
    .getByTestId("project-diff-line")
    .filter({ hasText: "function CommunityTabs({ selectedCommitHash })" });
  await expect(diffLine).toBeVisible({ timeout: 10_000 });
  await diffLine.hover();
  await diffLine.getByTestId("project-diff-add-comment").click();

  const composer = page.getByTestId("project-inline-comment-thread");
  await composer
    .locator("[contenteditable='true']")
    .fill("Please add a type for this parameter.");
  await composer.getByRole("button", { name: "Send message" }).click();
  await expect(page.getByText("Line comment posted.")).toBeVisible();

  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__BUZZ_E2E_SIGNED_EVENTS__?.find(
          (event) => event.content === "Please add a type for this parameter.",
        ),
      ),
    )
    .not.toBeUndefined();
  const inlineCommentEvent = await page.evaluate(() =>
    window.__BUZZ_E2E_SIGNED_EVENTS__?.find(
      (event) => event.content === "Please add a type for this parameter.",
    ),
  );
  expect(inlineCommentEvent?.tags).toContainEqual(["t", "inline-comment"]);
  expect(inlineCommentEvent?.tags).toContainEqual(["c", expect.any(String)]);
  expect(inlineCommentEvent?.tags).toContainEqual([
    "file",
    "desktop/src/features/projects/ui/ProjectDetailScreen.tsx",
  ]);
  expect(inlineCommentEvent?.tags).toContainEqual(["side", "new"]);
  expect(inlineCommentEvent?.tags).toContainEqual(["line", "3"]);
  await expect(page.getByTestId("project-inline-comment")).toContainText(
    "Please add a type for this parameter.",
  );

  await page.getByRole("tab", { name: "Conversation" }).click();
  await expect(
    page.getByText("Please add a type for this parameter."),
  ).toBeVisible();
  await expect(
    page.getByText("desktop/src/features/projects/ui/ProjectDetailScreen.tsx"),
  ).toBeVisible();
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

  await page.getByRole("tab", { name: "Pull Request" }).click();
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
      expectedCommit: expect.any(String),
      sourceBranch: expect.any(String),
      targetBranch: "main",
      targetOwner: TEST_IDENTITIES.alice.pubkey,
    },
  });
});

test("viewer without repository ownership cannot merge", async ({ page }) => {
  await enableProjectsFeature(page);
  await page.addInitScript((owner) => {
    window.__BUZZ_E2E_PROJECT_OWNER_OVERRIDE__ = owner;
  }, TEST_IDENTITIES.alice.pubkey);
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: REVIEWER_AGENT_PUBKEY,
        name: "Reviewer Bot",
      },
    ],
  });
  await openBuzzProject(page);

  await page.getByRole("tab", { name: "Pull Request" }).click();
  const prRow = page.getByTestId("project-pull-request-row").first();
  await expect(prRow).toBeVisible({ timeout: 10_000 });
  await prRow.getByRole("button", { name: /^#/ }).click();

  await expect(
    page.getByRole("button", { name: "Merge", exact: true }),
  ).toHaveCount(0);
  const mergeCommandCount = await page.evaluate(
    () =>
      window.__BUZZ_E2E_COMMANDS__?.filter(
        (command) => command === "merge_project_pull_request",
      ).length ?? 0,
  );
  expect(mergeCommandCount).toBe(0);

  const authorizationError = await page.evaluate(async (targetOwner) => {
    try {
      await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
        "merge_project_pull_request",
        {
          input: {
            expectedCommit: "1".repeat(40),
            pullRequestAuthor: "2".repeat(64),
            pullRequestId: "3".repeat(64),
            repoAddress: `30617:${targetOwner}:buzz`,
            sourceBranch: "feature/untrusted",
            statusCreatedAt: 1,
            targetBranch: "main",
            targetOwner,
          },
        },
      );
      return null;
    } catch (error) {
      return error instanceof Error ? error.message : String(error);
    }
  }, TEST_IDENTITIES.alice.pubkey);
  expect(authorizationError).toContain(
    "Only the repository owner or the owner of its managed agent",
  );
});

test("project pull requests preserve partial results from batched queries", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_QUERY_KINDS__ = [1619];
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page
    .getByRole("button", { name: "Pull Requests", exact: true })
    .click();

  await expect(
    page.getByRole("button", { name: /^View / }).first(),
  ).toBeVisible();
  await expect(
    page.getByText(/Some pull request details could not be loaded/),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();

  const workItemFilters = await page.evaluate(
    () =>
      window.__BUZZ_E2E_PROJECT_QUERY_FILTERS__?.filter(
        (filter) => filter.limit === 2_000,
      ) ?? [],
  );
  expect(
    workItemFilters
      .map((filter) => JSON.stringify([...(filter.kinds ?? [])].sort()))
      .sort(),
  ).toEqual(
    [[1], [1618, 1621], [1619], [1630, 1631, 1632, 1633]]
      .map((kinds) => JSON.stringify(kinds))
      .sort(),
  );
  expect(
    workItemFilters.every((filter) => (filter["#a"]?.length ?? 0) > 1),
  ).toBe(true);
  const expectedRepoAddresses = [
    `30617:${DEFAULT_MOCK_PUBKEY}:buzz`,
    `30617:${TEST_IDENTITIES.alice.pubkey}:relay-tools`,
    `30617:${TEST_IDENTITIES.bob.pubkey}:design-system`,
  ].sort();
  for (const filter of workItemFilters) {
    expect([...(filter["#a"] ?? [])].sort()).toEqual(expectedRepoAddresses);
  }

  await page.evaluate(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_QUERY_KINDS__ = [];
  });
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(
    page.getByText(/Some pull request details could not be loaded/),
  ).toHaveCount(0);
});

test("project pull requests report aggregate root query failures", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_QUERY_KINDS__ = [1618];
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page
    .getByRole("button", { name: "Pull Requests", exact: true })
    .click();

  await expect(page.getByText("Could not load pull requests.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
  await expect(page.getByText("No pull requests yet.")).toHaveCount(0);

  await page.evaluate(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_QUERY_KINDS__ = [];
  });
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(page.getByText("Could not load pull requests.")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: /^View / }).first(),
  ).toBeVisible();
});

test("project issues preserve partial results from aggregate queries", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_QUERY_KINDS__ = [1];
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByRole("button", { name: "Issues", exact: true }).click();

  await expect(
    page.getByRole("button", { name: /^View / }).first(),
  ).toBeVisible();
  await expect(
    page.getByText("Some issue details could not be loaded."),
  ).toBeVisible();
  await expect(page.getByText(/Missing comments\./)).toBeVisible();
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();

  await page.evaluate(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_QUERY_KINDS__ = [];
  });
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(
    page.getByText("Some issue details could not be loaded."),
  ).toHaveCount(0);
});

test("project overview reports aggregate work-item failures", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_QUERY_KINDS__ = [1618];
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();

  await expect(
    page.getByText("Could not load project activity."),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();

  await page.evaluate(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_QUERY_KINDS__ = [];
  });
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(page.getByText("Could not load project activity.")).toHaveCount(
    0,
  );
});

test("project without a checkout offers fetch feedback and dropdown cloning", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await openBuzzProject(page);

  await expect(
    page.getByRole("button", { name: "Remote", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Remote", exact: true }),
  ).toHaveClass(/\bborder-input\/40\b/);
  await expect(page.getByRole("button", { name: /main/ })).toHaveClass(
    /\bborder-input\/40\b/,
  );
  await expect(
    page.getByRole("button", { name: "Clone", exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Fetch", exact: true }).click();
  await expect(page.getByText("Remote state refreshed.")).toBeVisible();

  await page.getByRole("button", { name: "Remote", exact: true }).click();
  const cloneItem = page.getByRole("menuitem", {
    name: "Local missing Clone",
  });
  await expect(cloneItem.getByText("Local missing")).toHaveClass(
    /text-muted-foreground/,
  );
  await expect(cloneItem.getByText("Clone", { exact: true })).toHaveClass(
    /\bborder-input\/60\b/,
  );
  await cloneItem.click();
  await expect(page.getByText("Cloned repository.")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Local", exact: true }),
  ).toBeVisible();
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
  await page.getByRole("tab", { name: "Pull Request", exact: true }).click();
  await page.getByRole("button", { name: "Pull Request", exact: true }).click();
  await expect(page.getByTestId("create-pull-request-repository")).toHaveValue(
    /:buzz$/,
  );
  await expect(page.getByTestId("create-pull-request-base-branch")).toHaveValue(
    "main",
  );
  await expect(
    page.getByTestId("create-pull-request-compare-branch"),
  ).toHaveValue("feature/projects-workflow");
  await page
    .getByTestId("create-pull-request-title")
    .fill("Complete the Projects git workflow");
  await page
    .getByTestId("create-pull-request-body")
    .fill("Adds the missing desktop write path.");
  await page.getByTestId("create-pull-request-submit").evaluate((button) => {
    button.click();
    button.click();
  });
  await expect(page.getByText("Pull request created.")).toBeVisible();

  const createdEvents = await page.evaluate(
    () =>
      window.__BUZZ_E2E_SIGNED_EVENTS__?.filter(
        (event) => event.kind === 1618,
      ) ?? [],
  );
  expect(createdEvents).toHaveLength(1);
  const [createdEvent] = createdEvents;
  expect(createdEvent?.tags).toContainEqual([
    "branch-name",
    "feature/projects-workflow",
  ]);
  expect(createdEvent?.tags).toContainEqual(["target-branch", "main"]);
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
  await page.getByRole("button", { name: "Push", exact: true }).click();
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

test("project issue can be created from the issues header", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("tab", { name: "Issues", exact: true }).click();
  await page.getByRole("button", { name: "Issues", exact: true }).click();
  await page
    .getByTestId("create-issue-title")
    .fill("Document the broken workflow");
  await page
    .getByTestId("create-issue-body")
    .fill("The project workflow needs a clear repair path.");
  await page.getByTestId("create-issue-submit").click();
  await expect(page.getByText("Issue created.")).toBeVisible();

  const createdEvent = await page.evaluate(() =>
    window.__BUZZ_E2E_SIGNED_EVENTS__?.find((event) => event.kind === 1621),
  );
  expect(createdEvent?.tags).toContainEqual([
    "subject",
    "Document the broken workflow",
  ]);
});
