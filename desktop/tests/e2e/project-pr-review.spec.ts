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
  await page.getByTestId("projects-section-projects").click();
  const projectEntry = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  await expect(projectEntry).toBeVisible({ timeout: 10_000 });
  await projectEntry.click();
}

async function addProjectToSidebar(
  page: import("@playwright/test").Page,
  dtag: string,
) {
  await page.getByTestId("sidebar-projects-section-label").hover();
  await page.getByTestId("sidebar-projects-create").click();
  const browser = page.getByTestId("project-browser-dialog");
  await browser.getByRole("searchbox", { name: "Search projects" }).fill(dtag);
  await browser.getByTestId(`project-browser-result-${dtag}`).click();
}

function pullRequestRowByAuthor(
  page: import("@playwright/test").Page,
  author: string,
) {
  return page.getByTestId("project-pull-request-row").filter({
    has: page.getByRole("button", { name: author, exact: true }),
  });
}

async function expectLocalRepositoryOpenAction(
  page: import("@playwright/test").Page,
) {
  const openButton = page.getByRole("button", { name: "Open", exact: true });
  await expect(openButton).toHaveAttribute(
    "title",
    "Open local repository folder",
  );
  await expect(
    page.getByRole("link", { name: "Open", exact: true }),
  ).toHaveCount(0);
}

test("same-second request changes supersedes approval", async ({ page }) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    Date.now = () => 1_900_000_000_000;
  });
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("tab", { name: "Review" }).click();
  const aliceRow = pullRequestRowByAuthor(page, "alice").first();
  await expect(aliceRow).toBeVisible({ timeout: 10_000 });
  await aliceRow.getByRole("button", { name: /^#/ }).click();

  await page.getByRole("button", { name: "Approve", exact: true }).click();
  const approveDialog = page.getByRole("dialog", {
    name: "Approve review",
  });
  await approveDialog
    .getByRole("textbox", { name: "Approval summary" })
    .fill("Approved at the fixed second.");
  await approveDialog
    .getByRole("button", { name: "Approve", exact: true })
    .click();
  await expect(page.getByText("Review approved.")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Approve", exact: true }),
  ).toHaveCount(0);

  const commentComposer = page.getByTestId(
    "project-pull-request-comment-composer",
  );
  await commentComposer
    .getByRole("button", { name: "Comment", exact: true })
    .click();
  await page.getByRole("menuitemradio", { name: "Request changes" }).click();
  await commentComposer
    .locator('[contenteditable="true"]')
    .fill("Changes requested at the same fixed second.");
  await commentComposer.getByRole("button", { name: "Send message" }).click();
  await expect(page.getByText("Changes requested.")).toBeVisible();

  const [approvalEvent, changeRequestEvent] = await page.evaluate(() => {
    const decisions =
      window.__BUZZ_E2E_SIGNED_EVENTS__?.filter(
        (event) =>
          event.kind === 1 &&
          event.tags.some(
            (tag) =>
              tag[0] === "t" &&
              (tag[1] === "approval" || tag[1] === "changes-requested"),
          ),
      ) ?? [];
    return [decisions.at(-2), decisions.at(-1)];
  });
  expect(approvalEvent?.tags).toContainEqual(["t", "approval"]);
  expect(changeRequestEvent?.tags).toContainEqual(["t", "changes-requested"]);
  expect(changeRequestEvent?.createdAt).toBeGreaterThan(
    approvalEvent?.createdAt ?? 0,
  );
});

