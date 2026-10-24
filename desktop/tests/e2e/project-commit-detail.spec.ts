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

async function openCreateProjectDialog(page: import("@playwright/test").Page) {
  await page.getByTestId("projects-section-projects").click();
  await page.getByTestId("projects-overview-create-project").click();
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
  await expect(browser).toBeHidden();
  await expect(page.getByTestId(`sidebar-project-${dtag}`)).toBeVisible();
}

async function openProjectRepository(
  page: import("@playwright/test").Page,
  repositoryId: string,
) {
  await expect(page).toHaveURL(/\/projects\//);
  const target = await page.evaluate((id) => {
    const url = new URL(window.location.href);
    url.searchParams.set("repositoryId", id);
    return `${url.pathname}${url.search}`;
  }, repositoryId);
  await page.goto(target, { waitUntil: "domcontentloaded" });
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

test("top-level project lists show metadata and overflow actions", async ({
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
    page.getByRole("heading", { level: 2, name: "Projects Activity" }),
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
  await expect(
    page.getByRole("button", { name: "Filter projects" }),
  ).toHaveCount(0);
  const projectRow = page.locator('[data-testid^="project-row-"]').first();
  const projectPositions = await trailingPositions(projectRow);
  await expect(projectRow.getByTestId("projects-row-context")).toBeVisible();
  await expect(projectRow.getByTestId("projects-row-people")).toBeVisible();

  await page.getByTestId("projects-section-repositories").click();
  await expect(
    page.getByRole("button", { name: "Filter repositories" }),
  ).toHaveCount(0);
  await expect(page.getByTestId("repository-row-buzz")).toBeVisible();
  await expect(page.getByTestId("repository-row-relay-tools")).toBeVisible();
  const repositoryRow = page.getByTestId("repository-row-buzz");
  await expect(
    repositoryRow.getByTestId("repositories-row-project"),
  ).toHaveCount(0);
  const repositoryTitle = repositoryRow.getByTestId("project-entity-title");
  const repositoryDescription = repositoryRow.getByTestId(
    "repositories-row-description",
  );
  await expect(repositoryDescription).toContainText(
    /Relay, desktop, and mobile|community platform/,
  );
  const [repositoryTitleBox, repositoryDescriptionBox] = await Promise.all([
    repositoryTitle.boundingBox(),
    repositoryDescription.boundingBox(),
  ]);
  expect(repositoryTitleBox).not.toBeNull();
  expect(repositoryDescriptionBox).not.toBeNull();
  expect(repositoryDescriptionBox?.x ?? 0).toBeGreaterThanOrEqual(
    (repositoryTitleBox?.x ?? 0) + (repositoryTitleBox?.width ?? 0),
  );
  await expect(repositoryDescription).toHaveCSS(
    "font-size",
    await repositoryTitle.evaluate(
      (element) => getComputedStyle(element).fontSize,
    ),
  );
  await expect(repositoryDescription).toHaveCSS("text-align", "left");
  const repositoryPositions = await trailingPositions(repositoryRow, {
    actionName: /More options for/,
    dateTestId: "repositories-row-date",
  });
  // Repository and project rows use different middle columns but retain the
  // same compact row height.
  expect(
    Math.abs(repositoryPositions.rowHeight - projectPositions.rowHeight),
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
  await expect(
    page.getByRole("button", { name: "Filter reviews" }),
  ).toHaveCount(0);
  await page.getByTestId("projects-overview-create-pull-request").click();
  await expect(page.getByTestId("create-pull-request-dialog")).toBeVisible();
  await expect(
    page.getByTestId("create-pull-request-repository"),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Tasks", exact: true }).click();
  await page.getByTestId("projects-overview-create-issue").click();
  await expect(page.getByTestId("create-issue-repository")).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Reviews", exact: true }).click();
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
  await expect(page.getByRole("button", { name: "Filter tasks" })).toHaveCount(
    0,
  );
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
  await expect(
    page.getByTestId("projects-overview-layout"),
  ).not.toHaveAttribute("data-project-context-detached", "true");
  await expect(page.getByTestId("projects-overview-context-rail")).toHaveCSS(
    "width",
    "0px",
  );
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

test("creating a project opens its channel conversation", async ({ page }) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await openCreateProjectDialog(page);
  await page.getByTestId("create-project-name").fill("multi-repo-demo");
  await page
    .getByTestId("create-project-description")
    .fill("A grouped project created through the desktop app.");
  await expect(page.getByTestId("create-project-listing")).toHaveText("Listed");
  await expect(page.getByTestId("create-project-template")).toHaveText(
    "Project home",
  );
  await expect(page.getByTestId("create-project-team")).toHaveText("None");
  await expect(page.getByTestId("create-project-agent")).toHaveText("None");
  await page.getByTestId("create-project-submit").click();

  await expect(page.getByTestId("create-project-dialog")).toBeHidden();
  await expect(page.getByTestId("project-channel-home")).toBeVisible();
  await expect(page.getByTestId("project-breadcrumb-project")).toHaveText(
    "multi-repo-demo",
  );
  await expect(page.getByTestId("chat-title")).toHaveText("multi-repo-demo");
  await expect(page.getByTestId("project-agent-chat-panel")).toHaveCount(0);
  await expect(page.getByTestId("message-channel-intro")).toBeVisible();
  await expect(page.getByTestId("message-channel-intro")).not.toContainText(
    "This is the beginning",
  );
  await expect(
    page
      .getByTestId("message-channel-intro-icon")
      .getByTestId("project-channel-icon"),
  ).toBeVisible();
  await expect(
    page.getByTestId("chat-header").getByTestId("project-channel-icon"),
  ).toBeVisible();
  await expect(
    page
      .getByTestId("channel-multi-repo-demo")
      .getByTestId("project-channel-icon"),
  ).toBeVisible();
  await expect(
    page.getByTestId("channel-intro-action-add-files"),
  ).toBeVisible();
  await expect(
    page.getByTestId("channel-intro-action-add-files-title"),
  ).toHaveText("Add files");
  await page.getByTestId("channel-intro-action-add-files").click();
  await expect(page.getByTestId("add-project-repository-dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("add-project-repository-dialog")).toBeHidden();
  await expect(page.getByTestId("project-home-summary-column")).toBeVisible();
  await expect(
    page
      .getByTestId("project-home-summary-column")
      .getByRole("heading", { name: "Overview", exact: true }),
  ).toHaveCount(0);
  await expect(
    page
      .getByTestId("project-home-summary-column")
      .getByTestId("auxiliary-panel-close"),
  ).toHaveCount(0);
  await expect(page.getByTestId("project-channel-home")).toHaveAttribute(
    "data-project-context-detached",
    "true",
  );
  await expect(page.getByTestId("project-home-summary-rail-panel")).toHaveCSS(
    "border-radius",
    "0px",
  );
  await expect(page.getByTestId("project-home-context-panel")).toBeVisible();
  await expect(page.getByTestId("project-home-context-about")).toHaveCount(0);
  await expect(
    page.getByTestId("project-home-context-home-channel"),
  ).toContainText("multi-repo-demo");
  await expect(
    page
      .getByTestId("project-home-context-home-channel")
      .getByTestId("project-channel-icon"),
  ).toBeVisible();
  await expect(
    page.getByTestId("project-home-context-home-channel"),
  ).not.toContainText("#multi-repo-demo");
  await expect(
    page.getByTestId("project-home-context-channel"),
  ).not.toContainText("people in this channel");
  const channelSection = page.getByTestId("project-home-context-channel");
  const channelSectionToggle = channelSection.getByRole("button", {
    name: "Channels",
    exact: true,
  });
  await channelSectionToggle.click();
  await expect(
    channelSection.getByTestId("project-home-context-home-channel"),
  ).toHaveCount(0);
  await channelSectionToggle.click();
  await expect(
    channelSection.getByTestId("project-home-context-home-channel"),
  ).toBeVisible();
  const codebaseSection = page.getByTestId("project-home-context-codebase");
  const codebaseSectionToggle = codebaseSection.getByRole("button", {
    name: "Codebase",
    exact: true,
  });
  await codebaseSectionToggle.click();
  await expect(
    codebaseSection.getByTestId("project-home-context-repo-multi-repo-demo"),
  ).toHaveCount(0);
  await codebaseSectionToggle.click();
  const channelAction = page.getByTestId("add-project-channel").locator("..");
  const repositoryAction = page
    .getByTestId("add-project-repository")
    .locator("..");
  await page.evaluate(() => {
    if (document.activeElement instanceof HTMLElement) {
      document.activeElement.blur();
    }
  });
  await page.mouse.move(1, 1);
  await expect(channelAction).toHaveCSS("opacity", "0");
  await expect(repositoryAction).toHaveCSS("opacity", "0");
  await page.getByTestId("project-home-context-channel").hover();
  await expect(channelAction).toHaveCSS("opacity", "1");
  await page.getByTestId("project-home-context-codebase").hover();
  await expect(repositoryAction).toHaveCSS("opacity", "1");
  await page.getByTestId("project-home-context-channel").hover();
  await page.getByTestId("add-project-channel").click();
  await expect(page.getByTestId("create-project-channel-dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("create-project-channel-dialog")).toBeHidden();
  await expect(
    page.getByTestId("sidebar-project-multi-repo-demo"),
  ).toBeVisible();
  await expect(
    page.getByTestId("sidebar-project-home-channel-multi-repo-demo"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("sidebar-project-expand-multi-repo-demo"),
  ).toHaveCount(0);
  await expect(page.getByTestId("project-home-context-codebase")).toContainText(
    "multi-repo-demo",
  );
  await expect(
    page.getByTestId("project-home-context-workspace"),
  ).toBeVisible();
  await expect(
    page
      .getByTestId("project-home-context-workspace")
      .getByRole("heading", { name: "Workspace" }),
  ).toHaveCount(0);
  await expect(page.getByTestId("project-home-context-tasks")).toBeEnabled();
  await expect(page.getByTestId("project-home-context-people")).toContainText(
    "1",
  );
  await expect(page.getByTestId("project-home-drawer-toggle")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await page.getByTestId("project-home-drawer-toggle").click();
  await expect(page.getByTestId("project-home-summary-column")).toHaveCount(0);
  await expect(page.getByTestId("project-home-drawer-toggle")).toHaveAttribute(
    "aria-pressed",
    "false",
  );
  await page.getByTestId("project-home-drawer-toggle").click();
  await expect(page.getByTestId("project-home-summary-column")).toBeVisible();
  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-management-type")).toContainText(
    "Project",
  );
  await page
    .getByTestId("channel-management-sheet")
    .getByTestId("auxiliary-panel-close")
    .click();
  await expect(page.getByTestId("channel-management-sheet")).toHaveCount(0);
  await page.getByTestId("project-home-context-files").click();
  await expect(page.getByTestId("project-home-workspace-sheet")).toBeVisible();
  await expect(
    page.getByTestId("project-home-workspace-sheet"),
  ).toHaveAttribute("data-tab", "files");
  await expect(page.getByTestId("focus-thread-drawer")).toBeVisible();
  await expect(page.getByTestId("project-home-summary-column")).toHaveCount(0);
  await page
    .getByTestId("focus-thread-drawer")
    .getByTestId("auxiliary-panel-close")
    .click();
  await expect(page.getByTestId("project-home-workspace-sheet")).toHaveCount(0);
  await expect(page.getByTestId("project-home-summary-column")).toBeVisible();
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("project-channel-home")).toHaveCount(0);
  await page.getByTestId("channel-multi-repo-demo").click();
  await expect(page.getByTestId("project-channel-home")).toBeVisible();
  await expect(page.getByTestId("project-home-summary-column")).toBeVisible();

  const createdEvents = await page.evaluate(
    () =>
      window.__BUZZ_E2E_ACCEPTED_PROJECT_EVENTS__?.filter((event) =>
        event.tags.some(
          (tag) => tag[0] === "d" && tag[1] === "multi-repo-demo",
        ),
      ) ?? [],
  );
  expect(createdEvents.map((event) => event.kind)).toEqual([30621, 30617]);
  const projectEvent = createdEvents.find((event) => event.kind === 30621);
  expect(projectEvent?.content).toBe("");
  expect(projectEvent?.tags.some((tag) => tag[0] === "a")).toBe(true);
  expect(
    projectEvent?.tags.find((tag) => tag[0] === "buzz-channel")?.[1],
  ).toMatch(
    /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i,
  );

  await page
    .getByTestId("project-detail-chrome")
    .getByRole("button", { name: "Projects" })
    .click();
  await openCreateProjectDialog(page);
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

test("unsupported relays cannot create a channel-first project", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    window.__BUZZ_E2E_UNSUPPORTED_PROJECT_ANNOUNCEMENTS__ = true;
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await openCreateProjectDialog(page);
  await page.getByTestId("create-project-name").fill("legacy-fallback");
  await page.getByTestId("create-project-submit").click();

  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  await expect(
    page.getByText("This relay does not support projects yet"),
  ).toBeVisible();

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
  expect(acceptedKinds).toEqual([]);
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
  await openCreateProjectDialog(page);
  await page.getByTestId("create-project-name").fill("retry-project");
  await page.getByTestId("create-project-submit").click();

  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  await expect(page.getByText("mock project event rejection")).toBeVisible();

  await page.getByTestId("create-project-submit").click();
  await expect(page.getByTestId("create-project-dialog")).toBeHidden();
  await expect(page.getByTestId("project-channel-home")).toBeVisible();
  await expect(page.getByTestId("project-breadcrumb-project")).toHaveText(
    "retry-project",
  );
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
  await openCreateProjectDialog(page);
  await page.getByTestId("create-project-name").fill("lost-ack-project");
  await page.getByTestId("create-project-submit").click();

  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  await expect(
    page.getByText("mock lost project acknowledgement"),
  ).toBeVisible();

  await page.getByTestId("create-project-submit").click();
  await expect(page.getByTestId("create-project-dialog")).toBeHidden();
  await expect(page.getByTestId("project-channel-home")).toBeVisible();
  await expect(page.getByTestId("project-breadcrumb-project")).toHaveText(
    "lost-ack-project",
  );
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

test("project sidebar rows open the home channel and nest extra channels", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await addProjectToSidebar(page, "buzz");

  const projectRow = page.getByTestId("sidebar-project-buzz");
  const expand = page.getByTestId("sidebar-project-expand-buzz");
  const nestedChannel = page.getByTestId("sidebar-project-channel-buzz-random");
  await expect(page).toHaveURL(/\/projects\//);
  await expect(page.getByTestId("project-channel-home")).toBeVisible();
  await expect(projectRow).toHaveAttribute("data-active", "true");
  await expect(expand).toHaveAttribute("aria-expanded", "false");
  await expect(nestedChannel).toBeHidden();
  await expect(page.getByTestId("sidebar-project-repository-buzz")).toHaveCount(
    0,
  );
  await expect(
    page.getByTestId("sidebar-project-home-channel-buzz"),
  ).toHaveCount(0);

  await expand.click();
  await expect(expand).toHaveAttribute("aria-expanded", "true");
  await expect(nestedChannel).toBeVisible();
  await expect(page).toHaveURL(/\/projects\//);

  await page.reload({ waitUntil: "domcontentloaded" });
  await addProjectToSidebar(page, "buzz");
  await expect(expand).toHaveAttribute("aria-expanded", "true");
  await expect(nestedChannel).toBeVisible();

  await nestedChannel.click();
  await expect(page).toHaveURL(/\/channels\//);
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(nestedChannel).toHaveAttribute("data-active", "true");
  await expect(projectRow).toHaveAttribute("data-active", "false");

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
  await expand.click();
  await expect(expand).toHaveAttribute("aria-expanded", "false");
  await expect(nestedChannel).toBeHidden();
  await expect(page).toHaveURL(/\/channels\//);

  await expand.click();
  await expect(expand).toHaveAttribute("aria-expanded", "true");
  await expect(nestedChannel).toBeVisible();
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
  await expect(page).toHaveURL(/\/projects\//);
  await expect(projectRow).toHaveAttribute("data-active", "true");
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/04-multi-repository-picker.png`,
  });

  await openProjectRepository(
    page,
    `${TEST_IDENTITIES.alice.pubkey}:relay-tools`,
  );
  await expect(page).toHaveURL(
    new RegExp(`repositoryId=${TEST_IDENTITIES.alice.pubkey}%3Arelay-tools`),
  );

  await page.getByTestId("sidebar-project-buzz").click();
  await expect(page.getByTestId("project-home-context-panel")).toBeVisible();
  await page.getByTestId("add-project-repository").click();
  await expect(page.getByTestId("attach-project-repository")).toBeVisible();
  await page.getByTestId("create-project-repository").click();
  await page.getByTestId("add-project-repository-name").fill("mobile-app");
  await page.getByTestId("add-project-repository-submit").click();
  await expect(page.getByTestId("add-project-repository-dialog")).toBeHidden();
  await expect(
    page.getByTestId("project-home-context-repo-mobile-app"),
  ).toBeVisible();
  await expect(
    page.getByTestId("sidebar-project-repository-mobile-app"),
  ).toHaveCount(0);
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
  ).toContainEqual(["buzz-channel", "cf63feec-21bb-5bf0-a2f8-0e4c3de8ec73"]);

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
    page.getByTestId("project-home-context-repo-design-system"),
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

test("latest files commit opens its detail without a divider", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
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
  await page.getByTestId("project-home-context-repo-buzz").click();
  await page.getByRole("tab", { name: "Files" }).click();

  const latestCommit = page.getByTestId("project-repository-latest-commit");
  await expect(latestCommit).toBeVisible();
  await expect(latestCommit).toHaveCSS("border-bottom-width", "0px");
  await expect(
    page.getByTestId("project-repository-latest-commit-summary"),
  ).toHaveCSS("font-size", "12px");
  await expect(
    page.getByTestId("project-repository-entry-row").first(),
  ).toHaveCSS("font-size", "12px");
  const repositoryEntryRow = page
    .getByTestId("project-repository-entry-row")
    .first();
  const repositoryEntryCells = repositoryEntryRow.locator("td");
  await expect(repositoryEntryCells.first()).toHaveCSS("border-radius", "0px");
  await repositoryEntryRow.hover();
  await expect(repositoryEntryCells.first()).toHaveCSS(
    "border-top-left-radius",
    "8px",
  );
  await expect(repositoryEntryCells.last()).toHaveCSS(
    "border-top-right-radius",
    "8px",
  );
  await latestCommit.click();
  await expect(page.getByTestId("project-commit-detail")).toBeVisible();
});

test("project workspace sheet stays independent from an open thread", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await openCreateProjectDialog(page);
  await page.getByTestId("create-project-name").fill("sheet-motion-demo");
  await page.getByTestId("create-project-submit").click();
  await expect(page.getByTestId("project-channel-home")).toBeVisible();
  await page.setViewportSize({ height: 720, width: 820 });

  const summaryColumn = page.getByTestId("project-home-summary-column");
  const resizeHandle = summaryColumn.getByTestId(
    "right-auxiliary-pane-resize-handle",
  );
  await expect(resizeHandle).toBeVisible();
  await resizeHandle.hover();
  await expect(
    resizeHandle.getByTestId("right-auxiliary-pane-resize-indicator"),
  ).toHaveCount(0);
  const summaryRail = page.getByTestId("project-home-summary-rail");
  const openRailWidth = await summaryRail.evaluate(
    (element) => element.getBoundingClientRect().width,
  );
  expect(openRailWidth).toBeGreaterThan(0);
  await page.getByTestId("project-home-context-tasks").click();
  await expect(page.getByTestId("project-home-workspace-sheet")).toBeVisible();

  const collapsedRailWidth = await summaryRail.evaluate(
    (element) => element.getBoundingClientRect().width,
  );
  expect(collapsedRailWidth).toBeLessThanOrEqual(1);
  const focusDrawer = page.getByTestId("focus-thread-drawer");
  const enteringDrawerWidth = (await focusDrawer.boundingBox())?.width ?? 0;
  expect(enteringDrawerWidth).toBeGreaterThan(0);
  await waitForAnimations(page);
  const settledDrawerWidth = (await focusDrawer.boundingBox())?.width ?? 0;
  expect(
    Math.abs(settledDrawerWidth - enteringDrawerWidth),
  ).toBeLessThanOrEqual(1);

  await focusDrawer.getByTestId("auxiliary-panel-close").click();
  await expect(page.getByTestId("project-home-workspace-sheet")).toHaveCount(0);
  await expect(page.getByTestId("project-home-summary-column")).toBeVisible();
  await waitForMockLiveSubscription(page, "sheet-motion-demo");
  const threadRootContent = "Workspace drawer thread root";
  await page.evaluate((content) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "sheet-motion-demo",
      content,
    });
  }, threadRootContent);
  const threadRoot = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .filter({ hasText: threadRootContent });
  await expect(threadRoot).toBeVisible();
  await threadRoot.hover();
  await threadRoot.getByRole("button", { name: "Reply" }).click();
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();

  await page.getByTestId("project-home-context-tasks").click();
  await expect(page.getByTestId("project-home-workspace-sheet")).toBeVisible();
  const workspaceDrawer = page.getByTestId("focus-thread-drawer");
  await expect(workspaceDrawer).toHaveCount(1);
  await expect(
    workspaceDrawer.getByTestId("project-home-workspace-sheet"),
  ).toBeVisible();
  await expect(page.getByTestId("message-thread-panel")).toHaveCount(1);
  await expect(workspaceDrawer.getByTestId("message-thread-panel")).toHaveCount(
    0,
  );
  const coveredThreadSurface = page.getByTestId("thread-surface");
  await expect(coveredThreadSurface).toHaveAttribute("inert", "");
  await expect(coveredThreadSurface).toHaveAttribute("aria-hidden", "true");
  const coveredSnapshot = await page.locator("body").ariaSnapshot();
  expect(coveredSnapshot).not.toContain(threadRootContent);
  expect(coveredSnapshot).toContain("Tasks");
  expect(coveredSnapshot).toContain("Close panel");

  const workspaceClose = workspaceDrawer.getByTestId("auxiliary-panel-close");
  await workspaceClose.focus();
  await expect(workspaceClose).toBeFocused();
  for (let index = 0; index < 8; index += 1) {
    await page.keyboard.press("Tab");
    expect(
      await page.evaluate(
        () =>
          document.activeElement?.closest('[data-testid="thread-surface"]') !==
          null,
      ),
    ).toBe(false);
  }

  await workspaceClose.click();
  const exitingWorkspaceState = await page.evaluate(() => {
    const workspaceSheet = document.querySelector(
      '[data-testid="project-home-workspace-sheet"]',
    );
    const threadSurface = document.querySelector(
      '[data-testid="thread-surface"]',
    );
    return {
      threadAriaHidden: threadSurface?.getAttribute("aria-hidden"),
      threadInert: threadSurface?.hasAttribute("inert") ?? false,
      workspaceSheetMounted: workspaceSheet !== null,
    };
  });
  expect(exitingWorkspaceState).toEqual({
    threadAriaHidden: "true",
    threadInert: true,
    workspaceSheetMounted: true,
  });
  await expect(page.getByTestId("project-home-workspace-sheet")).toHaveCount(0);
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
  const threadClose = coveredThreadSurface.getByTestId("auxiliary-panel-close");
  await expect(threadClose).toBeFocused();
  await expect(coveredThreadSurface).not.toHaveAttribute("inert", "");
  await expect(coveredThreadSurface).not.toHaveAttribute("aria-hidden", "true");

  await page.evaluate(() => {
    document.documentElement.style.fontSize = "140%";
  });
  await page.getByTestId("project-home-context-tasks").click();
  await expect(page.getByTestId("project-home-workspace-sheet")).toBeVisible();
  const enlargedTextWorkspaceClose = page
    .getByTestId("focus-thread-drawer")
    .getByTestId("auxiliary-panel-close");
  await expect
    .poll(() =>
      enlargedTextWorkspaceClose.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        return {
          leftInsideViewport: bounds.left >= 0,
          rightInsideViewport: bounds.right <= window.innerWidth,
          viewportWidth: window.innerWidth,
        };
      }),
    )
    .toEqual({
      leftInsideViewport: true,
      rightInsideViewport: true,
      viewportWidth: 820,
    });
  await enlargedTextWorkspaceClose.click();
  await expect(page.getByTestId("project-home-workspace-sheet")).toHaveCount(0);
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
  await page.evaluate(() => {
    document.documentElement.style.removeProperty("font-size");
  });

  await page.setViewportSize({ height: 1080, width: 1920 });
  const splitThreadPane = page
    .locator(
      '[data-testid="message-thread-panel"]:has([data-testid="right-auxiliary-pane-resize-handle"])',
    )
    .first();
  await expect(splitThreadPane).toBeVisible();
  const threadResizeHandle = splitThreadPane.getByTestId(
    "right-auxiliary-pane-resize-handle",
  );
  const threadResizeHandleBox = await threadResizeHandle.boundingBox();
  expect(threadResizeHandleBox).not.toBeNull();

  await page.getByTestId("project-home-context-tasks").click();
  await expect(page.getByTestId("project-home-workspace-sheet")).toBeVisible();
  await waitForAnimations(page);
  const workspaceCoversThreadDivider = await page.evaluate(
    ({ x, y }) => {
      return Boolean(
        document
          .elementFromPoint(x, y)
          ?.closest('[data-testid="focus-thread-drawer"]'),
      );
    },
    {
      x:
        (threadResizeHandleBox?.x ?? 0) +
        (threadResizeHandleBox?.width ?? 0) / 2,
      y: (threadResizeHandleBox?.y ?? 0) + 40,
    },
  );
  expect(workspaceCoversThreadDivider).toBe(true);
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("project-home-workspace-sheet")).toHaveCount(0);
  await expect(splitThreadPane).toBeVisible();
  await expect(threadClose).toBeFocused();
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
  await page.getByTestId("project-home-context-repo-buzz").click();
  await page.getByTestId("project-workspace-back").click();
  await page.getByTestId("project-home-context-tasks").click();
  await expect(page.getByTestId("project-home-workspace-sheet")).toBeVisible();
  const focusDrawer = page.getByTestId("focus-thread-drawer");
  await expect(focusDrawer).toHaveCount(1);
  await expect(focusDrawer).toHaveCSS("outline-style", "none");
  const workspaceSheet = page.getByTestId("project-home-workspace-sheet");
  await expect(workspaceSheet).toHaveAttribute("data-tab", "issues");
  await expect(
    page.getByTestId("project-home-workspace-sheet-expand"),
  ).toBeVisible();
  await expect(
    workspaceSheet.getByTestId("project-section-header"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("project-home-workspace-sheet-create"),
  ).toBeVisible();
  const sheetGroup = workspaceSheet
    .getByTestId("project-work-item-group-header")
    .first();
  const sheetRow = workspaceSheet.getByTestId("project-issue-row").first();
  await expect(sheetGroup).toBeVisible();
  await expect(sheetRow).toBeVisible();
  const [sheetGroupBox, sheetGroupIconBox, sheetRowBox, sheetRowIconBox] =
    await Promise.all([
      sheetGroup.boundingBox(),
      sheetGroup.getByTestId("project-group-icon").boundingBox(),
      sheetRow.boundingBox(),
      sheetRow.getByTestId("project-work-item-status-icon").boundingBox(),
    ]);
  expect(sheetGroupBox).not.toBeNull();
  expect(
    Math.abs((sheetGroupIconBox?.x ?? 0) - (sheetRowIconBox?.x ?? 0)),
  ).toBeLessThanOrEqual(2);
  expect(Math.round((sheetRowBox?.x ?? 0) - (sheetGroupBox?.x ?? 0))).toBe(8);
  await page.getByTestId("project-home-workspace-sheet-create").click();
  await expect(page.getByTestId("create-issue-dialog")).toBeVisible();
  await page
    .getByTestId("create-issue-dialog")
    .getByRole("button", { name: "Close" })
    .click();
  await expect(page.getByTestId("create-issue-dialog")).toHaveCount(0);
  await sheetRow.locator('[data-projects-text-priority="primary"]').click();
  await expect(
    workspaceSheet.getByTestId("project-issue-detail"),
  ).toBeVisible();
  await expect(page.getByTestId("idle-auxiliary-back")).toHaveAttribute(
    "aria-label",
    "Back to Tasks",
  );
  await expect(
    page.getByTestId("project-home-workspace-sheet-create"),
  ).toHaveCount(0);
  await page.getByTestId("idle-auxiliary-back").click();
  await expect(sheetRow).toBeVisible();
  await expect(workspaceSheet.getByTestId("project-issue-detail")).toHaveCount(
    0,
  );
  await expect(page.getByTestId("focus-thread-drawer")).toBeVisible();
  await expect(page.getByTestId("project-channel-home")).toBeVisible();
  await expect(page.getByTestId("project-home-summary-column")).toHaveCount(0);
  await expect(page.getByTestId("project-workspace-back")).toHaveCount(0);
  await page
    .getByTestId("focus-thread-drawer")
    .getByTestId("auxiliary-panel-close")
    .click();
  await expect(page.getByTestId("project-home-workspace-sheet")).toHaveCount(0);
  await expect(page.getByTestId("project-home-summary-column")).toBeVisible();
  await page.getByTestId("project-home-context-repo-buzz").click();
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await expect(page.getByTestId("project-workspace-back")).toBeVisible();
  await page.getByTestId("project-workspace-back").click();
  await expect(page.getByTestId("project-channel-home")).toBeVisible();
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await page.getByTestId("project-home-context-repo-buzz").click();
  await expect(page.getByTestId("project-workspace-back")).toBeVisible();

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
  ).toHaveClass(/text-muted-foreground\/55/);
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

  // Detail header: static descriptor, subject, and hash.
  await expect(
    page.getByRole("heading", { name: "Add Trello board workflow details" }),
  ).toBeVisible();
  const commitDetail = page.getByTestId("project-commit-detail");
  const commitHeader = commitDetail.locator("header").first();
  await expect(commitHeader).toContainText("Committed");
  await expect(commitHeader).not.toContainText("Brain");
  await expect(commitHeader.locator("img")).toHaveCount(0);
  await expect(commitDetail.locator(":scope > div").first()).toHaveCSS(
    "max-width",
    "768px",
  );
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

  // The repository segment returns to the project channel home.
  await commitRows
    .first()
    .getByRole("button", { name: /Add Trello board workflow details/ })
    .click();
  await expect(page.getByTestId("project-commit-detail")).toBeVisible();
  await page
    .getByRole("navigation", { name: "Project breadcrumb" })
    .getByTestId("project-breadcrumb-repository")
    .click();
  await expect(page.getByTestId("project-channel-home")).toBeVisible();
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  // The Projects root segment leaves the project entirely.
  await page
    .getByRole("navigation", { name: "Project breadcrumb" })
    .getByRole("button", { name: "Projects", exact: true })
    .click();
  await expect(projectEntry).toBeVisible();
});

test("project home task sheet expands into the repository Tasks view", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
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
  await page.getByTestId("project-home-context-tasks").click();
  const sheet = page.getByTestId("project-home-workspace-sheet");
  await expect(sheet).toBeVisible();
  const taskRow = sheet.getByTestId("project-issue-row").first();
  const taskTitle = await taskRow
    .locator('[data-projects-text-priority="primary"]')
    .textContent();
  const taskIdentifier = await taskRow
    .getByTestId("project-work-item-identifier")
    .textContent();
  expect(taskTitle).toBeTruthy();
  expect(taskIdentifier).toBeTruthy();
  await taskRow.locator('[data-projects-text-priority="primary"]').click();
  await expect(sheet.getByTestId("project-issue-detail")).toBeVisible();
  await page.getByTestId("project-home-workspace-sheet-expand").click();
  await expect(page.getByTestId("project-channel-home")).toHaveCount(0);
  const expandedDetail = page.getByTestId("project-issue-detail");
  await expect(expandedDetail).toContainText(taskTitle ?? "");
  await expect(expandedDetail).toContainText(taskIdentifier ?? "");
  await expect(page.getByTestId("project-detail-chrome")).toBeVisible();
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
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
  await page.getByTestId("project-home-context-repo-buzz").click();
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
  await page.getByTestId("project-home-context-repo-buzz").click();
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
  ).toHaveClass(/text-muted-foreground\/55/);
  await expect(
    page.getByTestId("project-work-item-group-header").first(),
  ).toBeVisible();
  await expect(
    prRows.first().locator("[data-projects-text-priority='primary']"),
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
  ).toHaveClass(/text-muted-foreground\/55/);
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
  // The overview must reflect the newly added repository.
  await expect(
    page.getByTestId("project-home-context-repo-lost-ack-repo"),
  ).toBeVisible();
  await expect(
    page.getByTestId("sidebar-project-repository-lost-ack-repo"),
  ).toHaveCount(0);

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
