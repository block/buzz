import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);

// The projects surface is a preview feature — opt in before the app mounts.
async function enableProjectsFeature(page: import("@playwright/test").Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "buzz-feature-overrides-v1",
      JSON.stringify({ projects: true }),
    );
  });
}

/** Hide every seeded mock project so Projects renders the true empty state. */
async function hideAllMockProjects(page: import("@playwright/test").Page) {
  const hiddenCards = [
    `30617:${DEFAULT_MOCK_PUBKEY}:buzz`,
    `30617:${TEST_IDENTITIES.alice.pubkey}:relay-tools`,
    `30617:${TEST_IDENTITIES.bob.pubkey}:design-system`,
  ];
  await page.addInitScript((cards) => {
    window.localStorage.setItem(
      "buzz.projects.hidden-cards.v1",
      JSON.stringify(cards),
    );
  }, hiddenCards);
}

test("empty Projects keeps Create repository controls", async ({ page }) => {
  await enableProjectsFeature(page);
  await hideAllMockProjects(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();

  await expect(
    page.getByRole("heading", { level: 1, name: "Projects" }),
  ).toBeVisible();
  await expect(page.getByTestId("projects-empty-state")).toBeVisible();
  await expect(page.getByTestId("projects-create-menu")).toBeVisible();

  await page.getByTestId("projects-create-menu").hover();
  await page.getByRole("menuitem", { name: "Repository" }).click();
  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  await page.keyboard.press("Escape");

  await page.getByTestId("projects-empty-create-repository").click();
  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  await expect(page.getByTestId("create-project-name")).toBeVisible();
});