test("PR creator/owner can toggle draft, request reviews, and approve", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_EVENT_KINDS__ = [1631];
  });
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("tab", { name: "Review" }).click();
  const prRows = page.getByTestId("project-pull-request-row");
  await expect(prRows.first()).toBeVisible({ timeout: 10_000 });

  // Pick a PR authored by alice: the viewer is not the author, so the
  // Approve button must be available alongside the owner status controls.
  const aliceRow = prRows
    .filter({
      has: page.getByRole("button", { name: "alice", exact: true }),
    })
    .first();
  await expect(aliceRow).toBeVisible();
  await aliceRow.getByRole("button", { name: /^#/ }).click();

  const header = page.getByRole("heading", { level: 3 });
  await expect(header.first()).toBeVisible();
  await expect(page.getByTestId("project-review-summary")).toBeVisible();
  const sourceChannelLink = page.getByRole("button", {
    name: "Open author-claimed origin channel #general",
    exact: true,
  });
  await expect(sourceChannelLink).toBeVisible();

  // Owner viewing an open PR: draft toggle and both review decisions are offered.
  const morePullRequestActions = page.getByRole("button", {
    name: "More review actions",
  });
  const approve = page.getByRole("button", { name: "Approve", exact: true });
  const commentComposer = page.getByTestId(
    "project-pull-request-comment-composer",
  );
  const reviewMode = commentComposer.getByRole("button", {
    name: "Comment",
    exact: true,
  });
  await expect(morePullRequestActions).toBeVisible();
  await expect(approve).toBeVisible();
  await expect(reviewMode).toBeVisible();

  // Request a review from bob via the centered reviewer dialog.
  await page.getByRole("button", { name: "Add Reviewer", exact: true }).click();
  await expect(
    page.getByRole("dialog").getByRole("heading", { name: "Add reviewer" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/00-add-reviewer-dialog.png`,
  });
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
  const reviewHistoryToggle = page.getByTestId(
    "project-pull-request-review-history-toggle",
  );
  await expect(reviewHistoryToggle).toHaveAttribute("aria-expanded", "true");
  await expect(
    page.getByTestId("project-pull-request-timeline-row"),
  ).toHaveCount(1);
  // The requested reviewer appears in the reviewers row and default timeline.
  await expect(page.getByText("Requested a review from bob")).toBeVisible({
    timeout: 10_000,
  });

  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/01-review-requested.png`,
  });

  await reviewMode.click();
  await page.getByRole("menuitemradio", { name: "Request changes" }).click();
  await commentComposer
    .locator('[contenteditable="true"]')
    .fill("Please handle the empty state before merging.");
  await commentComposer.getByRole("button", { name: "Send message" }).click();
  await expect(page.getByText("Changes requested.")).toBeVisible();
  await expect(reviewMode).toHaveText("Comment");
  await expect(
    page.getByText("Please handle the empty state before merging."),
  ).toBeVisible();
  await expect(
    page
      .getByTestId("project-pull-request-timeline-row")
      .filter({ hasText: "requested changes" }),
  ).toBeVisible({ timeout: 10_000 });
  await expect(page.getByTestId("project-review-summary")).toHaveText(
    "Changes requested",
  );
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
  expect(changeRequestEvent?.content).toBe(
    "Please handle the empty state before merging.",
  );
  expect(changeRequestEvent?.tags).toContainEqual(["c", expect.any(String)]);
  const reviewDecisionEvents = await page.evaluate(
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
  expect(reviewDecisionEvents).toHaveLength(1);

  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/05-changes-requested.png`,
  });
  const changeRequestRow = page
    .getByTestId("project-pull-request-timeline-row")
    .filter({ hasText: "requested changes" })
    .first();
  await changeRequestRow.getByRole("button").first().hover();
  await expect(page.getByTestId("user-profile-popover")).toBeVisible();
  await page.mouse.move(0, 0);
  await expect(page.getByTestId("user-profile-popover")).toBeHidden();

  await expect(reviewHistoryToggle).toContainText("Collapse review history");
  const expandedReviewRows = page.getByTestId(
    "project-pull-request-timeline-row",
  );
  await expect(expandedReviewRows).toHaveCount(2);
  await expect(expandedReviewRows.nth(0)).toContainText(
    "Requested a review from bob",
  );
  await expect(expandedReviewRows.nth(1)).toContainText("requested changes");
  await expect(approve).toBeVisible();
  await reviewHistoryToggle.click();
  await expect(reviewHistoryToggle).toContainText("Show 2 earlier activities");
  await expect(
    page.getByTestId("project-pull-request-timeline-row"),
  ).toHaveCount(0);
  await expect(changeRequestRow).toBeHidden();

  // Replace the completed change request with an approval. Both decisions
  // remain tied to the current commit and their timestamps preserve order.
  await approve.click();
  const approveDialog = page.getByRole("dialog", {
    name: "Approve review",
  });
  await approveDialog
    .getByRole("textbox", { name: "Approval summary" })
    .fill("Ready to merge.");
  await approveDialog
    .getByRole("button", { name: "Approve", exact: true })
    .click();
  await expect(page.getByText("Review approved.")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Approve", exact: true }),
  ).toHaveCount(0);
  await expect(page.getByTestId("project-review-summary")).toHaveText(
    "Awaiting review",
  );
  await expect(page.getByText("Approved", { exact: true })).toBeVisible();
  const approvalEvent = await page.evaluate(() =>
    window.__BUZZ_E2E_SIGNED_EVENTS__
      ?.filter(
        (event) =>
          event.kind === 1 &&
          event.tags.some((tag) => tag[0] === "t" && tag[1] === "approval"),
      )
      .at(-1),
  );
  expect(approvalEvent?.content).toBe("Ready to merge.");
  expect(approvalEvent?.tags).toContainEqual(["c", expect.any(String)]);
  expect(approvalEvent?.createdAt).toBeGreaterThan(
    changeRequestEvent?.createdAt ?? 0,
  );
  await reviewHistoryToggle.click();
  await expect(page.getByText("Ready to merge.")).toBeVisible({
    timeout: 10_000,
  });
  await expect(
    page.getByTestId("project-pull-request-timeline-row"),
  ).toHaveCount(3);

  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/02-approved.png`,
  });

  // Histories over three entries show only the latest three until explicitly
  // expanded. Collapsing the whole timeline preserves that inner choice.
  await commentComposer
    .locator('[contenteditable="true"]')
    .fill("Remember the expanded history state.");
  await commentComposer.getByRole("button", { name: "Send message" }).click();
  await expect(page.getByText("Comment posted.")).toBeVisible();
  const timelineRows = page.getByTestId("project-pull-request-timeline-row");
  const earlierActivities = page.getByTestId(
    "project-pull-request-earlier-activities",
  );
  await expect(timelineRows).toHaveCount(3);
  await expect(earlierActivities).toContainText("Show 1 earlier activity");

  await reviewHistoryToggle.click();
  await expect(timelineRows).toHaveCount(0);
  await reviewHistoryToggle.click();
  await expect(timelineRows).toHaveCount(3);
  await expect(earlierActivities).toBeVisible();

  await earlierActivities.click();
  await expect(timelineRows).toHaveCount(4);
  await reviewHistoryToggle.click();
  await expect(timelineRows).toHaveCount(0);
  await reviewHistoryToggle.click();
  await expect(timelineRows).toHaveCount(4);
  await expect(earlierActivities).toHaveCount(0);

  // Convert to draft: badge flips to Draft and the ready button appears.
  await morePullRequestActions.click();
  await page.getByRole("menuitem", { name: "Convert to draft" }).click();
  await expect(page.getByText("Converted to draft.")).toBeVisible();
  const readyForReview = page.getByRole("button", {
    name: "Ready for review",
  });
  await expect(readyForReview).toBeVisible({ timeout: 10_000 });
  await expect(morePullRequestActions).toBeVisible();

  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/03-draft.png`,
  });

  // And back: Ready for review restores the Open state.
  await readyForReview.click();
  await expect(page.getByText("Marked as ready for review.")).toBeVisible();
  await expect(morePullRequestActions).toBeVisible({ timeout: 10_000 });

  // Closing is reversible, unlike merging: a closed PR can be reopened.
  await morePullRequestActions.click();
  const closePullRequest = page.getByRole("menuitem", {
    name: "Close review",
  });
  await closePullRequest.click();
  await expect(page.getByText("Review closed.")).toBeVisible();
  const reopenPullRequest = page.getByRole("button", {
    name: "Reopen review",
  });
  await expect(reopenPullRequest).toBeVisible({ timeout: 10_000 });
  await expect(closePullRequest).toHaveCount(0);

  await reopenPullRequest.click();
  await expect(page.getByText("Review reopened.")).toBeVisible();
  await expect(morePullRequestActions).toBeVisible({ timeout: 10_000 });

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
  await expect(page.getByText("Published merged review status.")).toBeVisible();
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

  await sourceChannelLink.click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
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

  await page.getByRole("tab", { name: "Review" }).click();
  const aliceRow = pullRequestRowByAuthor(page, "alice").first();
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

  await page.getByRole("tab", { name: "Review" }).click();
  const aliceRow = pullRequestRowByAuthor(page, "alice").first();
  await aliceRow.getByRole("button", { name: /^#/ }).click();
  await page.getByRole("button", { name: /^Files changed/ }).click();

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
  await composer.getByRole("button", { name: "Comment", exact: true }).click();
  await page.getByRole("menuitemradio", { name: "Request changes" }).click();
  await composer.getByRole("button", { name: "Send message" }).click();
  await expect(page.getByText("Changes requested.")).toBeVisible();

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
  expect(inlineCommentEvent?.tags).toContainEqual(["t", "changes-requested"]);
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

  const timelineComment = page
    .getByTestId("project-pull-request-timeline-row")
    .filter({ hasText: "Please add a type for this parameter." });
  await expect(timelineComment).toContainText(
    "desktop/src/features/projects/ui/ProjectDetailScreen.tsx",
  );
  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/04-inline-comment.png`,
  });

  await page
    .getByRole("button", {
      name: "Open desktop/src/features/projects/ui/ProjectDetailScreen.tsx new line 3 in Files changed",
    })
    .click();
  await expect(
    page.getByRole("button", { name: /^Files changed/ }),
  ).toHaveAttribute("aria-expanded", "true");
  const focusedLine = page.getByTestId("project-diff-focused-line");
  await expect(focusedLine).toBeVisible();
  await expect(focusedLine).toHaveAttribute(
    "data-path",
    "desktop/src/features/projects/ui/ProjectDetailScreen.tsx",
  );
  await expect(focusedLine).toHaveAttribute("data-side", "new");
  await expect(focusedLine).toHaveAttribute("data-line", "3");
  await focusedLine.click();
  await expect(page.getByTestId("project-diff-focused-line")).toHaveCount(0);
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

  await page.getByRole("tab", { name: "Review" }).click();
  const agentRow = pullRequestRowByAuthor(page, "Brain").first();
  await expect(agentRow).toBeVisible({ timeout: 10_000 });
  await agentRow.getByRole("button", { name: /^#/ }).click();
  await page.getByRole("button", { name: "Add Reviewer", exact: true }).click();
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
  await page.getByRole("button", { name: "More review actions" }).click();
  const closePullRequest = page.getByRole("menuitem", {
    name: "Close review",
  });
  await expect(closePullRequest).toBeVisible();
  await closePullRequest.click();
  await expect(page.getByText("Review closed.")).toBeVisible();
  await page.getByRole("button", { name: "Reopen review" }).click();
  await expect(page.getByText("Review reopened.")).toBeVisible();
  const statusPayloads = await page.evaluate(() =>
    window.__BUZZ_E2E_COMMAND_PAYLOADS__?.filter(
      (entry) => entry.command === "sign_project_pull_request_status",
    ),
  );
  expect(statusPayloads).toHaveLength(2);
  expect(statusPayloads?.map((entry) => entry.payload)).toEqual([
    expect.objectContaining({
      input: expect.objectContaining({
        status: "closed",
        targetOwner: TEST_IDENTITIES.alice.pubkey,
      }),
    }),
    expect.objectContaining({
      input: expect.objectContaining({
        status: "open",
        targetOwner: TEST_IDENTITIES.alice.pubkey,
      }),
    }),
  ]);
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

  await page.getByRole("tab", { name: "Review" }).click();
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
  await page.getByRole("button", { name: "Reviews", exact: true }).click();

  await expect(
    page.getByRole("button", { name: /^View / }).first(),
  ).toBeVisible();
  await expect(
    page.getByText(/Some review details could not be loaded/),
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
    page.getByText(/Some review details could not be loaded/),
  ).toHaveCount(0);
});

