import { expect, test, type Locator, type Page } from "@playwright/test";

import { installMockBridge, openCreateChannelDialog } from "../helpers/bridge";

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const OWNER = "deadbeef".repeat(8);
const COLLABORATOR_OWNER = "cafe".repeat(16);

async function addAndOpenProject(page: Page, dtag: string) {
  const projectRow = page.getByTestId(`sidebar-project-${dtag}`);
  if ((await projectRow.count()) === 0) {
    await page.getByTestId("sidebar-projects-section-label").hover();
    await page.getByTestId("sidebar-projects-create").click();
    await page.getByTestId(`project-browser-result-${dtag}`).click();
  }
  await projectRow.click();
}

async function openGeneralChannelSettings(page: Page, dtag: string) {
  await addAndOpenProject(page, dtag);
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-project-features")).toBeVisible();
}

async function createPocRootChannel(page: Page) {
  const name = "channel-project-poc";
  await openCreateChannelDialog(page);
  await page.getByTestId("create-channel-name").fill(name);
  await page.getByTestId("create-channel-submit").click();
  await expect(page.getByTestId("chat-title")).toHaveText(name);
  const channelId = page.url().match(/\/channels\/([^/?]+)/)?.[1];
  if (!channelId)
    throw new Error("Created channel route did not include an ID.");
  return { channelId, name };
}

async function acceptedProjectEvents(page: Page) {
  return page.evaluate(() => window.__BUZZ_E2E_ACCEPTED_PROJECT_EVENTS__ ?? []);
}

async function dragSidebarRow(page: Page, source: Locator, target: Locator) {
  const sourceBox = await source.boundingBox();
  const targetBox = await target.boundingBox();
  if (!sourceBox || !targetBox) {
    throw new Error("Sidebar drag source or target was unavailable.");
  }
  await page.mouse.move(
    sourceBox.x + sourceBox.width / 2,
    sourceBox.y + sourceBox.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    targetBox.x + targetBox.width / 2,
    targetBox.y + Math.min(targetBox.height / 2, 80),
    { steps: 12 },
  );
  await expect(page.getByTestId("sidebar-channel-drag-overlay")).toBeVisible();
  await page.mouse.up();
  await expect(
    page.getByTestId("sidebar-channel-drag-overlay"),
  ).not.toBeVisible();
}

