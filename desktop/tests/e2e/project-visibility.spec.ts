import { expect, test, type Page } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const PRIVATE_CHANNEL_ID = "3c2d9f0a-1b44-5e77-9a21-6f8b0c4d2e91";
const PRIVATE_CHANNEL_NAME = "secret-projects";
const KIND_REPO_ANNOUNCEMENT = 30617;

async function openProjects(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await expect(
    page.getByRole("heading", { level: 1, name: "Projects" }),
  ).toBeVisible();
}

async function openCreateProjectDialog(page: Page) {
  await page.getByTestId("projects-create-menu").click();
  await page.getByRole("menuitem", { name: "Repository" }).click();
  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
}

async function openProject(page: Page, dtag = "buzz") {
  await page.getByRole("button", { name: "Repositories", exact: true }).click();
  const projectEntry = page
    .locator(
      `[data-testid="project-card-${dtag}"], [data-testid="project-row-${dtag}"]`,
    )
    .first();
  await expect(projectEntry).toBeVisible({ timeout: 10_000 });
  await projectEntry.click();
}

async function publishedProjectEvents(page: Page) {
  return page.evaluate(() =>
    (window.__BUZZ_E2E_PUBLISHED_PROJECT_EVENTS__ ?? []).map((event) => ({
      kind: event.kind,
      tags: event.tags,
    })),
  );
}

test("create dialog defaults public and publishes only the chosen non-DM channel", async ({
  page,
}) => {
  await installMockBridge(page, {
    projectEligibleChannelNames: [PRIVATE_CHANNEL_NAME, "alice-tyler"],
  });
  await openProjects(page);
  await openCreateProjectDialog(page);

  const publicChoice = page.getByTestId("create-project-visibility-public");
  const privateChoice = page.getByTestId("create-project-visibility-private");
  const channelPicker = page.getByTestId("create-project-channel");
  const submit = page.getByTestId("create-project-submit");

  await expect(publicChoice).toBeChecked();
  await expect(
    page.getByText("Anyone in the workspace can find and clone"),
  ).toBeVisible();
  await expect(page.getByText("Only members of one channel")).toBeVisible();
  await expect(channelPicker).toHaveCount(0);

  await page.getByTestId("create-project-name").fill("private demo");
  await page.getByText("Private", { exact: true }).click();
  await expect(privateChoice).toBeChecked();
  await expect(channelPicker).toBeVisible();
  await expect(channelPicker.locator("option")).toHaveText([
    "Choose a joined channel…",
    `#${PRIVATE_CHANNEL_NAME}`,
  ]);
  await expect(channelPicker).not.toContainText("alice-tyler");
  await expect(submit).toBeDisabled();

  await channelPicker.selectOption(PRIVATE_CHANNEL_ID);
  await expect(submit).toBeEnabled();
  await submit.click();
  await expect(page.getByTestId("create-project-dialog")).toHaveCount(0);

  const events = await publishedProjectEvents(page);
  expect(events).toEqual([
    expect.objectContaining({
      kind: KIND_REPO_ANNOUNCEMENT,
      tags: expect.arrayContaining([
        ["d", "private-demo"],
        ["buzz-visibility", "private"],
        ["buzz-channel", PRIVATE_CHANNEL_ID],
      ]),
    }),
  ]);
});

test("create dialog explains when no access channels are available", async ({
  page,
}) => {
  await installMockBridge(page, { projectEligibleChannelNames: [] });
  await openProjects(page);
  await openCreateProjectDialog(page);

  await page.getByTestId("create-project-name").fill("blocked private demo");
  await page.getByText("Private", { exact: true }).click();
  await expect(
    page.getByTestId("create-project-visibility-private"),
  ).toBeChecked();

  await expect(
    page.getByText("Join a channel to create a private project"),
  ).toBeVisible();
  await expect(page.getByTestId("create-project-channel")).toBeDisabled();
  await expect(page.getByTestId("create-project-submit")).toBeDisabled();
});

test("create dialog surfaces the relay rejection message verbatim", async ({
  page,
}) => {
  const relayMessage =
    "private repository owner must be a current member of buzz-channel";
  await installMockBridge(page, {
    projectEligibleChannelNames: [PRIVATE_CHANNEL_NAME],
    projectEventRejectionMessages: [relayMessage],
  });
  await openProjects(page);
  await openCreateProjectDialog(page);

  await page.getByTestId("create-project-name").fill("rejected private demo");
  await page.getByText("Private", { exact: true }).click();
  await expect(
    page.getByTestId("create-project-visibility-private"),
  ).toBeChecked();
  await page
    .getByTestId("create-project-channel")
    .selectOption(PRIVATE_CHANNEL_ID);
  await page.getByTestId("create-project-submit").click();

  await expect(page.getByRole("alert")).toHaveText(relayMessage);
  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  expect(await publishedProjectEvents(page)).toHaveLength(0);
});