test("project pull request author rollover stays identity-only", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByRole("button", { name: "Reviews", exact: true }).click();
  await page.getByRole("button", { name: "List layout" }).click();

  const row = page.locator('[data-testid^="projects-pr-row-"]').first();
  const author = row.getByTestId("projects-pr-author");
  await expect(author).toBeVisible();
  await expect(
    author.locator(
      '[data-testid="projects-pr-author-avatar-image"], [data-testid="projects-pr-author-avatar-fallback"]',
    ),
  ).toBeVisible();

  const authorLabel =
    (
      await author.getByTestId("projects-pr-author-label").textContent()
    )?.trim() ?? "";
  expect(authorLabel.length).toBeGreaterThan(0);
  await author.hover();
  const rollover = page.getByTestId("projects-pr-author-rollover");
  await expect(rollover).toBeVisible();
  await expect(rollover).toContainText(authorLabel);
  await expect(rollover).toContainText(/Agent|Person/);
  await expect(rollover).not.toContainText("Created");
  await expect(page.getByTestId("user-profile-popover")).toHaveCount(0);
});

test("project issue author rollover matches pull requests", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByRole("button", { name: "Tasks", exact: true }).click();
  await page.getByRole("button", { name: "List layout" }).click();

  const row = page.locator('[data-testid^="projects-issue-row-"]').first();
  const author = row.getByTestId("projects-issue-author");
  await expect(author).toBeVisible();
  await expect(
    author.locator(
      '[data-testid="projects-issue-author-avatar-image"], [data-testid="projects-issue-author-avatar-fallback"]',
    ),
  ).toBeVisible();

  const authorLabel =
    (
      await author.getByTestId("projects-issue-author-label").textContent()
    )?.trim() ?? "";
  expect(authorLabel.length).toBeGreaterThan(0);
  await author.hover();
  const rollover = page.getByTestId("projects-issue-author-rollover");
  await expect(rollover).toBeVisible();
  await expect(rollover).toContainText(authorLabel);
  await expect(rollover).toContainText(/Agent|Person/);
  await expect(rollover).not.toContainText("Created");
  await expect(page.getByTestId("user-profile-popover")).toHaveCount(0);
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
  await page.getByRole("button", { name: "Reviews", exact: true }).click();

  await expect(page.getByText("Could not load reviews.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
  await expect(page.getByText("No reviews yet.")).toHaveCount(0);

  await page.evaluate(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_QUERY_KINDS__ = [];
  });
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(page.getByText("Could not load reviews.")).toHaveCount(0);
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
  await page.getByRole("button", { name: "Tasks", exact: true }).click();

  await expect(
    page.getByRole("button", { name: /^View / }).first(),
  ).toBeVisible();
  await expect(
    page.getByText("Some task details could not be loaded."),
  ).toBeVisible();
  // Rejecting kind 1 fails both the comment window and the exhaustive
  // assignment-operation query, so both sections are reported missing.
  await expect(page.getByText(/Missing assignments, comments\./)).toBeVisible();
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();

  await page.evaluate(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_QUERY_KINDS__ = [];
  });
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(
    page.getByText("Some task details could not be loaded."),
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

test("project overview presents collapsible context beside grouped activity", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();

  await expect(page.getByTestId("projects-overview-panel")).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await expect(page.getByTestId("projects-overview-layout")).toHaveAttribute(
    "data-project-context-detached",
    "true",
  );
  const overviewLayout = page.getByTestId("projects-overview-layout");
  const overviewContentPod = page.getByTestId("projects-overview-content-pod");
  const appContentSurface = page
    .locator("[data-buzz-content-surface]")
    .filter({ has: overviewLayout })
    .first();
  await expect(appContentSurface).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await expect(appContentSurface).toHaveCSS("border-radius", "0px");
  await expect(overviewContentPod).toHaveCSS("border-radius", "16px");
  await expect(page.getByTestId("projects-workspace-chrome")).toContainText(
    "Projects",
  );
  await expect(page.getByTestId("projects-workspace-chrome")).toContainText(
    "Activity",
  );
  await expect(page.getByTestId("projects-page-tabs")).toBeVisible();
  await expect(page.getByTestId("projects-page-header")).toContainText(
    "Welcome to Activity",
  );
  await expect(page.getByTestId("projects-activity-search")).toBeVisible();
  await expect(page.getByTestId("projects-activity-intro")).toContainText(
    "Keep up with commits, reviews, and tasks",
  );
  await expect(
    page.getByTestId("projects-overview-context-panel"),
  ).toBeVisible();
  await expect(page.getByTestId("projects-overview-context-title")).toHaveText(
    "Projects",
  );
  expect(
    Math.round(
      (await page.getByTestId("projects-overview-context-panel").boundingBox())
        ?.width ?? 0,
    ),
  ).toBe(280);
  await expect(
    page.getByTestId("projects-overview-create-project"),
  ).toContainText("Create project");
  await expect(
    page
      .getByTestId("projects-overview-context-panel")
      .getByTestId("projects-create-menu"),
  ).toBeVisible();
  await expect(page.getByTestId("projects-overview-stats-pod")).toBeVisible();
  await expect(
    page
      .getByTestId("projects-overview-context-panel")
      .getByTestId("projects-overview-people"),
  ).toBeVisible();
  await expect(
    page
      .getByTestId("projects-overview-stats-pod")
      .getByTestId("projects-overview-people"),
  ).toHaveCount(0);
  await expect(
    page
      .getByTestId("projects-overview-context-panel")
      .getByTestId("projects-overview-context-title"),
  ).toBeVisible();
  await expect(
    page
      .getByTestId("projects-overview-context-panel")
      .getByRole("heading", { name: "Details" }),
  ).toHaveCount(0);
  await expect(
    page
      .getByTestId("projects-overview-context-panel")
      .getByRole("heading", { name: "Activity" }),
  ).toHaveCount(0);
  await expect(
    page
      .getByTestId("projects-overview-context-panel")
      .getByTestId("projects-overview-activity"),
  ).toBeVisible();
  await expect(
    page.getByTestId("projects-activity-group").first(),
  ).toContainText(/This week|Last week|Earlier/);
  const searchBox = await page
    .getByTestId("projects-activity-search")
    .boundingBox();
  const tabMenuBox = await page.getByTestId("projects-page-tabs").boundingBox();
  const firstTabBox = await page
    .getByTestId("projects-section-all")
    .boundingBox();
  const introBox = await page
    .getByTestId("projects-activity-intro")
    .boundingBox();
  const feedBox = await page
    .getByTestId("projects-activity-group")
    .first()
    .boundingBox();
  const contentPodBox = await overviewContentPod.boundingBox();
  expect(searchBox).toBeTruthy();
  expect(tabMenuBox).toBeTruthy();
  expect(firstTabBox).toBeTruthy();
  expect(introBox).toBeTruthy();
  expect(feedBox).toBeTruthy();
  expect(contentPodBox).toBeTruthy();
  expect(searchBox?.x ?? 0).toBeLessThan(firstTabBox?.x ?? 0);
  expect(Math.abs((searchBox?.y ?? 0) - (firstTabBox?.y ?? 0))).toBeLessThan(8);
  expect(tabMenuBox?.y ?? 0).toBeLessThan(introBox?.y ?? 0);
  expect(introBox?.y ?? 0).toBeLessThan(feedBox?.y ?? 0);
  expect(feedBox?.width ?? 0).toBeLessThan((contentPodBox?.width ?? 0) - 48);
  const feedCenter = (feedBox?.x ?? 0) + (feedBox?.width ?? 0) / 2;
  const contentCenter =
    (contentPodBox?.x ?? 0) + (contentPodBox?.width ?? 0) / 2;
  expect(Math.abs(feedCenter - contentCenter)).toBeLessThan(24);

  const stats = page.getByTestId("projects-overview-stat");
  await expect(stats).toHaveCount(5);
  await expect(stats.nth(2)).toContainText("Channels");
  const createBox = await page
    .getByTestId("projects-overview-create-project")
    .boundingBox();
  const lastStatBox = await stats.last().boundingBox();
  const peopleBox = await page
    .getByTestId("projects-overview-people")
    .boundingBox();
  expect(createBox).toBeTruthy();
  expect(lastStatBox).toBeTruthy();
  expect(peopleBox).toBeTruthy();
  expect(peopleBox?.y ?? 0).toBeGreaterThan(createBox?.y ?? 0);
  expect(peopleBox?.y ?? 0).toBeGreaterThan(
    (lastStatBox?.y ?? 0) + (lastStatBox?.height ?? 0) - 1,
  );

  const activityCards = page.getByTestId("projects-activity-card");
  await expect(activityCards.first()).toBeVisible();
  const activityCardCount = await activityCards.count();
  for (let index = 0; index < activityCardCount; index += 1) {
    await expect(activityCards.nth(index)).toHaveCSS(
      "background-color",
      "rgba(0, 0, 0, 0)",
    );
    await expect(activityCards.nth(index)).toHaveCSS("border-style", "solid");
  }

  await page.getByTestId("projects-section-channels").click();
  await expect(page.getByTestId("projects-page-header")).toContainText(
    "Channels",
  );
  await expect(
    page.getByTestId("projects-overview-context-panel"),
  ).toBeVisible();
  await expect(page.getByTestId("projects-overview-context-title")).toHaveText(
    "Channels",
  );
  await expect(
    page.getByTestId("projects-overview-create-project"),
  ).toHaveCount(0);
  await expect(page.getByTestId("projects-overview-people")).toHaveCount(0);
  await expect(page.getByTestId("projects-overview-activity")).toHaveCount(0);
  await expect(stats).toHaveCount(3);
  await expect(stats.first()).toContainText("Channels");
  await expect(page.getByTestId("projects-overview-layout")).toHaveAttribute(
    "data-project-context-detached",
    "true",
  );
  const channelRows = page.getByTestId("project-channel-row");
  // One row per project or repository binding: buzz, buzz · relay-tools,
  // design-system, and relay-tools all bind the shared mock channel.
  await expect(channelRows).toHaveCount(4);
  await expect(channelRows.first()).toContainText("#general");
  const channelsList = page.getByTestId("projects-channels-list");
  await expect(channelsList).toContainText("buzz");
  await expect(channelsList).toContainText("relay-tools");
  await expect(channelsList).toContainText("design-system");
  await expect(page.getByTestId("project-channel-project")).toHaveCount(4);
  await expect(page.getByTestId("project-channel-repository")).toHaveCount(4);
  await page.getByTestId("projects-section-projects").click();
  await expect(page.getByTestId("projects-page-header")).toContainText(
    "Projects",
  );
  await expect(
    page.getByTestId("projects-overview-context-panel"),
  ).toBeVisible();
  await expect(page.getByTestId("projects-overview-context-title")).toHaveText(
    "Projects",
  );
  await expect(
    page.getByTestId("projects-overview-create-project"),
  ).toBeVisible();
  await expect(page.getByTestId("projects-overview-people")).toBeVisible();
  await expect(page.getByTestId("projects-overview-activity")).toBeVisible();
  await page.getByTestId("projects-section-repositories").click();
  await expect(page.getByTestId("projects-overview-context-title")).toHaveText(
    "Repositories",
  );
  await expect(
    page.getByTestId("projects-overview-create-project"),
  ).toHaveCount(0);
  await expect(stats.nth(1)).toContainText("Active tasks");
  await expect(page.getByTestId("projects-overview-activity")).toBeVisible();
  await page.getByTestId("projects-section-issues").click();
  await expect(page.getByTestId("projects-overview-context-title")).toHaveText(
    "Tasks",
  );
  await expect(
    page.getByTestId("projects-overview-create-issue"),
  ).toContainText("Create task");
  await expect(page.getByTestId("projects-overview-activity")).toHaveCount(0);
  await page.getByTestId("projects-section-prs").click();
  await expect(page.getByTestId("projects-overview-context-title")).toHaveText(
    "Reviews",
  );
  await expect(
    page.getByTestId("projects-overview-create-pull-request"),
  ).toContainText("Create review");
  await expect(page.getByTestId("projects-overview-people")).toBeVisible();
  await expect(page.getByTestId("projects-overview-activity")).toHaveCount(0);
  await page.getByTestId("projects-section-all").click();
  await expect(
    page.getByTestId("projects-overview-context-panel"),
  ).toBeVisible();
  await expect(page.getByTestId("projects-overview-context-title")).toHaveText(
    "Projects",
  );
  await expect(page.getByTestId("projects-overview-activity")).toBeVisible();

  await page.getByTestId("projects-overview-create-project").click();
  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("create-project-dialog")).toBeHidden();

  await page.getByTestId("projects-overview-context-toggle").click();
  await expect(
    page.getByTestId("projects-overview-context-panel"),
  ).not.toBeVisible();
  await expect(
    page.getByTestId("projects-overview-layout"),
  ).not.toHaveAttribute("data-project-context-detached", "true");
  await expect(stats).toHaveCount(0);
  await expect(activityCards.first()).toBeVisible();
  await expect(page.getByTestId("projects-create-menu")).toBeVisible();
});