test("first channel feature quietly creates one backing project", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  const rootChannel = await createPocRootChannel(page);
  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-project-features")).toBeVisible();

  await page.getByTestId("channel-feature-repositories-switch").click();
  await expect
    .poll(async () => (await acceptedProjectEvents(page)).length)
    .toBe(2);

  const events = await acceptedProjectEvents(page);
  expect(events.map((event) => event.kind).sort()).toEqual([30617, 30621]);
  for (const event of events) {
    expect(event.tags).toContainEqual(["buzz-channel", rootChannel.channelId]);
  }
  const repository = events.find((event) => event.kind === 30617);
  const project = events.find((event) => event.kind === 30621);
  const repositoryDtag = repository?.tags.find((tag) => tag[0] === "d")?.[1];
  const projectDtag = project?.tags.find((tag) => tag[0] === "d")?.[1];
  expect(projectDtag).toBeTruthy();
  expect(project?.tags).toContainEqual([
    "a",
    `30617:${OWNER}:${repositoryDtag}`,
  ]);

  await page.getByTestId("auxiliary-panel-close").click();
  await expect(page.getByTestId("project-channel-home")).toBeVisible();
  await expect(page.getByTestId("project-channel-tab-tasks")).toHaveCount(0);
  await expect(page.getByTestId("project-channel-tab-repos")).toBeVisible();
  await expect(page.getByTestId("project-channel-tab-reviews")).toHaveCount(0);
  await expect(page.getByTestId("project-channel-tab-commits")).toHaveCount(0);
  await expect(page.getByTestId("project-channel-tab-files")).toHaveCount(0);
  await expect(page.getByTestId("project-channel-tab-people")).toHaveCount(0);
  await expect(page.getByTestId("project-channel-tab-codebase")).toHaveCount(0);
  await expect(page.getByTestId("project-channel-tab-channels")).toHaveCount(0);
  await expect(page.getByTestId("project-home-summary-rail")).toHaveCount(0);

  await page.getByTestId("channel-management-trigger").click();
  await page.getByTestId("channel-feature-tasks-switch").click();
  await page.getByTestId("auxiliary-panel-close").click();
  await expect(page.getByTestId("project-channel-tab-tasks")).toBeVisible();
  await expect(page.getByTestId("project-channel-tab-channels")).toHaveCount(0);

  await page.getByTestId("channel-management-trigger").click();
  await page.getByTestId("channel-feature-breakouts-switch").click();
  await expect
    .poll(async () => (await acceptedProjectEvents(page)).length)
    .toBe(2);
  await page.getByTestId("auxiliary-panel-close").click();
  await expect(page.getByTestId("project-channel-tab-channels")).toBeVisible();

  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-feature-tasks-switch")).toBeChecked();
  await expect(
    page.getByTestId("channel-feature-repositories-switch"),
  ).toBeChecked();
  await expect(
    page.getByTestId("channel-feature-breakouts-switch"),
  ).toBeChecked();
  await expect(
    page.getByTestId("channel-feature-reviews-switch"),
  ).not.toBeChecked();
  await page.getByTestId("channel-feature-reviews-switch").click();
  await page.getByTestId("auxiliary-panel-close").click();

  await expect(page.getByTestId("project-channel-home")).toBeVisible();
  await expect(page.getByTestId("channel-project-feature-bar")).toHaveCount(0);
  await expect(page.getByTestId("project-channel-tab-reviews")).toBeVisible();
  await page.getByTestId("project-channel-tab-tasks").click();
  const tasksWorkspace = page.getByTestId("project-home-workspace-sheet");
  await expect(tasksWorkspace).toHaveAttribute("data-tab", "issues");
  await expect(page.getByTestId("channel-main-content")).toBeVisible();
  await expect(page.getByTestId("channel-composer-overlay")).not.toBeVisible();
  await expect(
    tasksWorkspace.getByText("No tasks yet", { exact: true }),
  ).toBeVisible();
  await page.getByTestId("project-home-workspace-sheet-create").click();
  await page.getByTestId("create-issue-title").fill("POC task");
  await page.getByTestId("create-issue-submit").click();
  await expect(
    tasksWorkspace.getByTestId("project-issue-detail"),
  ).toContainText("POC task");
  await page.getByTestId("chat-title-tab").click();
  await expect(page.getByTestId("project-home-workspace-sheet")).toHaveCount(0);
  await expect(page.getByTestId("channel-composer-overlay")).toBeVisible();

  await page.getByTestId("project-channel-tab-channels").click();
  await expect(
    page.getByTestId("project-channel-content-channels"),
  ).toBeVisible();
  await page.getByTestId("add-project-channel").click();
  await page.getByTestId("create-channel-name").fill("poc-breakout");
  await page.getByTestId("create-channel-submit").click();
  await expect(page.getByTestId("chat-title")).toHaveText("poc-breakout");

  await page.getByTestId(`sidebar-project-${projectDtag}`).click();
  await expect(page.getByTestId("project-channel-home")).toBeVisible();
  await page.getByTestId("project-channel-tab-repos").click();
  await expect(page.getByTestId("project-channel-content-repos")).toBeVisible();
  await expect(page.getByTestId("add-project-repository")).toBeVisible();
  await expect(page.getByTestId("project-channel-repos-tabs")).toHaveCount(0);
  await page.getByTestId("project-channel-tab-reviews").click();
  await expect(
    page.getByTestId("project-home-workspace-sheet"),
  ).toHaveAttribute("data-tab", "prs");
  await page.setViewportSize({ height: 844, width: 390 });
  await expect(page.getByTestId("project-channel-home")).toBeVisible();
  await expect(
    page.getByTestId("project-channel-tab-reviews"),
  ).toBeInViewport();
  await page.getByTestId("project-channel-tab-repos").click();
  await expect(page.getByTestId("project-channel-tab-repos")).toBeInViewport();
  await expect(page.getByTestId("project-channel-content-repos")).toBeVisible();
  await page.getByTestId("project-channel-tab-tasks").click();
  const mobileTasksWorkspace = page.getByTestId("project-home-workspace-sheet");
  await expect(
    mobileTasksWorkspace.getByText("POC task", { exact: true }),
  ).toBeVisible();
  await page.getByTestId("project-channel-tab-repos").click();
  await page
    .getByTestId(`project-channel-resource-repository-${repositoryDtag}`)
    .click();
  await expect(page).toHaveURL(/\/projects\/.*repositoryId=/);
});

