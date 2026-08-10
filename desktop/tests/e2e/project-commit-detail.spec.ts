import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/project-commit-detail";
const ALIGNMENT_TOLERANCE_PX = 2;

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

test("top-level project lists align dates and overflow actions", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    window.localStorage.setItem("buzz.projects.viewMode", "list");
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await expect(
    page.getByRole("heading", { level: 1, name: "Projects" }),
  ).toBeVisible();

  async function trailingPositions(
    row: import("@playwright/test").Locator,
    {
      actionName = /More options for/,
      dateTestId = "projects-row-date",
      summaryTestId,
    }: {
      actionName?: RegExp;
      dateTestId?: string;
      summaryTestId?: string;
    } = {},
  ) {
    await waitForAnimations(page);
    const date = row.getByTestId(dateTestId);
    const menu = row.getByRole("button", { name: actionName });
    await expect(date).toBeVisible();
    await expect(menu).toBeVisible();
    const dateBox = await date.boundingBox();
    const menuBox = await menu.boundingBox();
    const rowBox = await row.boundingBox();
    const summaryBox = summaryTestId
      ? await row.getByTestId(summaryTestId).boundingBox()
      : null;
    expect(dateBox).not.toBeNull();
    expect(menuBox).not.toBeNull();
    expect(rowBox).not.toBeNull();
    if (summaryTestId) expect(summaryBox).not.toBeNull();
    return {
      dateX: dateBox?.x ?? 0,
      menuX: menuBox?.x ?? 0,
      rowHeight: rowBox?.height ?? 0,
      summaryX: summaryBox?.x ?? null,
    };
  }

  await page.getByTestId("projects-section-projects").click();
  await page.getByRole("button", { name: "Filter projects" }).click();
  await expect(
    page.getByRole("menuitem", { name: "My Projects" }),
  ).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Local" })).toBeVisible();
  await page.keyboard.press("Escape");
  const projectPositions = await trailingPositions(
    page.locator('[data-testid^="project-row-"]').first(),
    { summaryTestId: "projects-row-summary" },
  );

  await page.getByTestId("projects-section-repositories").click();
  await page.getByRole("button", { name: "Filter repositories" }).click();
  await expect(
    page.getByRole("menuitem", { name: "My Repositories" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("repository-row-buzz")).toBeVisible();
  await expect(page.getByTestId("repository-row-relay-tools")).toBeVisible();
  const repositoryRow = page.getByTestId("repository-row-buzz");
  await expect(
    repositoryRow.getByTestId("repositories-row-summary"),
  ).toContainText("commits");
  await expect(
    repositoryRow.getByTestId("repositories-row-branch"),
  ).toContainText("main");
  // Subtitle is the repository location (owner/repo for Buzz-hosted repos).
  await expect(repositoryRow.locator("p")).toHaveText(/\/buzz$/);
  const repositoryPositions = await trailingPositions(repositoryRow, {
    actionName: /More options for/,
    dateTestId: "repositories-row-date",
    summaryTestId: "repositories-row-summary",
  });
  expect(
    Math.abs(
      (repositoryPositions.summaryX ?? 0) - (projectPositions.summaryX ?? 0),
    ),
  ).toBeLessThanOrEqual(ALIGNMENT_TOLERANCE_PX);
  expect(
    Math.abs(repositoryPositions.rowHeight - projectPositions.rowHeight),
  ).toBeLessThanOrEqual(ALIGNMENT_TOLERANCE_PX);
  expect(
    Math.abs(repositoryPositions.dateX - projectPositions.dateX),
  ).toBeLessThanOrEqual(ALIGNMENT_TOLERANCE_PX);
  expect(
    Math.abs(repositoryPositions.menuX - projectPositions.menuX),
  ).toBeLessThanOrEqual(ALIGNMENT_TOLERANCE_PX);
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/05-project-repositories-list.png`,
  });
  await page
    .getByTestId("repository-row-relay-tools")
    .getByRole("button", { name: "More options for relay-tools" })
    .click();
  await expect(
    page.getByRole("menuitem", { name: "Clone & open in Terminal" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");

  await page
    .getByRole("button", { name: "Pull Requests", exact: true })
    .click();
  await page.getByRole("button", { name: "Filter pull requests" }).click();
  await expect(
    page.getByRole("menuitem", { name: "My Pull Requests" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByTestId("projects-create-menu").hover();
  await expect(page.getByRole("menuitem", { name: "Project" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Issue" })).toBeVisible();
  await page
    .getByRole("menuitem", { name: "Pull Request", exact: true })
    .click();
  await expect(page.getByTestId("create-pull-request-dialog")).toBeVisible();
  await expect(
    page.getByTestId("create-pull-request-repository"),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByTestId("projects-create-menu").hover();
  await page.getByRole("menuitem", { name: "Issue" }).click();
  await expect(page.getByTestId("create-issue-repository")).toBeVisible();
  await page.keyboard.press("Escape");
  const pullRequestRow = page
    .locator('[data-testid^="projects-pr-row-"]')
    .first();
  const pullRequestPositions = await trailingPositions(pullRequestRow);
  await pullRequestRow
    .getByRole("button", { name: /More options for/ })
    .click();
  await expect(
    page.getByRole("menuitem", { name: /Review PR|View (draft|merge|closed)/ }),
  ).toBeVisible();
  await page.keyboard.press("Escape");

  await page.getByRole("button", { name: "Issues", exact: true }).click();
  await page.getByRole("button", { name: "Filter issues" }).click();
  await expect(page.getByRole("menuitem", { name: "My Issues" })).toBeVisible();
  await page.keyboard.press("Escape");
  const issueRow = page.locator('[data-testid^="projects-issue-row-"]').first();
  const issuePositions = await trailingPositions(issueRow);

  expect(
    Math.abs(pullRequestPositions.dateX - projectPositions.dateX),
  ).toBeLessThanOrEqual(ALIGNMENT_TOLERANCE_PX);
  expect(
    Math.abs(pullRequestPositions.menuX - projectPositions.menuX),
  ).toBeLessThanOrEqual(ALIGNMENT_TOLERANCE_PX);
  expect(
    Math.abs(issuePositions.dateX - projectPositions.dateX),
  ).toBeLessThanOrEqual(ALIGNMENT_TOLERANCE_PX);
  expect(
    Math.abs(issuePositions.menuX - projectPositions.menuX),
  ).toBeLessThanOrEqual(ALIGNMENT_TOLERANCE_PX);

  await page.setViewportSize({ height: 720, width: 900 });
  await page.getByTestId("projects-section-projects").click();
  const responsiveRepositoryRow = page
    .locator('[data-testid^="project-row-"]')
    .first();
  await expect(
    responsiveRepositoryRow.getByTestId("projects-row-summary"),
  ).toBeHidden();
  await expect(
    responsiveRepositoryRow.getByTestId("projects-row-people"),
  ).toBeHidden();
  await expect(
    responsiveRepositoryRow.getByTestId("projects-row-date"),
  ).toBeVisible();
  await expect(
    responsiveRepositoryRow.getByRole("button", { name: /More options for/ }),
  ).toBeVisible();
  expect(
    await responsiveRepositoryRow.evaluate(
      (row) => row.scrollWidth <= row.clientWidth,
    ),
  ).toBe(true);
});

test("creating a project publishes its initial repository grouping", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByTestId("projects-create-menu").hover();
  await page.getByRole("menuitem", { name: "Project" }).click();
  await page.getByTestId("create-project-name").fill("multi-repo-demo");
  await page
    .getByTestId("create-project-description")
    .fill("A grouped project created through the desktop app.");
  await page
    .getByTestId("create-project-clone-url")
    .fill("https://relay.example.com/git/owner/multi-repo-demo.git");
  await page.getByTestId("create-project-submit").click();

  await expect(page.getByTestId("create-project-dialog")).toBeHidden();
  await expect(
    page
      .locator(
        '[data-testid="project-card-multi-repo-demo"], [data-testid="project-row-multi-repo-demo"]',
      )
      .first(),
  ).toBeVisible();

  const createdEvents = await page.evaluate(
    () =>
      window.__BUZZ_E2E_ACCEPTED_PROJECT_EVENTS__?.filter((event) =>
        event.tags.some(
          (tag) => tag[0] === "d" && tag[1] === "multi-repo-demo",
        ),
      ) ?? [],
  );
  expect(createdEvents.map((event) => event.kind).sort()).toEqual([
    30617, 30621,
  ]);
  const projectEvent = createdEvents.find((event) => event.kind === 30621);
  expect(projectEvent?.tags).toContainEqual([
    "a",
    `30617:${"deadbeef".repeat(8)}:multi-repo-demo`,
  ]);
  expect(projectEvent?.content).toBe("");

  await page.getByTestId("projects-create-menu").hover();
  await page.getByRole("menuitem", { name: "Project" }).click();
  await page.getByTestId("create-project-name").fill("multi-repo-demo");
  await page.getByTestId("create-project-submit").click();
  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  await expect(
    page.getByText('You already have a project named "multi-repo-demo".'),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_ACCEPTED_PROJECT_EVENTS__?.filter((event) =>
            event.tags.some(
              (tag) => tag[0] === "d" && tag[1] === "multi-repo-demo",
            ),
          ).length ?? 0,
      ),
    )
    .toBe(2);
});

test("unsupported relays keep the initial repository accessible", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    window.__BUZZ_E2E_UNSUPPORTED_PROJECT_ANNOUNCEMENTS__ = true;
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByTestId("projects-create-menu").hover();
  await page.getByRole("menuitem", { name: "Project" }).click();
  await page.getByTestId("create-project-name").fill("legacy-fallback");
  await page.getByTestId("create-project-submit").click();

  await expect(page.getByTestId("create-project-dialog")).toBeHidden();
  await expect(page.getByText("Created as a standalone project")).toBeVisible();
  await waitForAnimations(page);
  const projectEntry = page
    .locator(
      '[data-testid="project-card-legacy-fallback"], [data-testid="project-row-legacy-fallback"]',
    )
    .first();
  await expect(projectEntry).toBeVisible();
  await projectEntry
    .getByRole("button", { name: "View legacy-fallback" })
    .click();
  await expect(page.getByTestId("project-repository-picker")).toContainText(
    "legacy-fallback",
  );
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/06-single-repository-add.png`,
  });

  const acceptedKinds = await page.evaluate(
    () =>
      window.__BUZZ_E2E_ACCEPTED_PROJECT_EVENTS__
        ?.filter((event) =>
          event.tags.some(
            (tag) => tag[0] === "d" && tag[1] === "legacy-fallback",
          ),
        )
        .map((event) => event.kind) ?? [],
  );
  expect(acceptedKinds).toEqual([30617]);
});