test("repository rows identify their git host", async ({ page }) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByRole("button", { name: "Repositories", exact: true }).click();
  await page.getByRole("button", { name: "List layout" }).click();

  const buzzHostIcon = page
    .getByTestId("repository-row-buzz")
    .getByTestId("repository-host-icon");
  await expect(buzzHostIcon).toHaveAttribute(
    "aria-label",
    "Buzz-hosted repository",
  );
  await expect(
    page
      .getByTestId("repository-row-relay-tools")
      .getByTestId("repository-host-icon"),
  ).toHaveAttribute("aria-label", "Git data hosted on github.com");

  await buzzHostIcon.hover();
  await expect(
    page.getByRole("tooltip", { name: "Buzz-hosted repository" }),
  ).toBeVisible();
});

test("project subsections do not paint backgrounds behind list or grid items", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();

  for (const section of ["Repositories", "Reviews", "Tasks"] as const) {
    await page.getByRole("button", { name: section, exact: true }).click();
    await page.getByRole("button", { name: "List layout" }).click();

    const listItems = page.locator(
      section === "Repositories"
        ? '[data-testid^="repository-row-"]'
        : section === "Reviews"
          ? '[data-testid^="projects-pr-row-"]'
          : '[data-testid^="projects-issue-row-"]',
    );
    await expect(listItems.first()).toBeVisible();
    const listItemCount = await listItems.count();
    for (let index = 0; index < listItemCount; index += 1) {
      await expect(listItems.nth(index)).toHaveCSS(
        "background-color",
        "rgba(0, 0, 0, 0)",
      );
    }

    await page.getByRole("button", { name: "Grid layout" }).click();
    const gridCards = page.locator(
      section === "Repositories"
        ? '[data-testid^="repository-card-"]'
        : "[data-projects-grid-card]",
    );
    await expect(gridCards.first()).toBeVisible();
    const gridCardCount = await gridCards.count();
    for (let index = 0; index < gridCardCount; index += 1) {
      await expect(gridCards.nth(index)).toHaveCSS(
        "background-color",
        "rgba(0, 0, 0, 0)",
      );
      await expect(gridCards.nth(index)).toHaveCSS("border-style", "solid");
    }
  }
});

