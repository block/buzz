import { expect, type Locator, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/projects-v3-screenshots";

async function expectSinglePrimaryTextColumn(row: Locator) {
  const primary = row.locator('[data-projects-text-priority="primary"]');
  const secondary = row.locator('[data-projects-text-priority="secondary"]');
  await expect(primary).toHaveCount(1);
  expect(await secondary.count()).toBeGreaterThan(0);
  const primaryColor = await primary.evaluate(
    (element) => getComputedStyle(element).color,
  );
  const secondaryColors = await secondary.evaluateAll((elements) =>
    elements.map((element) => getComputedStyle(element).color),
  );
  expect(secondaryColors.every((color) => color !== primaryColor)).toBe(true);
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

test("projects activity overview screenshot", async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("buzz-theme", "light");
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await expect(page.getByTestId("projects-page-tabs")).toBeVisible();
  await expect(page.getByTestId("projects-page-header")).toBeVisible();
  await expect(page.getByTestId("projects-activity-search")).toBeVisible();
  await expect(page.getByTestId("projects-activity-intro")).toContainText(
    "Projects Activity",
  );
  await expect(
    page.getByTestId("projects-overview-context-panel"),
  ).toBeVisible();
  await expect(
    page.getByTestId("projects-overview-create-project"),
  ).toBeVisible();
  await expect(
    page.getByTestId("projects-activity-group").first(),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/00-projects-pulse.png` });
});

test("sidebar project add flow browses before creating", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("sidebar-project-buzz")).toHaveCount(0);
  await page.getByTestId("sidebar-projects-section-label").hover();
  await page.getByTestId("sidebar-projects-create").click();

  const browser = page.getByTestId("project-browser-dialog");
  await expect(browser).toBeVisible();
  const search = browser.getByRole("searchbox", { name: "Search projects" });
  await search.fill("new workspace");
  await browser.getByTestId("project-browser-create").click();

  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  await expect(page.getByTestId("create-project-name")).toHaveValue(
    "new workspace",
  );
  await page.getByRole("button", { name: "Back to projects" }).click();
  await expect(browser).toBeVisible();

  await search.fill("buzz");
  await browser.getByTestId("project-browser-result-buzz").click();
  await expect(browser).toBeHidden();
  await expect(
    page.getByRole("navigation", { name: "Project breadcrumb" }),
  ).toContainText("buzz");
  const addedProject = page.getByTestId("sidebar-project-buzz");
  await expect(addedProject).toBeVisible();
  await addedProject.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Remove from sidebar" }).click();
  await expect(addedProject).toHaveCount(0);
});

test("restricted repositories keep event work visible and offer access help", async ({
  page,
}) => {
  await page.addInitScript((owner) => {
    window.__BUZZ_E2E_PROJECT_OWNER_OVERRIDE__ = owner;
  }, TEST_IDENTITIES.alice.pubkey);
  await installMockBridge(page, {
    projectAccessChannelId: "11111111-1111-4111-8111-111111111111",
    projectRepoSnapshotError: "remote: repository not found",
  });
  await openBuzzProject(page);

  const unavailableState = page
    .getByTestId("project-repository-unavailable")
    .first();
  await expect(unavailableState).toContainText("Repository access restricted");
  await expect(
    unavailableState.getByTestId("repository-owner-name"),
  ).toHaveText("alice");
  await expect(unavailableState).toContainText(/the repository owner/i);
  await expect(
    unavailableState
      .locator(
        '[data-testid="repository-owner-avatar-image"], [data-testid="repository-owner-avatar-fallback"]',
      )
      .first(),
  ).toBeVisible();
  await expect(page.getByTestId("project-section-header")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Ask for access" }),
  ).toHaveCount(2);
  await expect(
    page.getByRole("button", { name: "Clone", exact: true }),
  ).toHaveCount(0);
  await expect(page.getByTestId("project-repository-file-count")).toHaveText(
    "—",
  );

  await page.getByRole("tab", { name: "Tasks", exact: true }).click();
  await expect(page.getByTestId("project-issue-row").first()).toBeVisible();

  await page.getByRole("tab", { name: "Review", exact: true }).click();
  await expect(
    page.getByTestId("project-pull-request-row").first(),
  ).toBeVisible();

  await page.getByRole("tab", { name: "Files", exact: true }).click();
  await expect(page.getByText("Repository access restricted")).toBeVisible();
  await expect(page.getByTestId("project-section-header")).toHaveCount(0);

  await page.getByRole("tab", { name: "Commits", exact: true }).click();
  await expect(page.getByText("Repository access restricted")).toBeVisible();
  await page.getByRole("button", { name: "Ask for access" }).first().click();

  const chatPanel = page.getByTestId("project-agent-chat-panel");
  await expect(chatPanel).toBeVisible();
  await expect(chatPanel.getByTestId("project-agent-context")).toContainText(
    "Commits",
  );
  await expect(chatPanel.getByTestId("message-composer")).toBeVisible();
});

test("repository pages show a centered Buzz loader while fetching", async ({
  page,
}) => {
  await installMockBridge(page, { projectRepoSnapshotDelayMs: 750 });
  await openBuzzProject(page);

  const loader = page.getByTestId("buzz-loading-state");
  await expect(loader).toBeVisible();
  await expect(
    loader.getByRole("img", { name: "Loading repository" }),
  ).toBeVisible();
  const animatedMark = loader.locator(".buzz-logo__mark");
  await expect(animatedMark).toHaveCSS(
    "animation-name",
    "buzz-logo-scale-pulse",
  );
  await expect(animatedMark).toHaveCSS("opacity", "1");
  await expect(loader).toHaveCSS("justify-content", "center");
  await expect(loader).toBeHidden({ timeout: 5_000 });
});

// Walks the Projects v3 workspace through its headline states so PR
// screenshots capture distinct pixels per feature (overview box, tab-strip
// plus, issue detail with inline copy link + avatar timeline, PR detail).
test("projects v3 workspace screenshot states", async ({ page }) => {
  await installMockBridge(page);
  await openBuzzProject(page);
  const initialProjectBreadcrumb = page.getByRole("navigation", {
    name: "Project breadcrumb",
  });
  await expect(
    initialProjectBreadcrumb.getByTestId("project-breadcrumb-project"),
  ).toBeVisible();
  await expect(
    initialProjectBreadcrumb.getByTestId("project-breadcrumb-repository"),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Open Discussion" }),
  ).toHaveCount(0);

  // Borderless workspace: repository controls live in the persistent,
  // resizable auxiliary panel instead of a header row.
  const overviewTab = page.getByRole("tab", { name: "Overview" });
  const filesTab = page.getByRole("tab", { name: "Files" });
  const channelsTab = page.getByRole("tab", { name: "Channels" });
  const contributorsTab = page.getByRole("tab", { name: "Contributors" });
  const tabMenu = page.getByTestId("project-workspace-tab-menu");
  const workspacePanel = page.getByTestId("project-workspace-panel");
  const projectDetailScroll = page.getByTestId("project-detail-scroll");
  const repositoryActionsPanel = page.getByTestId(
    "project-repository-actions-panel",
  );
  const expectFullWidthSection = async () => {
    const sectionHeader = workspacePanel.getByTestId("project-section-header");
    const [workspaceBox, headerBox, menuBox] = await Promise.all([
      workspacePanel.boundingBox(),
      sectionHeader.boundingBox(),
      tabMenu.boundingBox(),
    ]);
    expect(workspaceBox).not.toBeNull();
    expect(headerBox).not.toBeNull();
    expect(menuBox).not.toBeNull();
    expect(headerBox?.x).toBe(workspaceBox?.x);
    expect(headerBox?.width).toBe(workspaceBox?.width);
    expect(workspaceBox?.x).toBe(menuBox?.x);
    expect((workspaceBox?.x ?? 0) + (workspaceBox?.width ?? 0)).toBe(
      (menuBox?.x ?? 0) + (menuBox?.width ?? 0),
    );
  };
  await expect(filesTab).toBeVisible();
  await expect(projectDetailScroll).toHaveCSS("overscroll-behavior-y", "none");
  await expect(overviewTab).toHaveAttribute("data-state", "active");
  const [projectDetailScrollBounds, initialTabMenuBounds] = await Promise.all([
    projectDetailScroll.boundingBox(),
    tabMenu.boundingBox(),
  ]);
  expect(projectDetailScrollBounds).not.toBeNull();
  expect(initialTabMenuBounds?.y).toBe(projectDetailScrollBounds?.y);
  expect(
    await filesTab.evaluate((element) =>
      Number.parseFloat(getComputedStyle(element).borderRadius),
    ),
  ).toBeGreaterThan(13);
  expect((await filesTab.boundingBox())?.height).toBe(28);
  await expect(repositoryActionsPanel).toBeVisible();
  const projectContextCard = repositoryActionsPanel.getByTestId(
    "project-context-card",
  );
  await expect(projectContextCard).toBeVisible();
  await expect(projectContextCard).toHaveCSS("border-radius", "16px");
  const repositoryHeading = repositoryActionsPanel.getByRole("heading", {
    name: "buzz",
    exact: true,
  });
  await expect(repositoryHeading).toBeVisible();
  await expect(repositoryHeading).toHaveCSS("font-size", "14px");
  await expect(
    repositoryActionsPanel.getByRole("heading", {
      name: "Actions",
      exact: true,
    }),
  ).toHaveCount(0);
  await expect(
    repositoryActionsPanel.getByRole("heading", {
      name: "Languages",
      exact: true,
    }),
  ).toHaveCount(0);
  await expect(
    repositoryActionsPanel.getByRole("heading", {
      name: "People",
      exact: true,
    }),
  ).toHaveCount(0);
  const detailsHeading = repositoryActionsPanel.getByRole("heading", {
    name: "Details",
    exact: true,
  });
  const peopleContent = repositoryActionsPanel.getByTestId(
    "project-repository-people",
  );
  const peopleSection = peopleContent.locator("..");
  const fetchButton = repositoryActionsPanel.getByRole("button", {
    name: "Fetch",
    exact: true,
  });
  await expect(peopleSection).toHaveCSS("border-top-width", "0px");
  await expect(detailsHeading.locator("..")).toHaveCSS(
    "border-top-width",
    "0px",
  );
  expect(
    await repositoryHeading.evaluate(
      (element) => getComputedStyle(element).color,
    ),
  ).toBe(
    await detailsHeading.evaluate((element) => getComputedStyle(element).color),
  );
  const actionsSection = repositoryActionsPanel
    .getByRole("button", { name: "Terminal", exact: true })
    .locator("..")
    .locator("..");
  const [actionsSectionBounds, peopleSectionBounds, detailsHeadingBounds] =
    await Promise.all([
      actionsSection.boundingBox(),
      peopleSection.boundingBox(),
      detailsHeading.boundingBox(),
    ]);
  expect(
    (peopleSectionBounds?.y ?? 0) -
      ((actionsSectionBounds?.y ?? 0) + (actionsSectionBounds?.height ?? 0)),
  ).toBe(8);
  expect((detailsHeadingBounds?.y ?? 0) - (peopleSectionBounds?.y ?? 0)).toBe(
    8,
  );
  const people = repositoryActionsPanel.getByTestId(
    "project-repository-person",
  );
  const [firstPersonBounds, secondPersonBounds] = await Promise.all([
    people.nth(0).boundingBox(),
    people.nth(1).boundingBox(),
  ]);
  expect(secondPersonBounds?.x ?? 0).toBeLessThan(
    (firstPersonBounds?.x ?? 0) + (firstPersonBounds?.width ?? 0),
  );
  await people.first().hover();
  await expect(
    page.getByTestId("project-repository-person-tooltip").first(),
  ).toBeVisible();
  await expect(fetchButton).toHaveCSS("text-align", "left");
  const sourceButton = repositoryActionsPanel.getByRole("button", {
    name: "Remote",
    exact: true,
  });
  const latestDetailRow = repositoryActionsPanel
    .getByText("Latest", { exact: true })
    .locator("..");
  const [
    alignedDetailsHeadingBounds,
    sourceButtonBounds,
    sourceSectionBounds,
    sourceTrailingIconBounds,
    fetchButtonBounds,
    fetchIconBounds,
    branchTrailingIconBounds,
    latestDetailRowBounds,
    latestDetailValueBounds,
    managementActionsBounds,
  ] = await Promise.all([
    detailsHeading.boundingBox(),
    sourceButton.boundingBox(),
    sourceButton.locator("..").locator("..").boundingBox(),
    sourceButton.locator("svg").last().boundingBox(),
    fetchButton.boundingBox(),
    fetchButton.locator("svg").first().boundingBox(),
    repositoryActionsPanel
      .getByTestId("project-repository-branch-trigger")
      .locator("svg")
      .last()
      .boundingBox(),
    latestDetailRow.boundingBox(),
    latestDetailRow.locator("dd").boundingBox(),
    repositoryActionsPanel
      .getByTestId("project-repository-management-actions")
      .boundingBox(),
  ]);
  expect(alignedDetailsHeadingBounds?.height).toBe(28);
  expect(sourceButtonBounds?.height).toBe(28);
  expect(fetchButtonBounds?.height).toBe(28);
  expect(latestDetailRowBounds?.height).toBe(28);
  expect(
    (actionsSectionBounds?.y ?? 0) -
      ((sourceSectionBounds?.y ?? 0) + (sourceSectionBounds?.height ?? 0)),
  ).toBe(2);
  expect(fetchButtonBounds?.x ?? 0).toBeLessThan(
    alignedDetailsHeadingBounds?.x ?? 0,
  );
  expect(fetchIconBounds?.x).toBe(alignedDetailsHeadingBounds?.x);
  const detailRight =
    (latestDetailValueBounds?.x ?? 0) + (latestDetailValueBounds?.width ?? 0);
  expect(
    (sourceTrailingIconBounds?.x ?? 0) + (sourceTrailingIconBounds?.width ?? 0),
  ).toBe(detailRight);
  expect(
    (branchTrailingIconBounds?.x ?? 0) + (branchTrailingIconBounds?.width ?? 0),
  ).toBe(detailRight);
  expect(
    (managementActionsBounds?.x ?? 0) + (managementActionsBounds?.width ?? 0),
  ).toBe(detailRight);
  await expect(
    repositoryActionsPanel.getByText("Working copy", { exact: true }),
  ).toHaveCount(0);
  await expect(
    repositoryActionsPanel.getByTestId("project-right-panel-scope"),
  ).toHaveCount(0);
  const sharedHeaderBackdrop = page.getByTestId(
    "project-shared-header-backdrop",
  );
  await expect(sharedHeaderBackdrop).toBeVisible();
  await expect
    .poll(() =>
      sharedHeaderBackdrop.evaluate(
        (element) => getComputedStyle(element).backdropFilter,
      ),
    )
    .not.toBe("none");
  const projectPanelLayout = page.getByTestId("project-panel-layout");
  const projectContentPod = page.getByTestId("project-content-pod");
  const appContentSurface = page
    .locator("[data-buzz-content-surface]")
    .filter({ has: projectPanelLayout })
    .first();
  await expect(projectPanelLayout).toHaveAttribute("data-detached", "true");
  await expect(appContentSurface).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await expect(appContentSurface).toHaveCSS("border-radius", "0px");
  await expect(projectContentPod).toHaveCSS("border-radius", "16px");
  await expect(projectContentPod).toHaveCSS("box-shadow", "none");
  await expect(repositoryActionsPanel).toHaveCSS("border-radius", "0px");
  await expect(repositoryActionsPanel).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  await expect(appContentSurface).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await expect(projectContentPod).toHaveCSS("border-radius", "16px");
  await expect(projectContextCard).toHaveCSS("border-radius", "16px");
  await page.evaluate(() => document.documentElement.classList.remove("dark"));
  await expect(projectContextCard).toHaveCSS("border-radius", "16px");
  await expect(projectContextCard).toHaveCSS("border-width", "0px");
  await expect(projectContextCard).toHaveCSS("box-shadow", "none");
  const [
    projectContextCardBounds,
    sharedHeaderBackdropBounds,
    projectContentPodBounds,
    repositoryActionsPanelBounds,
  ] = await Promise.all([
    projectContextCard.boundingBox(),
    sharedHeaderBackdrop.boundingBox(),
    projectContentPod.boundingBox(),
    repositoryActionsPanel.boundingBox(),
  ]);
  expect(projectContextCardBounds).not.toBeNull();
  expect(sharedHeaderBackdropBounds).not.toBeNull();
  expect(projectContentPodBounds).not.toBeNull();
  expect(repositoryActionsPanelBounds).not.toBeNull();
  expect(Math.round(repositoryActionsPanelBounds?.width ?? 0)).toBe(280);
  expect(
    await projectContextCard.evaluate(
      (element) => getComputedStyle(element).backgroundColor,
    ),
  ).toBe(
    await projectContentPod.evaluate(
      (element) => getComputedStyle(element).backgroundColor,
    ),
  );
  expect(projectContextCardBounds?.height ?? 0).toBeLessThan(
    repositoryActionsPanelBounds?.height ?? 0,
  );
  expect(projectContextCardBounds?.y).toBe(repositoryActionsPanelBounds?.y);
  expect(
    (sharedHeaderBackdropBounds?.x ?? 0) +
      (sharedHeaderBackdropBounds?.width ?? 0),
  ).toBeLessThanOrEqual(
    (projectContentPodBounds?.x ?? 0) + (projectContentPodBounds?.width ?? 0),
  );
  expect(
    (repositoryActionsPanelBounds?.x ?? 0) -
      ((projectContentPodBounds?.x ?? 0) +
        (projectContentPodBounds?.width ?? 0)),
  ).toBe(8);
  expect(
    (sharedHeaderBackdropBounds?.x ?? 0) +
      (sharedHeaderBackdropBounds?.width ?? 0),
  ).toBeLessThan(repositoryActionsPanelBounds?.x ?? 0);
  const contextRail = page.getByTestId("project-context-rail");
  const resizeHandle = repositoryActionsPanel.getByTestId(
    "right-auxiliary-pane-resize-handle",
  );
  const resizeHandleBounds = await resizeHandle.boundingBox();
  expect(resizeHandleBounds).not.toBeNull();
  const resizeStartX =
    (resizeHandleBounds?.x ?? 0) + (resizeHandleBounds?.width ?? 0) / 2;
  const resizeStartY =
    (resizeHandleBounds?.y ?? 0) + (resizeHandleBounds?.height ?? 0) / 2;
  await resizeHandle.dispatchEvent("pointerdown", {
    button: 0,
    clientX: resizeStartX,
    clientY: resizeStartY,
    pointerId: 1,
    pointerType: "mouse",
  });
  await expect(contextRail).toHaveAttribute("data-resizing", "true");
  await expect(contextRail).toHaveCSS("transition-duration", "0s");
  await page.mouse.move(resizeStartX - 40, resizeStartY);
  await expect
    .poll(async () =>
      repositoryActionsPanel.evaluate((element) =>
        Math.round(element.getBoundingClientRect().width),
      ),
    )
    .toBe(320);
  await page.mouse.up();
  await expect(contextRail).toHaveAttribute("data-resizing", "false");
  await expect(contextRail).toHaveCSS("transition-duration", "0.2s");
  await resizeHandle.dblclick();
  await expect
    .poll(async () =>
      repositoryActionsPanel.evaluate((element) =>
        Math.round(element.getBoundingClientRect().width),
      ),
    )
    .toBe(280);
  const repositoryPanelTab = page.getByTestId(
    "project-right-panel-repository-tab",
  );
  const chatPanelTab = page.getByTestId("project-right-panel-chat-tab");
  const terminalButton = page.getByTestId("project-terminal-toggle");
  const terminalIcon = page.getByTestId("project-terminal-icon");
  const repositoryContextIcon = page.getByTestId(
    "project-right-panel-repository-icon",
  );
  await expect(repositoryPanelTab).toHaveAttribute("aria-pressed", "true");
  await expect(repositoryPanelTab).toHaveAttribute(
    "aria-label",
    "Hide project context",
  );
  await expect(terminalIcon).toBeVisible();
  await expect(repositoryPanelTab).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await expect(repositoryContextIcon).toHaveCSS("opacity", "1");
  await expect(contextRail).toHaveCSS("width", "288px");
  await expect(contextRail).toHaveCSS("transition-duration", "0.2s");
  // The icon is an inline SVG drawn with currentColor, so it must inherit
  // the toggle button's text color to stay tinted with button state.
  expect(
    await terminalIcon.evaluate((element) => ({
      color: getComputedStyle(element).color,
      strokesCurrentColor: Array.from(element.querySelectorAll("rect")).some(
        (rect) =>
          rect.getAttribute("stroke") === "currentColor" ||
          rect.getAttribute("fill") === "currentColor",
      ),
      tagName: element.tagName.toLowerCase(),
    })),
  ).toMatchObject({
    color: await terminalButton.evaluate(
      (element) => getComputedStyle(element).color,
    ),
    strokesCurrentColor: true,
    tagName: "svg",
  });
  const [repositoryTabBounds, chatTabBounds, terminalTabBounds] =
    await Promise.all([
      repositoryPanelTab.boundingBox(),
      chatPanelTab.boundingBox(),
      terminalButton.boundingBox(),
    ]);
  expect(repositoryTabBounds?.width).toBe(chatTabBounds?.width);
  expect(chatTabBounds?.x).toBeLessThan(terminalTabBounds?.x ?? 0);
  expect(terminalTabBounds?.x).toBeLessThan(repositoryTabBounds?.x ?? 0);
  await chatPanelTab.click();
  const agentChatPanel = page.getByTestId("project-agent-chat-panel");
  await expect(agentChatPanel).toBeVisible();
  await expect(projectPanelLayout).toHaveAttribute("data-detached", "false");
  await expect(projectContentPod).toHaveCount(0);
  await expect(appContentSurface).toHaveCSS("border-radius", "16px");
  await expect
    .poll(() =>
      appContentSurface.evaluate(
        (element) => getComputedStyle(element).backgroundColor,
      ),
    )
    .not.toBe("rgba(0, 0, 0, 0)");
  await expect(agentChatPanel.getByTestId("message-composer")).toBeVisible();
  const agentContext = agentChatPanel.getByTestId("project-agent-context");
  await expect(agentContext).toBeVisible();
  await expect(agentContext).toContainText("Overview");
  await expect(agentContext).not.toContainText("Buzz /");
  // The context rail reveals the chat panel with a width transition; measure
  // only after it settles or the panel's unclipped box overhangs the rail.
  await waitForAnimations(page);
  const [
    attachedSharedHeaderBackdropBounds,
    tabMenuHeaderBounds,
    agentContextBounds,
  ] = await Promise.all([
    sharedHeaderBackdrop.boundingBox(),
    tabMenu.boundingBox(),
    agentContext.boundingBox(),
  ]);
  expect(attachedSharedHeaderBackdropBounds).not.toBeNull();
  expect(tabMenuHeaderBounds).not.toBeNull();
  expect(agentContextBounds).not.toBeNull();
  expect(tabMenuHeaderBounds?.height).toBe(52);
  expect(
    Math.abs((tabMenuHeaderBounds?.y ?? 0) - (agentContextBounds?.y ?? 0)),
  ).toBeLessThanOrEqual(1);
  expect(
    Math.abs(
      (tabMenuHeaderBounds?.height ?? 0) - (agentContextBounds?.height ?? 0),
    ),
  ).toBeLessThanOrEqual(1);
  expect(attachedSharedHeaderBackdropBounds?.x).toBeLessThanOrEqual(
    tabMenuHeaderBounds?.x ?? 0,
  );
  expect(
    (attachedSharedHeaderBackdropBounds?.x ?? 0) +
      (attachedSharedHeaderBackdropBounds?.width ?? 0),
  ).toBeGreaterThanOrEqual(
    (agentContextBounds?.x ?? 0) + (agentContextBounds?.width ?? 0),
  );
  await expect
    .poll(() =>
      agentContext.evaluate((element) => getComputedStyle(element).position),
    )
    .toBe("absolute");
  const agentConversationScroll = agentChatPanel.getByTestId(
    "project-agent-conversation-scroll",
  );
  await expect
    .poll(() =>
      agentConversationScroll.evaluate((element) =>
        Number.parseFloat(getComputedStyle(element).paddingTop),
      ),
    )
    .toBeGreaterThanOrEqual(52);
  await agentChatPanel
    .getByTestId("message-input")
    .fill("Summarize this repository.");
  await agentChatPanel.getByTestId("send-message").click();
  const projectAgentMessage = agentChatPanel
    .locator("[data-message-id]")
    .filter({ hasText: "Summarize this repository." });
  await expect(projectAgentMessage).toBeVisible();
  const projectAgentMessageId =
    await projectAgentMessage.getAttribute("data-message-id");
  expect(projectAgentMessageId).not.toBeNull();
  const projectAgentChannelId = await agentChatPanel
    .locator("[data-project-agent-channel-id]")
    .getAttribute("data-project-agent-channel-id");
  expect(projectAgentChannelId).not.toBeNull();
  await page.evaluate(
    async ({ channelId, parentEventId }) => {
      if (!channelId)
        throw new Error("Project agent DM channel was not recorded.");
      await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("send_channel_message", {
        channelId,
        content: "A persisted threaded agent response.",
        parentEventId: parentEventId ?? undefined,
      });
    },
    { channelId: projectAgentChannelId, parentEventId: projectAgentMessageId },
  );
  await expect(
    agentChatPanel.getByText("A persisted threaded agent response."),
  ).toBeVisible();
  await chatPanelTab.click();
  // The rail collapses but retains the panel so conversation state survives
  // toggling; assert the collapsed rail instead of a full unmount.
  await expect(contextRail).toHaveCSS("width", "0px");
  await expect(contextRail).toHaveAttribute("aria-hidden", "true");
  await expect(chatPanelTab).toHaveAttribute("aria-label", "Show project chat");
  await chatPanelTab.click();
  await expect(
    agentChatPanel.getByText("A persisted threaded agent response."),
  ).toBeVisible();
  await expect(chatPanelTab).toHaveAttribute("aria-label", "Hide project chat");
  await expect(repositoryActionsPanel).toHaveCount(0);
  await page.getByTestId("project-right-panel-repository-tab").click();
  await expect(repositoryActionsPanel).toBeVisible();
  await expect(
    page.getByTestId("project-repository-selection-row"),
  ).toHaveCount(0);
  await repositoryPanelTab.click();
  await expect(contextRail).toHaveCSS("width", "0px");
  await expect(repositoryContextIcon).toHaveCSS("opacity", "0.6");
  await expect(repositoryPanelTab).toHaveAttribute(
    "aria-label",
    "Show project context",
  );
  const [attachedContentSurfaceBounds, collapsedMainPaneBounds] =
    await Promise.all([
      appContentSurface.boundingBox(),
      workspacePanel.boundingBox(),
    ]);
  expect(attachedContentSurfaceBounds).not.toBeNull();
  await expect(appContentSurface).toHaveCSS("box-shadow", "none");
  // The detached pod fills the surface minus its hairline top/left inset and
  // the 8px bottom gutter (ml-px mt-px mb-2 on the pod wrapper).
  expect(
    (projectContentPodBounds?.x ?? 0) - (attachedContentSurfaceBounds?.x ?? 0),
  ).toBe(1);
  expect(
    (projectContentPodBounds?.y ?? 0) - (attachedContentSurfaceBounds?.y ?? 0),
  ).toBe(1);
  expect(
    (attachedContentSurfaceBounds?.height ?? 0) -
      (projectContentPodBounds?.height ?? 0),
  ).toBe(9);
  const viewportSize = page.viewportSize();
  expect(collapsedMainPaneBounds).not.toBeNull();
  expect(viewportSize).not.toBeNull();
  expect(
    Math.abs(
      (collapsedMainPaneBounds?.x ?? 0) +
        (collapsedMainPaneBounds?.width ?? 0) -
        (viewportSize?.width ?? 0),
    ),
  ).toBeLessThanOrEqual(8);
  await repositoryPanelTab.click();
  await expect(contextRail).toHaveCSS("width", "288px");
  await expect(repositoryContextIcon).toHaveCSS("opacity", "1");
  await expect(repositoryActionsPanel).toBeVisible();
  await expect(repositoryPanelTab).toHaveAttribute(
    "aria-label",
    "Hide project context",
  );
  await expect(page.getByTestId("project-detail-copy-link")).toBeVisible();
  await expect(
    repositoryActionsPanel.getByTestId("project-detail-copy-link"),
  ).toHaveCount(0);
  await expect(
    workspacePanel.getByTestId("project-workspace-tab-menu"),
  ).toHaveCount(0);
  const overviewBounds = await overviewTab.boundingBox();
  const channelsBounds = await channelsTab.boundingBox();
  const contributorsBounds = await contributorsTab.boundingBox();
  const tabMenuBounds = await tabMenu.boundingBox();
  const workspaceBounds = await workspacePanel.boundingBox();
  const repositoryActionsBounds = await repositoryActionsPanel.boundingBox();
  expect(overviewBounds).not.toBeNull();
  expect(channelsBounds).not.toBeNull();
  expect(contributorsBounds).not.toBeNull();
  expect(tabMenuBounds).not.toBeNull();
  expect(workspaceBounds).not.toBeNull();
  expect(repositoryActionsBounds).not.toBeNull();
  expect(channelsBounds?.x).toBeLessThan(contributorsBounds?.x ?? 0);
  expect(overviewBounds?.x).toBeGreaterThan(workspaceBounds?.x ?? 0);
  expect(tabMenuBounds?.y).toBeLessThan(workspaceBounds?.y ?? 0);
  expect(repositoryActionsBounds?.x).toBeGreaterThan(workspaceBounds?.x ?? 0);
  await expect(
    workspacePanel.getByRole("heading", { name: "README", exact: true }),
  ).toHaveCount(0);
  await expect(
    workspacePanel.getByTestId("project-section-header-icon"),
  ).toHaveCount(0);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/01-workspace-overview.png` });

  await repositoryActionsPanel
    .getByRole("button", { name: "Clone", exact: true })
    .click();
  const localSourceTrigger = repositoryActionsPanel.getByRole("button", {
    name: /^Local /,
  });
  await expect(localSourceTrigger).toBeVisible();
  await expect(
    repositoryActionsPanel.getByTestId("project-repository-local-path"),
  ).toHaveText("…/buzz/REPOS/buzz");
  await expect(
    repositoryActionsPanel.getByRole("button", {
      name: "Open",
      exact: true,
    }),
  ).toHaveAttribute("title", "Open local repository folder");
  await localSourceTrigger.click();
  await page
    .getByRole("menuitemradio", { name: "Remote", exact: true })
    .click();
  await expect(
    repositoryActionsPanel.getByRole("button", {
      name: "Remote",
      exact: true,
    }),
  ).toBeVisible();

  await filesTab.click();
  await expect(repositoryActionsPanel).toBeVisible();
  await expect(
    workspacePanel.getByRole("heading", { name: "Files", exact: true }),
  ).toBeVisible();
  await expect(
    workspacePanel.getByTestId("project-repository-entry-icon").first(),
  ).toHaveCSS("border-radius", "8px");
  await chatPanelTab.click();
  await expect(agentContext).toContainText("Files");
  await expect(
    agentChatPanel.getByText("A persisted threaded agent response."),
  ).toBeVisible();
  await repositoryPanelTab.click();
  await expect(repositoryActionsPanel).toBeVisible();
  await expectFullWidthSection();
  await page.getByRole("tab", { name: "Commits" }).click();
  await expect(repositoryActionsPanel).toBeVisible();
  await expect(
    workspacePanel.getByRole("heading", { name: "Commits", exact: true }),
  ).toBeVisible();
  await expectFullWidthSection();
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
  await expectFullWidthSection();
  const contributorRow = page.getByTestId("project-contributor-row").first();
  await expect(
    contributorRow.getByTestId("project-contributor-commit-count"),
  ).toBeVisible();
  await expect(
    contributorRow.getByTestId("project-contributor-review-count"),
  ).toBeVisible();
  await expect(
    contributorRow.getByTestId("project-contributor-task-count"),
  ).toBeVisible();
  const contributorBounds = await contributorRow.boundingBox();
  const contributorIdentityBounds = await contributorRow
    .getByTestId("project-contributor-identity")
    .boundingBox();
  const contributorCommitCountBounds = await contributorRow
    .getByTestId("project-contributor-commit-count")
    .boundingBox();
  expect(contributorBounds).not.toBeNull();
  expect(contributorBounds?.height).toBeLessThanOrEqual(40);
  expect(contributorIdentityBounds).not.toBeNull();
  expect(contributorCommitCountBounds?.x).toBeGreaterThan(
    contributorIdentityBounds?.x ?? 0,
  );
  const contributorCommitCounts = await page
    .getByTestId("project-contributor-commit-count")
    .evaluateAll((nodes) =>
      nodes.map((node) => {
        const value = Number.parseInt(node.textContent?.trim() ?? "", 10);
        return Number.isNaN(value) ? -1 : value;
      }),
    );
  expect(contributorCommitCounts).toEqual(
    [...contributorCommitCounts].sort((left, right) => right - left),
  );

  await channelsTab.click();
  await expect(
    workspacePanel.getByRole("heading", { name: "Channels", exact: true }),
  ).toBeVisible();
  await expect(
    repositoryActionsPanel.getByTestId("project-right-panel-scope"),
  ).toHaveCount(0);
  await expect(
    repositoryActionsPanel.getByText("Working copy", { exact: true }),
  ).toHaveCount(0);
  await expect(
    repositoryActionsPanel.getByRole("heading", {
      name: "Actions",
      exact: true,
    }),
  ).toHaveCount(0);
  for (const action of ["Clone", "Fetch", "Terminal"]) {
    await expect(
      repositoryActionsPanel.getByRole("button", {
        name: action,
        exact: true,
      }),
    ).toHaveCount(0);
  }
  await expectFullWidthSection();

  // Issues tab: the create action lives in the section header.
  await page.getByRole("tab", { name: "Tasks", exact: true }).click();
  await expect(repositoryActionsPanel).toBeVisible();
  await expect(
    repositoryActionsPanel
      .getByTestId("project-repository-people")
      .locator(".."),
  ).toHaveCSS("border-top-width", "0px");
  await expect(
    repositoryActionsPanel.getByRole("heading", {
      name: "Task activity",
      exact: true,
    }),
  ).toHaveCount(0);
  const taskDetailsHeading = repositoryActionsPanel.getByRole("heading", {
    name: "Details",
    exact: true,
  });
  await expect(taskDetailsHeading).toBeVisible();
  const [taskDetailsBounds, taskPeopleBounds] = await Promise.all([
    taskDetailsHeading.boundingBox(),
    repositoryActionsPanel
      .getByTestId("project-repository-people")
      .boundingBox(),
  ]);
  expect(taskDetailsBounds?.y ?? 0).toBeLessThan(taskPeopleBounds?.y ?? 0);
  await expectFullWidthSection();
  const newIssueButton = workspacePanel.getByRole("button", {
    name: "Create task",
  });
  await expect(newIssueButton).toBeVisible();
  const contextCreateTaskButton = repositoryActionsPanel.getByRole("button", {
    name: "Create task",
    exact: true,
  });
  await expect(contextCreateTaskButton).toBeVisible();
  await contextCreateTaskButton.click();
  await expect(page.getByTestId("create-issue-dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(
    tabMenu.getByRole("button", { name: "Create task" }),
  ).toHaveCount(0);
  const issuesHeading = workspacePanel.getByRole("heading", {
    name: "Tasks",
    exact: true,
  });
  const issuesHeadingBounds = await issuesHeading.boundingBox();
  const newIssueBounds = await newIssueButton.boundingBox();
  expect(issuesHeadingBounds).not.toBeNull();
  expect(newIssueBounds).not.toBeNull();
  expect(newIssueBounds?.x).toBeGreaterThan(issuesHeadingBounds?.x ?? 0);
  const issueRow = page.getByTestId("project-issue-row").first();
  await expect(issueRow).toBeVisible({ timeout: 10_000 });
  await expect(issueRow).toHaveCSS("border-radius", "8px");
  const [issueHeaderBounds, firstIssueBounds, secondIssueBounds] =
    await Promise.all([
      page.getByTestId("project-work-item-group-header").first().boundingBox(),
      page.getByTestId("project-issue-row").nth(0).boundingBox(),
      page.getByTestId("project-issue-row").nth(1).boundingBox(),
    ]);
  expect(
    (firstIssueBounds?.y ?? 0) -
      ((issueHeaderBounds?.y ?? 0) + (issueHeaderBounds?.height ?? 0)),
  ).toBe(4);
  expect(firstIssueBounds?.x).toBe(issueHeaderBounds?.x);
  expect(firstIssueBounds?.width).toBe(issueHeaderBounds?.width);
  expect(
    (secondIssueBounds?.y ?? 0) -
      ((firstIssueBounds?.y ?? 0) + (firstIssueBounds?.height ?? 0)),
  ).toBe(2);
  const issueDate = issueRow.getByTestId("project-issue-row-date");
  const issueId = issueRow.getByTitle("View task");
  await expect(issueDate).toBeVisible();
  const issueDateBounds = await issueDate.boundingBox();
  const issueIdBounds = await issueId.boundingBox();
  const issueRowBounds = await issueRow.boundingBox();
  expect(issueDateBounds).not.toBeNull();
  expect(issueIdBounds).not.toBeNull();
  expect(issueRowBounds).not.toBeNull();
  expect(issueRowBounds?.height).toBeLessThanOrEqual(40);
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
  const issueDetail = page.getByTestId("project-issue-detail");
  await expect(issueDetail).toHaveCSS("max-width", "768px");
  await expect(
    issueDetail.getByRole("heading", { level: 3 }).first(),
  ).toHaveCSS("font-size", "18px");
  await expect(
    page.getByTestId("project-issue-comment-timeline-row").first(),
  ).toBeVisible();
  const issueActivity = workspacePanel.getByRole("button", {
    name: "Activity",
    exact: true,
  });
  await issueActivity.click();
  await expect(composer).toBeVisible();
  await expect(
    page.getByTestId("project-issue-comment-timeline-row"),
  ).toHaveCount(0);
  await issueActivity.click();
  // Let the "Comment posted." toast dismiss so it doesn't photobomb.
  await expect(page.getByText("Comment posted.")).toHaveCount(0, {
    timeout: 10_000,
  });
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/03-issue-detail.png` });

  // PR list: section header owns creation; detail keeps its entity header.
  await page
    .getByRole("navigation", { name: "Project breadcrumb" })
    .getByRole("button", { name: "Tasks", exact: true })
    .click();
  await expect(tabMenu).toBeVisible();
  await page.getByRole("tab", { name: "Review", exact: true }).click();
  await expect(
    repositoryActionsPanel.getByTestId("project-right-panel-scope"),
  ).toHaveCount(0);
  await expect(
    repositoryActionsPanel.getByRole("heading", {
      name: "Review activity",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    workspacePanel.getByRole("heading", {
      name: "Reviews",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    workspacePanel.getByRole("button", { name: "Create review" }),
  ).toBeVisible();
  await expectFullWidthSection();
  await expect(
    tabMenu.getByRole("button", { name: "Create review" }),
  ).toHaveCount(0);
  const prRow = page.getByTestId("project-pull-request-row").first();
  await expect(prRow).toBeVisible({ timeout: 10_000 });
  const [reviewHeaderBounds, firstReviewBounds, secondReviewBounds] =
    await Promise.all([
      page.getByTestId("project-work-item-group-header").first().boundingBox(),
      page.getByTestId("project-pull-request-row").nth(0).boundingBox(),
      page.getByTestId("project-pull-request-row").nth(1).boundingBox(),
    ]);
  expect(
    (firstReviewBounds?.y ?? 0) -
      ((reviewHeaderBounds?.y ?? 0) + (reviewHeaderBounds?.height ?? 0)),
  ).toBe(4);
  expect(firstReviewBounds?.x).toBe(reviewHeaderBounds?.x);
  expect(firstReviewBounds?.width).toBe(reviewHeaderBounds?.width);
  expect(
    (secondReviewBounds?.y ?? 0) -
      ((firstReviewBounds?.y ?? 0) + (firstReviewBounds?.height ?? 0)),
  ).toBe(2);
  const prDate = prRow.getByTestId("project-pull-request-row-date");
  const prId = prRow.getByTitle("View review");
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
  const pullRequestDetail = page.getByTestId("project-pull-request-detail");
  await expect(pullRequestDetail).toHaveCSS("max-width", "768px");
  await expect(
    pullRequestDetail.getByRole("heading", { level: 3 }).first(),
  ).toHaveCSS("font-size", "18px");
  const reviewCommits = workspacePanel.getByRole("button", {
    name: "Commits",
    exact: true,
  });
  await expect(reviewCommits).toHaveAttribute("aria-expanded", "false");
  await expect(reviewCommits).toHaveCSS("font-size", "14px");
  await expect(reviewCommits).toHaveCSS("font-weight", "500");
  await reviewCommits.click();
  const openedReviewCommits = workspacePanel.getByRole("button", {
    name: /^Commits \d+$/,
  });
  await expect(openedReviewCommits).toHaveAttribute("aria-expanded", "true");
  await openedReviewCommits.click();
  const reviewComposer = page.getByTestId(
    "project-pull-request-comment-composer",
  );
  await expect(reviewComposer).toBeVisible();
  const reviewActivity = workspacePanel.getByRole("button", {
    name: "Activity",
    exact: true,
  });
  await reviewActivity.click();
  await expect(reviewComposer).toBeVisible();
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

  await page.getByTestId("projects-section-all").click();
  await expectSinglePrimaryTextColumn(
    page.getByTestId("projects-activity-card").first(),
  );

  await page.getByTestId("projects-section-projects").click();
  await expectSinglePrimaryTextColumn(
    page.getByTestId(/^project-row-/).first(),
  );

  await page.getByTestId("projects-section-repositories").click();
  await expectSinglePrimaryTextColumn(
    page.getByTestId(/^repository-row-/).first(),
  );

  await page.getByTestId("projects-section-prs").click();
  const reviewList = page.getByTestId("projects-list-container");
  await expect(reviewList).toBeVisible();
  const pullRequestRow = page.getByTestId(/^projects-pr-row-/).first();
  await expect(pullRequestRow).toBeVisible();
  await expectSinglePrimaryTextColumn(pullRequestRow);
  await expect(pullRequestRow).toContainText(/relay-tools|buzz|design-system/);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/05-pr-list-metadata.png` });

  await page.getByTestId("projects-section-issues").click();
  const taskList = page.getByTestId("projects-list-container");
  await expect(taskList).toBeVisible();
  const issueRow = page.getByTestId(/^projects-issue-row-/).first();
  await expect(issueRow).toBeVisible();
  await expectSinglePrimaryTextColumn(issueRow);
  await expect(issueRow).toContainText(/relay-tools|buzz|design-system/);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/06-issue-list-metadata.png` });

  await page.getByTestId("projects-section-channels").click();
  const channelRow = page.getByTestId("project-channel-row").first();
  await expect(channelRow).toBeVisible();
  await expectSinglePrimaryTextColumn(channelRow);
  await expect(channelRow).toContainText("#general");
  await expect(
    page.getByTestId("project-channel-project").first(),
  ).toBeVisible();
  await expect(
    page.getByTestId("project-channel-repository").first(),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/07-channels-list.png` });
});