test("project creation can retry after its repository publication fails", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_EVENT_KINDS__ = [30621];
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByTestId("projects-create-menu").hover();
  await page.getByRole("menuitem", { name: "Project" }).click();
  await page.getByTestId("create-project-name").fill("retry-project");
  await page.getByTestId("create-project-submit").click();

  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  await expect(page.getByText("mock project event rejection")).toBeVisible();

  await page.getByTestId("create-project-submit").click();
  await expect(page.getByTestId("create-project-dialog")).toBeHidden();
  await expect(
    page
      .locator(
        '[data-testid="project-card-retry-project"], [data-testid="project-row-retry-project"]',
      )
      .first(),
  ).toBeVisible();
});

test("project creation is idempotent after a lost publish acknowledgement", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    window.__BUZZ_E2E_FAIL_PROJECT_EVENT_ACK_KINDS__ = [30621];
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByTestId("projects-create-menu").hover();
  await page.getByRole("menuitem", { name: "Project" }).click();
  await page.getByTestId("create-project-name").fill("lost-ack-project");
  await page.getByTestId("create-project-submit").click();

  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  await expect(
    page.getByText("mock lost project acknowledgement"),
  ).toBeVisible();

  await page.getByTestId("create-project-submit").click();
  await expect(page.getByTestId("create-project-dialog")).toBeHidden();
  await expect(
    page
      .locator(
        '[data-testid="project-card-lost-ack-project"], [data-testid="project-row-lost-ack-project"]',
      )
      .first(),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_ACCEPTED_PROJECT_EVENTS__?.filter((event) =>
            event.tags.some(
              (tag) => tag[0] === "d" && tag[1] === "lost-ack-project",
            ),
          ).length ?? 0,
      ),
    )
    .toBe(2);
});