test("project detail content areas do not paint background fills", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await openBuzzProject(page);

  const expectVisiblePanelsToBeTransparent = async ({
    bordered = true,
    required = false,
  }: {
    bordered?: boolean;
    required?: boolean;
  } = {}) => {
    const panels = page.locator("[data-project-detail-panel]:visible");
    const panelCount = await panels.count();
    if (required) await expect(panels.first()).toBeVisible();
    for (let index = 0; index < panelCount; index += 1) {
      await expect(panels.nth(index)).toHaveCSS(
        "background-color",
        "rgba(0, 0, 0, 0)",
      );
      if (bordered) {
        await expect(panels.nth(index)).toHaveCSS("border-style", "solid");
      } else {
        await expect(panels.nth(index)).toHaveCSS("border-width", "0px");
      }
    }
  };

  for (const tab of [
    "Overview",
    "Files",
    "Commits",
    "Tasks",
    "Review",
    "Contributors",
  ]) {
    await page.getByRole("tab", { name: tab, exact: true }).click();
    await expectVisiblePanelsToBeTransparent();
  }

  await page.getByRole("tab", { name: "Review", exact: true }).click();
  const pullRequest = page.getByTestId("project-pull-request-row").first();
  await expect(pullRequest).toBeVisible();
  await pullRequest.getByRole("button", { name: /^#/ }).click();
  await expectVisiblePanelsToBeTransparent({
    bordered: false,
    required: true,
  });
});

test("project without a checkout offers fetch feedback and cloning", async ({
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
  ).toBeVisible();
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
  const commands = await page.evaluate(
    () => window.__BUZZ_E2E_COMMANDS__ ?? [],
  );
  expect(commands).toContain("clone_project_repository");
});

test("project branches can be created from the selected remote branch", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page, {
    projectHeadBranch: "master",
    relaySelf: TEST_IDENTITIES.bob.pubkey,
  });
  await openBuzzProject(page);

  await page.getByRole("button", { name: /main/ }).click();
  await page.getByTestId("project-create-branch").click();
  await page
    .getByTestId("project-create-branch-name")
    .fill("feature/branch-management");
  await page.getByTestId("project-create-branch-submit").click();

  await expect(
    page.getByText("Created branch feature/branch-management from main.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /feature\/branch-management/ }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: /feature\/branch-management/ })
    .click();
  await expect(
    page.getByRole("menuitemradio", { name: "feature/branch-management" }),
  ).toBeVisible();
  await page.getByRole("menuitemradio", { name: "main" }).click();
  await page.getByRole("button", { name: /main/ }).click();
  await expect(
    page.getByRole("menuitemradio", { name: "feature/branch-management" }),
  ).toBeVisible();
  const commands = await page.evaluate(
    () => window.__BUZZ_E2E_COMMANDS__ ?? [],
  );
  expect(commands).toContain("create_project_remote_branch");

  await openBuzzProject(page);
  await page.getByRole("button", { name: /main/ }).click();
  await expect(
    page.getByRole("menuitemradio", { name: "feature/branch-management" }),
  ).toBeVisible();
});

test("repository tags can be browsed as immutable remote snapshots", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("button", { name: /main/ }).click();
  await expect(page.getByText("Tags", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("menuitemradio", { name: /v1\.0\.0.*0123456/ }),
  ).toBeVisible();
  await page.getByRole("menuitemradio", { name: /v1\.0\.0.*0123456/ }).click();

  await expect(page.getByRole("button", { name: /v1\.0\.0/ })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Remote", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const call = [...(window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])]
          .reverse()
          .find((entry) => entry.command === "get_project_repo_snapshot");
        return (call?.payload as { targetRef?: string } | undefined)?.targetRef;
      }),
    )
    .toBe("refs/tags/v1.0.0");
  await page.getByRole("button", { name: /v1\.0\.0/ }).click();
  await expect(page.getByTestId("project-create-branch")).toHaveCount(0);
  await expect(page.getByTestId("project-delete-branch")).toHaveCount(0);

  await page.getByRole("menuitemradio", { name: "main" }).click();
  await page.getByRole("button", { name: /main/ }).click();
  await expect(page.getByTestId("project-create-branch")).toBeVisible();
});

