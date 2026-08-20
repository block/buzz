import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/project-commit-detail";
const ALIGNMENT_TOLERANCE_PX = 2;
const LATEST_COMMIT_HASH = "0123456789abcdef0123456789abcdef01234567";

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

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(() =>
      page.evaluate(
        (name) =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: name,
          }) ?? false,
        channelName,
      ),
    )
    .toBe(true);
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
    page.getByRole("heading", { level: 2, name: "Projects", exact: true }),
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
  const projectRow = page.locator('[data-testid^="project-row-"]').first();
  const projectPositions = await trailingPositions(projectRow);
  await expect(projectRow.getByTestId("projects-row-context")).toBeVisible();
  await expect(projectRow.getByTestId("projects-row-people")).toBeVisible();

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
    repositoryRow.getByTestId("repositories-row-project"),
  ).toBeVisible();
  await expect(
    repositoryRow.getByTestId("repositories-row-description"),
  ).toContainText(/Relay, desktop, and mobile|community platform/);
  const repositoryPositions = await trailingPositions(repositoryRow, {
    actionName: /More options for/,
    dateTestId: "repositories-row-date",
  });
  // No summaryX comparison: repository rows carry text stats next to the bar
  // while project rows show the bar alone, so the columns differ in width by
  // design. The right-anchored date and menu still align across the lists.
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

  await page.getByRole("button", { name: "Reviews", exact: true }).click();
  await page.getByRole("button", { name: "Filter reviews" }).click();
  await expect(
    page.getByRole("menuitem", { name: "My Reviews" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByTestId("projects-create-menu").hover();
  await expect(page.getByRole("menuitem", { name: "Project" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Task" })).toBeVisible();
  await page.getByRole("menuitem", { name: "Review", exact: true }).click();
  await expect(page.getByTestId("create-pull-request-dialog")).toBeVisible();
  await expect(
    page.getByTestId("create-pull-request-repository"),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByTestId("projects-create-menu").hover();
  await page.getByRole("menuitem", { name: "Task" }).click();
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
    page.getByRole("menuitem", {
      name: /Open review|View (draft|merge|closed)/,
    }),
  ).toBeVisible();
  await page.keyboard.press("Escape");

  await page.getByRole("button", { name: "Tasks", exact: true }).click();
  await page.getByRole("button", { name: "Filter tasks" }).click();
  await expect(page.getByRole("menuitem", { name: "My Tasks" })).toBeVisible();
  await page.keyboard.press("Escape");
  const issueRow = page.locator('[data-testid^="projects-issue-row-"]').first();
  await expect(issueRow).toBeVisible();
  const issuePositions = await trailingPositions(issueRow);

  expect(
    Math.abs(pullRequestPositions.dateX - issuePositions.dateX),
  ).toBeLessThanOrEqual(ALIGNMENT_TOLERANCE_PX);
  expect(
    Math.abs(pullRequestPositions.menuX - issuePositions.menuX),
  ).toBeLessThanOrEqual(ALIGNMENT_TOLERANCE_PX);
  expect(
    Math.abs(pullRequestPositions.rowHeight - issuePositions.rowHeight),
  ).toBeLessThanOrEqual(ALIGNMENT_TOLERANCE_PX);
  await page.setViewportSize({ height: 720, width: 900 });
  await page.getByTestId("projects-section-projects").click();
  const responsiveRepositoryRow = page
    .locator('[data-testid^="project-row-"]')
    .first();
  await expect(
    responsiveRepositoryRow.getByTestId("projects-row-context"),
  ).toBeVisible();
  await expect(
    responsiveRepositoryRow.getByTestId("projects-row-people"),
  ).toBeVisible();
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
  const repositoryRow = page.getByTestId(
    "sidebar-project-repository-legacy-fallback",
  );
  await expect(repositoryRow).toBeVisible();
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
  await addProjectToSidebar(page, "buzz");

  const primaryRepository = page.getByTestId("sidebar-project-repository-buzz");
  const relayToolsRepository = page.getByTestId(
    "sidebar-project-repository-relay-tools",
  );
  const projectRow = page.getByTestId("sidebar-project-buzz");
  await expect(projectRow).toHaveAttribute("aria-expanded", "true");
  await expect(primaryRepository).toHaveAttribute("data-active", "true");
  await expect(relayToolsRepository).toBeVisible();

  await projectRow.click();
  await expect(projectRow).toHaveAttribute("aria-expanded", "false");
  await expect(relayToolsRepository).toBeHidden();

  await page.reload({ waitUntil: "domcontentloaded" });
  await addProjectToSidebar(page, "buzz");
  await expect(projectRow).toHaveAttribute("aria-expanded", "false");
  await expect(relayToolsRepository).toBeHidden();

  await projectRow.click();
  await expect(projectRow).toHaveAttribute("aria-expanded", "true");
  await expect(relayToolsRepository).toBeVisible();

  await page.getByTestId("channel-general").click();
  await expect(projectRow).toHaveAttribute("aria-expanded", "true");
  await expect(relayToolsRepository).toBeVisible();
  const sidebarScrollContent = page.getByTestId("sidebar-scroll-content");
  const channelSidebarMetrics = await sidebarScrollContent.evaluate(
    (element) => {
      const bounds = element.getBoundingClientRect();
      return {
        clientWidth: element.clientWidth,
        left: bounds.left,
        width: bounds.width,
      };
    },
  );
  await projectRow.click();
  await expect(page).not.toHaveURL(/\/projects\//);
  await expect(projectRow).toHaveAttribute("aria-expanded", "false");
  await expect(relayToolsRepository).toBeHidden();

  await projectRow.click();
  await expect(page).not.toHaveURL(/\/projects\//);
  await expect(projectRow).toHaveAttribute("aria-expanded", "true");
  await expect(relayToolsRepository).toBeVisible();
  const projectSidebarMetrics = await sidebarScrollContent.evaluate(
    (element) => {
      const bounds = element.getBoundingClientRect();
      return {
        clientWidth: element.clientWidth,
        left: bounds.left,
        width: bounds.width,
      };
    },
  );
  expect(projectSidebarMetrics).toEqual(channelSidebarMetrics);

  await projectRow.click();
  await expect(projectRow).toHaveAttribute("aria-expanded", "false");
  await page.getByTestId("channel-general").click();
  const sidebarScroller = page.locator('[data-sidebar="content"]');
  const anchoredScrollTop = await sidebarScroller.evaluate((element) => {
    element.scrollTop = Math.min(
      20,
      Math.max(0, element.scrollHeight - element.clientHeight),
    );
    return element.scrollTop;
  });
  await projectRow.click();
  await expect(page).not.toHaveURL(/\/projects\//);
  await expect(projectRow).toHaveAttribute("aria-expanded", "true");
  await expect
    .poll(() =>
      sidebarScroller.evaluate((element) => Math.round(element.scrollTop)),
    )
    .toBe(Math.round(anchoredScrollTop));
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/04-multi-repository-picker.png`,
  });

  await relayToolsRepository.click();
  await expect(relayToolsRepository).toHaveAttribute("data-active", "true");
  await expect(page).toHaveURL(
    new RegExp(`repositoryId=${TEST_IDENTITIES.alice.pubkey}%3Arelay-tools`),
  );

  await page.getByTestId("add-project-repository").click();
  await expect(page.getByTestId("attach-project-repository")).toBeVisible();
  await page.getByTestId("create-project-repository").click();
  await page.getByTestId("add-project-repository-name").fill("mobile-app");
  await page.getByTestId("add-project-repository-submit").click();
  await expect(page.getByTestId("add-project-repository-dialog")).toBeHidden();
  await expect(
    page.getByTestId("sidebar-project-repository-mobile-app"),
  ).toBeVisible();
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
  await expect(
    page.getByTestId("sidebar-project-repository-design-system"),
  ).toBeVisible();
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

  // Commits use the same compact, aligned row structure as work items.
  await expect(
    page.getByRole("heading", { name: "Commits", exact: true }),
  ).toBeVisible();
  const firstCommitRow = commitRows.first();
  expect((await firstCommitRow.boundingBox())?.height).toBeLessThanOrEqual(40);
  await expect(firstCommitRow.getByTitle(/^View commit /)).toBeVisible();
  await expect(
    firstCommitRow.getByTestId("project-commit-author"),
  ).toHaveAttribute("title", /^Committed by .+ · \d+ commits?$/);
  await expect(
    firstCommitRow.getByRole("button", { name: "Copy commit hash" }),
  ).toBeVisible();
  await expect(
    firstCommitRow.getByTestId("project-commit-row-date"),
  ).toHaveClass(/text-muted-foreground\/70/);
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

  // Detail header: resolved author, subject, and hash.
  await expect(page.getByText("Brain", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Add Trello board workflow details" }),
  ).toBeVisible();
  const commitDetail = page.getByTestId("project-commit-detail");
  await expect(commitDetail).toHaveCSS("max-width", "768px");
  await expect(
    commitDetail.getByRole("heading", {
      name: "Add Trello board workflow details",
    }),
  ).toHaveCSS("font-size", "18px");
  await expect(
    page.getByRole("button", { name: "Copy commit hash" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Copy commit link" }),
  ).toBeVisible();
  await expect(page.getByTestId("project-workspace-tab-menu")).toHaveCount(0);
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
  await expect(page.getByTestId("project-workspace-tab-menu")).toBeVisible();

  // The commits feed itself gets a grayed sub-tab crumb.
  await expect(
    page.getByRole("navigation", { name: "Project breadcrumb" }),
  ).toContainText("Commits");

  // The project-name segment goes to the project home (Overview tab).
  await commitRows
    .first()
    .getByRole("button", { name: /Add Trello board workflow details/ })
    .click();
  await expect(page.getByText("Brain", { exact: true })).toBeVisible();
  await page
    .getByRole("navigation", { name: "Project breadcrumb" })
    .getByTestId("project-breadcrumb-repository")
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

test("project discussion row opens its channel thread in context", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();
  await waitForMockLiveSubscription(page, "general");
  await page.evaluate(
    ({ author, commitHash }) => {
      const now = Math.floor(Date.now() / 1_000);
      const root = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `Context leading to ${commitHash} OR ${commitHash.slice(0, 7)}`,
        createdAt: now - 1,
        kind: 9,
        pubkey: author,
      });
      if (!root) throw new Error("mock message emitter is not installed");
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `Follow-up about ${commitHash} OR ${commitHash.slice(0, 7)}`,
        createdAt: now,
        kind: 9,
        parentEventId: root.id,
        pubkey: author,
      });
    },
    {
      author: TEST_IDENTITIES.alice.pubkey,
      commitHash: LATEST_COMMIT_HASH,
    },
  );

  await page.getByTestId("open-projects-view").click();
  await page.getByTestId("projects-section-projects").click();
  await page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first()
    .click();
  await page.getByRole("tab", { name: "Commits" }).click();
  const commitRow = page.getByTestId("project-activity-feed-item").first();
  await commitRow
    .getByRole("button", { name: /Add Trello board workflow details/ })
    .click();

  await page
    .getByRole("button", { name: "Open conversation in #general" })
    .click();
  const panel = page.getByTestId("project-conversation-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText(`Context leading to ${LATEST_COMMIT_HASH}`);
  await expect(panel).toContainText(`Follow-up about ${LATEST_COMMIT_HASH}`);
  await expect(
    page.getByRole("heading", { name: "Add Trello board workflow details" }),
  ).toBeVisible();
  await panel.getByRole("button", { name: "Close panel" }).click();
  await expect(panel).toBeHidden();
});

test("pull request and issue feeds use compact work item rows", async ({
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

  // Reviews use the compact single-line work-item row.
  await page.getByRole("tab", { name: "Review" }).click();
  const prRows = page.getByTestId("project-pull-request-row");
  await expect(prRows.first()).toBeVisible({ timeout: 10_000 });
  await expect(
    prRows.first().getByRole("button", { name: /^#/ }),
  ).toBeVisible();
  expect((await prRows.first().boundingBox())?.height).toBeLessThanOrEqual(40);
  await expect(
    prRows.first().getByTestId("project-pull-request-comments"),
  ).toHaveText("0");
  await expect(
    prRows.first().getByTestId("project-pull-request-row-date"),
  ).toHaveClass(/text-muted-foreground\/70/);
  await expect(
    page.getByTestId("project-work-item-group-header").first(),
  ).toBeVisible();
  await expect(
    prRows.first().locator("button").filter({ hasText: /.+/ }).nth(1),
  ).toHaveCSS("font-weight", "400");
  await waitForAnimations(page);
  await page.screenshot({ fullPage: false, path: `${SHOTS}/03-prs-feed.png` });

  // The inline #id opens the review detail, same as clicking the title.
  await prRows.first().getByRole("button", { name: /^#/ }).click();
  await expect(
    page.getByRole("navigation", { name: "Project breadcrumb" }),
  ).toContainText("Review");

  // Step back to the feed so the community tabs are available again.
  await page
    .getByRole("navigation", { name: "Project breadcrumb" })
    .getByRole("button", { name: "Review", exact: true })
    .click();
  await expect(prRows.first()).toBeVisible();

  // Tasks share the same compact structure.
  await page.getByRole("tab", { name: "Tasks" }).click();
  const issueRows = page.getByTestId("project-issue-row");
  await expect(issueRows.first()).toBeVisible({ timeout: 10_000 });
  await expect(
    issueRows.first().getByRole("button", { name: /^#/ }),
  ).toBeVisible();
  expect((await issueRows.first().boundingBox())?.height).toBeLessThanOrEqual(
    40,
  );
  const taskCategoryCells = issueRows.getByTestId("project-issue-row-category");
  await expect(taskCategoryCells.first()).toHaveText(
    /^(Issue|Change request|Improvement)$/,
  );
  const taskCreator = issueRows.first().getByTestId("project-issue-creator");
  const emptyAssignee = issueRows
    .first()
    .getByTestId("project-issue-assignee-placeholder");
  await expect(taskCreator).toHaveAttribute("title", /^Created by /);
  await expect(emptyAssignee).toHaveAttribute("title", "Unassigned");
  const [taskCreatorBox, emptyAssigneeBox] = await Promise.all([
    taskCreator.boundingBox(),
    emptyAssignee.boundingBox(),
  ]);
  expect(
    (emptyAssigneeBox?.x ?? 0) -
      ((taskCreatorBox?.x ?? 0) + (taskCreatorBox?.width ?? 0)),
  ).toBeLessThanOrEqual(4);
  await expect(
    issueRows.first().getByTestId("project-issue-comments"),
  ).toHaveText("0");
  await expect(
    issueRows.first().getByTestId("project-issue-row-date"),
  ).toHaveClass(/text-muted-foreground\/70/);
  const taskCategoryBoxes = await taskCategoryCells.evaluateAll((cells) =>
    cells.slice(0, 5).map((cell) => {
      const box = cell.getBoundingClientRect();
      return box.x + box.width;
    }),
  );
  expect(new Set(taskCategoryBoxes).size).toBe(1);
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
  await addProjectToSidebar(page, "buzz");

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
  await addProjectToSidebar(page, "buzz");

  await page.getByTestId("add-project-repository").click();
  await page.getByTestId("create-project-repository").click();
  await page.getByTestId("add-project-repository-name").fill("lost-ack-repo");
  await page.getByTestId("add-project-repository-submit").click();

  // The dialog should close — the operation recovered from the lost ACK.
  await expect(page.getByTestId("add-project-repository-dialog")).toBeHidden();
  // The sidebar must reflect the newly added repository.
  await expect(
    page.getByTestId("sidebar-project-repository-lost-ack-repo"),
  ).toBeVisible();

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

  // Direct navigation must not implicitly add the project to the sidebar.
  await expect(page.getByTestId("sidebar-project-buzz")).toHaveCount(0);
  // The seeded PR proves that this detail route resolved relay-tools rather
  // than falling back to the project's primary repository.
  // Use `first()` to avoid Playwright strict-mode violations: the text appears
  // in both the breadcrumb and the PR title heading once the detail panel opens.
  await expect(
    page.getByText("Entity-link test PR from relay-tools").first(),
  ).toBeVisible({ timeout: 10_000 });
  await expect(
    page.getByText("feature/entity-link-test").first(),
  ).toBeVisible();
});