test("owner visibility menu confirms only private to public", async ({
  page,
}) => {
  await installMockBridge(page, {
    projectEligibleChannelNames: [PRIVATE_CHANNEL_NAME, "engineering"],
    projectPrivateChannelName: PRIVATE_CHANNEL_NAME,
  });
  await openProjects(page);

  const privateCard = page.getByTestId("project-card-buzz");
  await page.getByRole("button", { name: "Repositories", exact: true }).click();
  await expect(
    privateCard.getByTestId("project-private-indicator"),
  ).toBeVisible();

  await page.getByRole("button", { name: "List layout" }).click();
  const privateRow = page.getByTestId("project-row-buzz");
  await expect(
    privateRow.getByTestId("project-private-indicator"),
  ).toBeVisible();
  await privateRow.getByRole("button", { name: "View buzz" }).press("Enter");

  const trigger = page.getByTestId("project-visibility-trigger");
  await expect(trigger).toContainText(`Private · #${PRIVATE_CHANNEL_NAME}`);
  await trigger.click();
  await expect(page.getByTestId("project-visibility-menu")).toBeVisible();
  await expect(page.getByTestId("project-visibility-public")).toBeVisible();
  await expect(page.getByTestId("project-visibility-private")).toBeVisible();
  await page.getByTestId("project-visibility-private").hover();
  await expect(
    page.getByTestId("project-visibility-channel-menu"),
  ).toBeVisible();
  await expect(
    page.getByTestId(`project-visibility-channel-${PRIVATE_CHANNEL_ID}`),
  ).toBeVisible();
  await page.getByTestId("project-visibility-private").press("ArrowLeft");
  await expect(page.getByTestId("project-visibility-menu")).toBeVisible();

  await page.getByTestId("project-visibility-public").click();
  const confirm = page.getByTestId("project-visibility-public-confirm");
  await expect(confirm).toContainText("Make this project public?");
  await expect(confirm).toContainText(
    "Anyone in the workspace will be able to find and clone it.",
  );
  await confirm.getByRole("button", { name: "Cancel" }).click();
  await expect(trigger).toContainText(`Private · #${PRIVATE_CHANNEL_NAME}`);
  expect(await publishedProjectEvents(page)).toHaveLength(0);

  await trigger.click();
  await page.getByTestId("project-visibility-public").click();
  await page.getByTestId("project-visibility-public-confirm-button").click();
  await expect(trigger).toContainText("Public");

  await trigger.click();
  await expect(page.getByTestId("project-visibility-menu")).toBeVisible();
  await page.getByTestId("project-visibility-private").hover();
  await expect(
    page.getByTestId("project-visibility-channel-menu"),
  ).toBeVisible();
  await expect(
    page.getByTestId(`project-visibility-channel-${PRIVATE_CHANNEL_ID}`),
  ).toBeVisible();
  await page
    .getByTestId(`project-visibility-channel-${PRIVATE_CHANNEL_ID}`)
    .click();
  await expect(
    page.getByTestId("project-visibility-public-confirm"),
  ).toHaveCount(0);
  await expect(trigger).toContainText(`Private · #${PRIVATE_CHANNEL_NAME}`);
  await expect
    .poll(async () => (await publishedProjectEvents(page)).length)
    .toBe(2);
  await expect(trigger).toBeEnabled();
  await expect(trigger).toHaveAttribute("data-state", "closed");

  await trigger.press("Enter");
  await expect(page.getByTestId("project-visibility-menu")).toBeVisible();
  await page.getByTestId("project-visibility-private").press("ArrowRight");
  await expect(
    page.getByTestId("project-visibility-channel-menu"),
  ).toBeVisible();
  const engineeringChannel = page.getByTestId(
    "project-visibility-channel-1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9",
  );
  await expect(engineeringChannel).toBeVisible();
  await engineeringChannel.click();
  await expect(
    page.getByTestId("project-visibility-public-confirm"),
  ).toHaveCount(0);
  await expect(trigger).toContainText("Private · #engineering");

  const events = await publishedProjectEvents(page);
  expect(events).toHaveLength(3);
  expect(events[0]?.tags).not.toContainEqual(["buzz-visibility", "private"]);
  expect(events[1]?.tags).toEqual(
    expect.arrayContaining([
      ["buzz-visibility", "private"],
      ["buzz-channel", PRIVATE_CHANNEL_ID],
    ]),
  );
  expect(events[2]?.tags).toEqual(
    expect.arrayContaining([
      ["buzz-visibility", "private"],
      ["buzz-channel", "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9"],
    ]),
  );
});

test("overview rail and non-owner detail expose private state without edit controls", async ({
  page,
}) => {
  await page.addInitScript((owner) => {
    window.__BUZZ_E2E_PROJECT_OWNER_OVERRIDE__ = owner;
  }, TEST_IDENTITIES.alice.pubkey);
  await installMockBridge(page, {
    projectEligibleChannelNames: [PRIVATE_CHANNEL_NAME],
    projectPrivateChannelName: PRIVATE_CHANNEL_NAME,
  });
  await openProjects(page);

  const privateActivity = page
    .getByRole("button", {
      name: "buzz Private project",
      exact: true,
    })
    .first();
  await expect(
    privateActivity.getByTestId("project-private-indicator"),
  ).toBeVisible();

  await openProject(page);
  const status = page.getByTestId("project-visibility-status");
  const tooltip = `Only members of #${PRIVATE_CHANNEL_NAME} and the owner can access this project.`;
  await expect(status).toHaveText(`Private · #${PRIVATE_CHANNEL_NAME}`);
  await expect(status).not.toHaveAttribute("title");
  await expect(page.getByTestId("project-visibility-trigger")).toHaveCount(0);
  await status.hover();
  await expect(page.getByRole("tooltip")).toHaveText(tooltip);
});