test("project branches can be deleted but the default branch cannot", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("button", { name: /main/ }).click();
  await expect(page.getByTestId("project-delete-branch")).toBeDisabled();
  await page.getByTestId("project-create-branch").click();
  await page
    .getByTestId("project-create-branch-name")
    .fill("feature/delete-me");
  await page.getByTestId("project-create-branch-submit").click();
  await expect(
    page.getByRole("button", { name: /feature\/delete-me/ }),
  ).toBeVisible();
  await page.getByRole("button", { name: /feature\/delete-me/ }).click();
  await page.getByTestId("project-delete-branch").click();
  await expect(page.getByTestId("project-delete-branch-dialog")).toBeVisible();
  await page.getByTestId("project-delete-branch-submit").click();

  await expect(
    page.getByText("Deleted branch feature/delete-me.", { exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: /main/ })).toBeVisible();
  const commands = await page.evaluate(
    () => window.__BUZZ_E2E_COMMANDS__ ?? [],
  );
  expect(commands).toContain("delete_project_remote_branch");
});

test("external repositories stay on local source after a branch round trip", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    const commit = "0123456789abcdef0123456789abcdef01234567";
    const localBranch =
      "wintermute/entity-link-recipient-cards-with-a-long-branch-name";
    window.sessionStorage.setItem(
      "buzz-e2e-project-branches",
      JSON.stringify({ "relay-tools": { [localBranch]: commit } }),
    );
    window.__BUZZ_E2E_PROJECT_REPO_SYNC_STATUS__ = {
      local_path: "/tmp/buzz/REPOS/relay-tools",
      local_branch: localBranch,
      local_branches: ["main", localBranch],
      local_head: commit,
      local_short_head: commit.slice(0, 7),
      remote_branch: localBranch,
      remote_head: commit,
      remote_short_head: commit.slice(0, 7),
      merge_base: commit,
      ahead_count: 0,
      behind_count: 0,
      has_uncommitted_changes: false,
      has_untracked_files: false,
      can_push: false,
      push_block_reason: "Local branch is already pushed.",
      can_pull: false,
      pull_block_reason: "Local branch is up to date.",
    };
    window.__BUZZ_E2E_PROJECT_LOCAL_REPO_SNAPSHOT__ = {
      path: "/tmp/buzz/REPOS/relay-tools",
      snapshot: {
        latest_commit: null,
        commits: [],
        contributors: [],
        files: [
          {
            path: "README.md",
            kind: "text",
            size: 21,
            preview_content: "# Local branch README",
            last_changed_at: null,
            latest_commit: null,
          },
        ],
      },
    };
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await addProjectToSidebar(page, "buzz");
  await page.getByTestId("sidebar-project-repository-relay-tools").click();

  await expect(
    page.getByRole("heading", { name: "Local branch README" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Local", exact: true }),
  ).toBeVisible();
  await expectLocalRepositoryOpenAction(page);

  await page.getByRole("button", { name: /main/ }).click();
  await page
    .getByRole("menuitemradio", {
      name: "wintermute/entity-link-recipient-cards-with-a-long-branch-name",
    })
    .click();
  const branchTrigger = page.getByTestId("project-repository-branch-trigger");
  await expect(
    page.getByRole("button", {
      name: /wintermute\/entity-link-recipient-cards-with-a-long-branch-name/,
    }),
  ).toBeVisible();
  await expect
    .poll(() =>
      branchTrigger.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    )
    .toBe(true);
  await expect
    .poll(() =>
      branchTrigger
        .locator("span")
        .evaluate((element) => element.scrollWidth > element.clientWidth),
    )
    .toBe(true);
  await expect(
    page.getByRole("button", { name: "Local", exact: true }),
  ).toBeVisible();
  await expectLocalRepositoryOpenAction(page);

  await branchTrigger.click();
  await page.getByRole("menuitemradio", { name: "main" }).click();

  await expect(page.getByRole("button", { name: /main/ })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Local", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Local branch README" }),
  ).toBeVisible();
  await expectLocalRepositoryOpenAction(page);
  await expect(page.getByText("Code hosted on github.com")).toHaveCount(0);
});