test("existing channel project data infers features with separate Projects navigation", async ({
  page,
}) => {
  const dtag = "general-root";
  const repositoryAddress = `30617:${COLLABORATOR_OWNER}:${dtag}`;
  const projectAddress = `30621:${COLLABORATOR_OWNER}:${dtag}`;
  await page.addInitScript(
    ({ channelId, owner, repoAddress, projectDtag }) => {
      const createdAt = Math.floor(Date.now() / 1_000) - 30;
      window.__BUZZ_E2E_EXTRA_PROJECT_EVENTS__ = [
        {
          id: "a1".padEnd(64, "0"),
          kind: 30617,
          pubkey: owner,
          created_at: createdAt,
          content: "",
          tags: [
            ["d", projectDtag],
            ["name", "general"],
            ["buzz-channel", channelId],
          ],
        },
        {
          id: "a2".padEnd(64, "0"),
          kind: 30621,
          pubkey: owner,
          created_at: createdAt,
          content: "",
          tags: [
            ["d", projectDtag],
            ["name", "general"],
            ["buzz-channel", channelId],
            ["a", repoAddress],
          ],
        },
        {
          id: "a3".padEnd(64, "0"),
          kind: 1621,
          pubkey: owner,
          created_at: createdAt,
          content: "Seeded task",
          tags: [
            ["a", repoAddress],
            ["subject", "Seeded task"],
            ["t", "issue"],
          ],
        },
      ];
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      owner: COLLABORATOR_OWNER,
      repoAddress: repositoryAddress,
      projectDtag: dtag,
    },
  );
  await installMockBridge(page);
  await page.goto("/");
  await openGeneralChannelSettings(page, dtag);

  await expect(page.getByTestId("channel-feature-tasks-switch")).toBeChecked();
  await expect(page.getByText("In use", { exact: true })).toHaveCount(0);
  await page.getByTestId("auxiliary-panel-close").click();
  await expect(page.getByTestId("project-channel-tab-tasks")).toBeVisible();
  await expect(page.getByTestId("project-channel-tab-reviews")).toHaveCount(0);
  await expect(page.getByTestId("project-channel-tab-commits")).toHaveCount(0);
  await expect(page.getByTestId("project-channel-tab-files")).toHaveCount(0);
  await expect(page.getByTestId("project-channel-tab-people")).toHaveCount(0);
  await expect(page.getByTestId("project-channel-tab-codebase")).toHaveCount(0);
  await expect(page.getByTestId("project-channel-tab-repos")).toHaveCount(0);
  await expect(page.getByTestId("project-channel-tab-channels")).toHaveCount(0);

  await page.getByTestId("channel-management-trigger").click();
  await page.getByTestId("channel-feature-reviews-switch").click();
  await page.getByTestId("auxiliary-panel-close").click();
  await expect(page.getByTestId("project-channel-tab-reviews")).toBeVisible();
  await expect(page.getByTestId("project-channel-tab-repos")).toHaveCount(0);
  await page.getByTestId("project-channel-tab-reviews").click();
  await expect(
    page.getByTestId("project-home-workspace-sheet"),
  ).toHaveAttribute("data-tab", "prs");
  await page.getByTestId("chat-title-tab").click();
  await page.getByTestId("channel-management-trigger").click();
  await page.getByTestId("channel-feature-repositories-switch").click();
  await expect
    .poll(async () => (await acceptedProjectEvents(page)).length)
    .toBe(0);
  await expect(page.getByTestId("open-projects-view")).toBeVisible();
  const projectsSection = page.getByTestId("sidebar-projects-section");
  await expect(projectsSection).toBeVisible();

  await page.getByTestId("auxiliary-panel-close").click();
  await expect(page.getByTestId("project-channel-tab-reviews")).toBeVisible();
  await expect(page.getByTestId("project-channel-tab-repos")).toBeVisible();
  const projectRow = page.getByTestId(`sidebar-project-${dtag}`);
  await expect(projectRow).toBeVisible();
  await expect(
    projectRow
      .locator('xpath=ancestor::*[@data-sidebar="group"][1]')
      .locator("[data-sidebar-section-title]"),
  ).toHaveText("Projects");
  await expect(
    page.getByTestId("stream-list").getByTestId(`sidebar-project-${dtag}`),
  ).toHaveCount(0);
  await projectRow.click();
  await page.getByTestId("channel-random").click({ button: "right" });
  await page.getByRole("menuitem", { name: "Move to section" }).hover();
  const projectDestination = page.getByTestId(
    `move-channel-to-project-${projectAddress}`,
  );
  await expect(projectDestination).toHaveText("general");
  await projectDestination.click();

  const channelsList = page.getByTestId("stream-list");
  await expect(page.getByTestId("project-channel-group-general")).toHaveCount(
    0,
  );
  await expect(channelsList).not.toContainText("general");
  await expect(page.getByTestId("channel-random")).toHaveCount(0);
  await expect(projectRow).toBeVisible();
  const projectDisclosure = page.getByTestId(`sidebar-project-expand-${dtag}`);
  await expect(projectDisclosure).toBeVisible();
  await projectDisclosure.click();
  const projectChildRow = page.getByTestId(
    `sidebar-project-channel-${dtag}-random`,
  );
  await expect(projectChildRow).toBeVisible();
  await expect(projectChildRow.locator("[data-channel-id]")).toHaveAttribute(
    "data-channel-id",
    /.+/,
  );
  await expect(
    projectChildRow.locator("[data-sidebar-draggable-channel]"),
  ).toBeVisible();
  await projectChildRow.click({ button: "right" });
  await expect(
    page.getByRole("menuitem", { name: "Move to section" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await dragSidebarRow(
    page,
    projectChildRow.locator("[data-sidebar-draggable-channel]"),
    page.locator("[data-sidebar-drop-ungrouped]"),
  );
  await expect(projectChildRow).toHaveCount(0);
  const ungroupedRandom = page.getByTestId("channel-random");
  await expect(ungroupedRandom).toBeVisible();
  await ungroupedRandom.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Move to section" }).hover();
  await page.getByTestId(`move-channel-to-project-${projectAddress}`).click();
  await expect(projectChildRow).toBeVisible();

  await page.getByTestId("sidebar-projects-settings").click();
  await page.getByRole("menuitem", { name: "Show" }).hover();
  await page.getByRole("menuitemradio", { name: "Owned by me" }).click();
  await expect(projectRow).toHaveCount(0);
  await expect(page.getByTestId("channel-general")).toBeVisible();
  await expect(page.getByTestId("channel-random")).toBeVisible();

  await page.getByTestId("sidebar-projects-settings").click();
  await page.getByRole("menuitem", { name: "Show" }).hover();
  await page.getByRole("menuitemradio", { name: "Added" }).click();
  await expect(projectRow).toBeVisible();
  await expect(page.getByTestId("channel-general")).toHaveCount(0);
  await projectRow.click();
  await page.getByTestId("channel-management-trigger").click();
  await expect(
    page.getByTestId("channel-feature-breakouts-switch"),
  ).toBeChecked();
  await page.getByTestId("auxiliary-panel-close").click();
  await expect(page.getByTestId("project-channel-home")).toBeVisible();
  await expect(page.getByTestId("channel-project-feature-bar")).toHaveCount(0);
  await page.getByTestId("project-channel-tab-channels").click();
  await expect(
    page.getByTestId("project-channel-resource-channel-random"),
  ).toBeVisible();
  await page.getByTestId("project-channel-tab-tasks").click();
  await expect(
    page
      .getByTestId("project-home-workspace-sheet")
      .getByText("Seeded task", { exact: true }),
  ).toBeVisible();

  await page.setViewportSize({ height: 844, width: 390 });
  const mobileSidebar = page.locator(
    '[data-sidebar="sidebar"][data-mobile="true"]',
  );
  await page.getByRole("button", { name: "Toggle Sidebar" }).click();
  await expect(mobileSidebar).toBeVisible();
  await expect
    .poll(async () => Math.round((await mobileSidebar.boundingBox())?.x ?? -1))
    .toBe(0);
  await expect(projectsSection).toBeVisible();
  await expect(projectRow).toBeVisible();
  await page.keyboard.press("Escape");
  await page.setViewportSize({ height: 720, width: 1280 });

  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-experimental").click();
  await expect(page.getByTestId("feature-toggle-projects")).toHaveCount(0);
});
