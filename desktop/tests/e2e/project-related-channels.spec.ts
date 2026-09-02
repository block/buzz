import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const WELCOME_CHANNEL_ID = "5f0b1b3c-2a37-5366-9b8c-31a4b21d8e77";
const WELCOME_CHANNEL_ROW = "project-home-context-channel-welcome-everyone";

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
  await expect(page.getByTestId("project-home-context-panel")).toBeVisible();
}

async function addWelcomeChannel(page: import("@playwright/test").Page) {
  await page.getByTestId("project-home-context-channel").hover();
  await page.getByTestId("add-project-channel").click();
  const dialog = page.getByTestId("add-project-channel-dialog");
  await expect(dialog).toBeVisible();
  await dialog
    .getByTestId(`add-existing-project-channel-${WELCOME_CHANNEL_ID}`)
    .click();
  await expect(dialog).toBeHidden();
  await expect(page.getByTestId(WELCOME_CHANNEL_ROW)).toBeVisible();
}

async function acceptedProjectChanges(page: import("@playwright/test").Page) {
  return page.evaluate(() =>
    (window.__BUZZ_E2E_ACCEPTED_PROJECT_EVENTS__ ?? [])
      .filter((event) => event.kind === 47010)
      .map((event) => ({
        content: JSON.parse(event.content),
        tags: event.tags,
      })),
  );
}

test("Project owner links an existing channel", async ({ page }) => {
  await installMockBridge(page);
  await openBuzzProject(page);
  await addWelcomeChannel(page);

  await expect
    .poll(async () => (await acceptedProjectChanges(page)).length)
    .toBe(1);
  const [change] = await acceptedProjectChanges(page);
  expect(change.tags).toEqual([
    ["a", `30621:${"deadbeef".repeat(8)}:buzz`],
    ["expected-revision", "1"],
  ]);
  expect(change.content).toEqual({
    v: 1,
    patch: {
      related_channels: { add: [WELCOME_CHANNEL_ID], remove: [] },
    },
  });
});

test("home-channel admin links and removes an existing channel", async ({
  page,
}) => {
  await page.addInitScript((owner) => {
    window.__BUZZ_E2E_PROJECT_OWNER_OVERRIDE__ = owner;
  }, TEST_IDENTITIES.alice.pubkey);
  await installMockBridge(page, { projectHomeRole: "admin" });
  await openBuzzProject(page);
  await addWelcomeChannel(page);

  const row = page.getByTestId(WELCOME_CHANNEL_ROW);
  await row.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Remove from project" }).click();
  await expect(row).toHaveCount(0);

  await expect
    .poll(async () => (await acceptedProjectChanges(page)).length)
    .toBe(2);
  const [add, remove] = await acceptedProjectChanges(page);
  expect(add.tags).toContainEqual(["expected-revision", "1"]);
  expect(remove.tags).toContainEqual(["expected-revision", "2"]);
  expect(add.content.patch.related_channels).toEqual({
    add: [WELCOME_CHANNEL_ID],
    remove: [],
  });
  expect(remove.content.patch.related_channels).toEqual({
    add: [],
    remove: [WELCOME_CHANNEL_ID],
  });
});