test("repository files beyond the eager preview limit load on demand", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    const deferredFiles = [
      ...Array.from({ length: 250 }, (_, index) => ({
        path: `.agents/generated-${String(index).padStart(3, "0")}.txt`,
        kind: "blob",
        size: 7,
        preview_content: "preview",
        last_changed_at: null,
        latest_commit: null,
      })),
      {
        path: "README.md",
        kind: "blob",
        size: 17,
        preview_content: null,
        last_changed_at: null,
        latest_commit: null,
      },
      {
        path: "src/application.rs",
        kind: "blob",
        size: 16,
        preview_content: null,
        last_changed_at: null,
        latest_commit: null,
      },
    ];
    window.__BUZZ_E2E_PROJECT_LOCAL_REPO_SNAPSHOT__ = {
      path: "/tmp/buzz/REPOS/relay-tools",
      snapshot: {
        latest_commit: null,
        commits: [],
        contributors: [],
        files: deferredFiles,
      },
    };
    window.__BUZZ_E2E_PROJECT_REPO_FILE_CONTENTS__ = {
      "README.md": "# Deferred README",
      "src/application.rs": "fn deferred() {}",
    };
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await addProjectToSidebar(page, "buzz");
  await page.getByTestId("sidebar-project-repository-relay-tools").click();

  await expect(
    page.getByRole("heading", { name: "Deferred README" }),
  ).toBeVisible();

  await page.getByRole("tab", { name: "Files" }).click();
  await page.getByRole("row", { name: "Open directory src" }).click();
  await page.getByRole("row", { name: "Open file application.rs" }).click();
  await expect(
    page.getByText("fn deferred() {}", { exact: true }),
  ).toBeVisible();

  const commands = await page.evaluate(
    () => window.__BUZZ_E2E_COMMANDS__ ?? [],
  );
  expect(commands).toContain("get_project_local_repo_file_content");
});