test("multi-repository projects switch the active repository", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByTestId("projects-section-projects").click();
  await page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first()
    .click();

  const picker = page.getByTestId("project-repository-picker");
  await expect(picker).toContainText("buzz");
  await picker.click();
  await expect(
    page.getByTestId("project-repository-relay-tools"),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/04-multi-repository-picker.png`,
  });

  await page.getByTestId("project-repository-relay-tools").click();
  await expect(picker).toContainText("relay-tools");
  await expect(page).toHaveURL(
    new RegExp(`repositoryId=${TEST_IDENTITIES.alice.pubkey}%3Arelay-tools`),
  );

  await page.getByTestId("add-project-repository").click();
  await page.getByTestId("create-project-repository").click();
  await page.getByTestId("add-project-repository-name").fill("mobile-app");
  await page.getByTestId("add-project-repository-submit").click();
  await expect(page.getByTestId("add-project-repository-dialog")).toBeHidden();
  await expect(picker).toContainText("mobile-app");
  const addedEvents = await page.evaluate(
    () =>
      window.__BUZZ_E2E_ACCEPTED_PROJECT_EVENTS__?.filter(
        (event) =>
          event.tags.some((tag) => tag[0] === "d" && tag[1] === "mobile-app") ||
          event.tags.some(
            (tag) =>
              tag[0] === "a" &&
              tag[1]?.endsWith(":mobile-app") &&
              event.kind === 30621,
          ),
      ) ?? [],
  );
  expect(addedEvents.map((event) => event.kind)).toEqual([30621, 30617]);
  expect(
    addedEvents.find((event) => event.kind === 30617)?.tags,
  ).toContainEqual(["buzz-channel", "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50"]);

  await page.getByTestId("add-project-repository").click();
  await page.getByTestId("attach-project-repository").click();
  await expect(
    page.getByTestId("attach-project-repository-dialog"),
  ).toBeVisible();
  await page.getByTestId("attach-existing-repository-design-system").click();
  await expect(
    page.getByTestId("attach-project-repository-dialog"),
  ).toBeHidden();
  await expect(picker).toContainText("design-system");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_ACCEPTED_PROJECT_EVENTS__?.some(
            (event) =>
              event.kind === 30621 &&
              event.tags.some(
                (tag) => tag[0] === "a" && tag[1]?.endsWith(":design-system"),
              ),
          ) ?? false,
      ),
    )
    .toBe(true);
});

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
  // Projects filter reveals the complete project cards/rows list.
  await page.getByTestId("projects-section-projects").click();

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

  // Commits share the rounded list structure used by issues and pull requests.
  await expect(
    page.getByRole("heading", { name: "Commits", exact: true }),
  ).toBeVisible();
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
  await expect(
    page.getByRole("link", { name: "project guide", exact: true }),
  ).toHaveAttribute("href", "https://example.com/project-guide");
  await expect(
    page.getByRole("button", { name: "Architecture" }),
  ).toBeVisible();
  await expect(page.locator("video")).toHaveAttribute(
    "src",
    "https://example.com/project-demo.mp4",
  );

  // Diff from the mocked get_project_repo_diff renders changed files.
  await expect(page.getByText("2 changed files")).toBeVisible({
    timeout: 10_000,
  });
  await expect(
    page.getByText("CommunityTabs({ selectedCommitHash })"),
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
  // Projects filter reveals the complete project cards/rows list.
  await page.getByTestId("projects-section-projects").click();

  const projectEntry = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  await expect(projectEntry).toBeVisible({ timeout: 10_000 });
  await projectEntry.click();

  // PR rows use the shared feed row: title button + #id cluster cell.
  await page.getByRole("tab", { name: "Pull Request" }).click();
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
  ).toContainText("Pull Request");

  // Step back to the feed so the community tabs are available again.
  await page
    .getByRole("navigation", { name: "Project breadcrumb" })
    .getByRole("button", { name: "Pull Request", exact: true })
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

test("adding a repository retries and reports an error when the 30617 publication is rejected", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  // Reject the repository-announcement event on every attempt so the mutation
  // exhausts its retry and surfaces a partial-write error.
  await page.addInitScript(() => {
    // Reject kind 30617 twice (initial attempt + one retry).
    window.__BUZZ_E2E_REJECT_PROJECT_EVENT_KINDS__ = [30617, 30617];
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByTestId("projects-section-projects").click();
  await page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first()
    .click();

  await page.getByTestId("add-project-repository").click();
  await page.getByTestId("create-project-repository").click();
  await page.getByTestId("add-project-repository-name").fill("rejected-repo");
  await page.getByTestId("add-project-repository-submit").click();

  // The project event (30621) is published; the repository event (30617) is
  // rejected on both attempts. The dialog must surface the partial-write error.
  await expect(page.getByTestId("add-project-repository-dialog")).toBeVisible();
  await expect(
    page.getByText(/repository could not be created/i),
  ).toBeVisible();

  // The 30621 must have been published (project was updated).
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_ACCEPTED_PROJECT_EVENTS__?.some(
            (event) =>
              event.kind === 30621 &&
              event.tags.some(
                (tag) => tag[0] === "a" && tag[1]?.endsWith(":rejected-repo"),
              ),
          ) ?? false,
      ),
    )
    .toBe(true);

  // The 30617 must NOT have been accepted (both attempts were rejected).
  const acceptedRepo = await page.evaluate(
    () =>
      window.__BUZZ_E2E_ACCEPTED_PROJECT_EVENTS__?.some(
        (event) =>
          event.kind === 30617 &&
          event.tags.some(
            (tag) => tag[0] === "d" && tag[1] === "rejected-repo",
          ),
      ) ?? false,
  );
  expect(
    acceptedRepo,
    "30617 must not be accepted when the relay rejects both attempts",
  ).toBe(false);
});

test("adding a repository treats a lost 30617 acknowledgement as success", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  // The relay will accept the 30617 but fail to deliver the ACK, then on the
  // retry query the event will be found — the mutation must succeed.
  await page.addInitScript(() => {
    window.__BUZZ_E2E_FAIL_PROJECT_EVENT_ACK_KINDS__ = [30617];
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByTestId("projects-section-projects").click();
  await page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first()
    .click();

  const picker = page.getByTestId("project-repository-picker");

  await page.getByTestId("add-project-repository").click();
  await page.getByTestId("create-project-repository").click();
  await page.getByTestId("add-project-repository-name").fill("lost-ack-repo");
  await page.getByTestId("add-project-repository-submit").click();

  // The dialog should close — the operation recovered from the lost ACK.
  await expect(page.getByTestId("add-project-repository-dialog")).toBeHidden();
  // The repository picker must reflect the newly added repository.
  await expect(picker).toContainText("lost-ack-repo");

  // Both events must have been accepted: the 30621 (project update) and the
  // 30617 (repository — accepted by relay even though ACK was lost).
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_ACCEPTED_PROJECT_EVENTS__?.filter((event) =>
            event.tags.some(
              (tag) => tag[0] === "d" && tag[1] === "lost-ack-repo",
            ),
          ).length ?? 0,
      ),
    )
    .toBeGreaterThanOrEqual(1);
});

test("adding a repository blocks when a standalone 30617 already exists at that coordinate", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  // Seed a standalone 30617 (not a project member) owned by the mock identity.
  // The add-repo mutation must block unconditionally when this coordinate exists,
  // even though it is not yet in the "buzz" project's member list.
  const MOCK_OWNER = "deadbeef".repeat(8);
  const STANDALONE_DTAG = "existing-standalone";
  await page.addInitScript(
    ({ owner, dtag }) => {
      window.__BUZZ_E2E_EXTRA_PROJECT_EVENTS__ = [
        {
          id: "standalone00".padEnd(64, "0"),
          kind: 30617,
          pubkey: owner,
          created_at: Math.floor(Date.now() / 1000) - 3600,
          content: "A standalone repository that exists outside any project.",
          tags: [
            ["d", dtag],
            ["name", "Existing Standalone"],
            ["clone", "https://git.example.com/standalone.git"],
          ],
        },
      ];
    },
    { owner: MOCK_OWNER, dtag: STANDALONE_DTAG },
  );
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByTestId("projects-section-projects").click();
  await page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first()
    .click();

  await page.getByTestId("add-project-repository").click();
  await page.getByTestId("create-project-repository").click();
  // Use the same name — the dtag will match the seeded standalone 30617.
  await page
    .getByTestId("add-project-repository-name")
    .fill("Existing Standalone");
  await page.getByTestId("add-project-repository-submit").click();

  // The dialog must remain open with a clobber error.
  await expect(page.getByTestId("add-project-repository-dialog")).toBeVisible();
  await expect(
    page.getByText(/already exists.*standalone.*another project/i),
  ).toBeVisible();

  // Neither a 30621 (project update) nor a 30617 (new repo) must have been published.
  const publishedForStandalone = await page.evaluate(
    ({ dtag }) =>
      window.__BUZZ_E2E_ACCEPTED_PROJECT_EVENTS__?.some(
        (event) =>
          event.tags.some((tag) => tag[0] === "d" && tag[1] === dtag) ||
          event.tags.some(
            (tag) => tag[0] === "a" && tag[1]?.endsWith(`:${dtag}`),
          ),
      ) ?? false,
    { dtag: STANDALONE_DTAG },
  );
  expect(
    publishedForStandalone,
    "neither project nor repository event must be published when clobber guard fires",
  ).toBe(false);
});

test("navigating via a 30617 entity-link route opens the correct non-primary repository and renders its PR", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  // Seed a known pull-request for relay-tools (the non-primary member of "buzz")
  // with a deterministic id so the URL can be constructed before navigation.
  const ALICE_PUBKEY =
    "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";
  const RELAY_TOOLS_ADDRESS = `30617:${ALICE_PUBKEY}:relay-tools`;
  const KNOWN_PR_ID = "entity-link-pr-test".padEnd(64, "0");

  await page.addInitScript(
    ({ repoAddress, prId, alicePubkey }) => {
      window.__BUZZ_E2E_EXTRA_PROJECT_EVENTS__ = [
        {
          id: prId,
          kind: 1618, // KIND_GIT_PULL_REQUEST
          pubkey: alicePubkey,
          created_at: Math.floor(Date.now() / 1000) - 60,
          content: "Entity-link test PR from relay-tools",
          tags: [
            ["a", repoAddress],
            ["subject", "Entity-link test PR from relay-tools"],
            ["c", "abc123".padEnd(40, "0")],
            ["h", "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50"],
            ["branch-name", "feature/entity-link-test"],
            ["clone", "https://github.com/block/relay-tools.git"],
          ],
        },
      ];
    },
    {
      repoAddress: RELAY_TOOLS_ADDRESS,
      prId: KNOWN_PR_ID,
      alicePubkey: ALICE_PUBKEY,
    },
  );
  await installMockBridge(page);

  // Navigate via the entity-link route using the hash router URL format.
  // Python's http.server (the e2e web server) serves only index.html at `/`;
  // a direct page.goto to `/projects/...` returns 404 because there is no
  // SPA fallback. The app uses createHashHistory(), so the correct URL is
  // `/#/projects/<id>?...` — the server always sees just `/` and the hash
  // fragment is resolved entirely client-side by TanStack Router. Colons are
  // valid in hash-fragment path segments and must NOT be percent-encoded:
  // TanStack Router's param extractor receives the raw decoded segment, and
  // %3A would be passed through literally (as the string "30617%3A…") rather
  // than decoded to "30617:…", causing the project lookup to fail.
  await page.goto(
    `/#/projects/${RELAY_TOOLS_ADDRESS}?pullRequestId=${KNOWN_PR_ID}`,
    { waitUntil: "domcontentloaded" },
  );

  // The repository picker must show "relay-tools" (non-primary), not "buzz" (primary).
  const picker = page.getByTestId("project-repository-picker");
  await expect(picker).toContainText("relay-tools", { timeout: 10_000 });
  await expect(picker).not.toContainText("buzz");

  // The PR detail panel must render from the relay-tools repository — not blank.
  // Use `first()` to avoid Playwright strict-mode violations: the text appears
  // in both the breadcrumb and the PR title heading once the detail panel opens.
  await expect(
    page.getByText("Entity-link test PR from relay-tools").first(),
  ).toBeVisible({ timeout: 10_000 });
});