test("pushed local branch can open a pull request", async ({ page }) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    const commit = "1234567890abcdef1234567890abcdef12345678";
    window.__BUZZ_E2E_PROJECT_REPO_SYNC_STATUS__ = {
      local_path: "/tmp/buzz/REPOS/buzz",
      local_branch: "feature/projects-workflow",
      local_branches: ["feature/projects-workflow", "space"],
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
  });
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("button", { name: /main/ }).click();
  await expect(
    page.getByRole("menuitemradio", { name: "space" }),
  ).toBeVisible();
  await page
    .getByRole("menuitemradio", { name: "feature/projects-workflow" })
    .click();
  await page.getByRole("tab", { name: "Review", exact: true }).click();
  await page.getByRole("button", { name: "Create review" }).click();
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
  await expect(page.getByText("Review created.")).toBeVisible();

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
});

test("project task can be created with a category from the tasks header", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await openBuzzProject(page);

  await page.getByRole("tab", { name: "Tasks", exact: true }).click();
  await page
    .getByTestId("project-section-header")
    .getByRole("button", { name: "Create task" })
    .click();
  await page
    .getByTestId("create-issue-category")
    .selectOption("change-request");
  await page
    .getByTestId("create-issue-title")
    .fill("Document the broken workflow");
  await page
    .getByTestId("create-issue-body")
    .fill("The project workflow needs a clear repair path.");
  await page.getByTestId("create-issue-submit").click();
  await expect(page.getByText("Task created.")).toBeVisible();

  const createdEvent = await page.evaluate(() =>
    window.__BUZZ_E2E_SIGNED_EVENTS__?.find((event) => event.kind === 1621),
  );
  expect(createdEvent?.tags).toContainEqual([
    "subject",
    "Document the broken workflow",
  ]);
  expect(createdEvent?.tags).toContainEqual(["t", "change-request"]);
});

test("narrow layouts keep section context reachable through a sheet", async ({
  page,
}) => {
  await page.setViewportSize({ height: 720, width: 900 });
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();

  // Below the detached breakpoint the docked rail never renders, but the
  // context toggle stays available.
  await expect(page.getByTestId("projects-page-tabs")).toBeVisible();
  await expect(page.getByTestId("projects-overview-context-panel")).toHaveCount(
    0,
  );
  const contextToggle = page.getByTestId("projects-overview-context-toggle");
  await expect(contextToggle).toBeVisible();
  await expect(contextToggle).toHaveAttribute("aria-pressed", "false");

  // Keyboard journey into the Tasks section: open the sheet from the toggle.
  await page.getByTestId("projects-section-issues").click();
  await expect(page.getByTestId("projects-page-header")).toContainText("Tasks");
  await contextToggle.focus();
  await page.keyboard.press("Enter");
  const contextSheet = page.getByTestId("projects-overview-context-sheet");
  await expect(contextSheet).toBeVisible();
  await expect(
    contextSheet.getByTestId("projects-overview-context-title"),
  ).toHaveText("Tasks");
  await expect(contextToggle).toHaveAttribute("aria-pressed", "true");

  // Escape dismisses the sheet and returns focus to the toggle.
  await page.keyboard.press("Escape");
  await expect(contextSheet).toBeHidden();
  await expect(contextToggle).toBeFocused();
  await expect(contextToggle).toHaveAttribute("aria-pressed", "false");

  // The same journey follows the Reviews section.
  await page.getByTestId("projects-section-prs").click();
  await expect(page.getByTestId("projects-page-header")).toContainText(
    "Reviews",
  );
  await contextToggle.focus();
  await page.keyboard.press("Enter");
  await expect(contextSheet).toBeVisible();
  await expect(
    contextSheet.getByTestId("projects-overview-context-title"),
  ).toHaveText("Reviews");
  await page.keyboard.press("Escape");
  await expect(contextSheet).toBeHidden();

  // Selecting a section inside the sheet navigates and dismisses it: the
  // Activity context lists every section as a stat row.
  await page.getByTestId("projects-section-all").click();
  await contextToggle.focus();
  await page.keyboard.press("Enter");
  await expect(contextSheet).toBeVisible();
  await contextSheet
    .getByTestId("projects-overview-stat")
    .filter({ hasText: "Tasks" })
    .click();
  await expect(contextSheet).toBeHidden();
  await expect(page.getByTestId("projects-page-header")).toContainText("Tasks");
});
