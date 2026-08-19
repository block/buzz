import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { FEATURE_OVERRIDES_STORAGE_KEY } from "../helpers/features";

const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);

// The projects surface is a preview feature — opt in before the app mounts.
// Must run before installMockBridge so React reads the override on mount.
async function enableProjectsFeature(page: import("@playwright/test").Page) {
  await page.addInitScript(
    ({ key }) => {
      window.localStorage.setItem(key, JSON.stringify({ projects: true }));
    },
    { key: FEATURE_OVERRIDES_STORAGE_KEY },
  );
}

/**
 * Hide every seeded mock project so Projects renders the true empty state.
 *
 * Addresses must match MOCK_PROJECT_SEEDS (and the kind:30621 "buzz" project
 * announcement) in `desktop/src/testing/e2eBridge.ts`. If a seed is added or
 * renamed there without updating this list, the list stays populated and the
 * `projects-empty-state` assertion below fails.
 */
async function hideAllMockProjects(page: import("@playwright/test").Page) {
  const hiddenCards = [
    `30621:${DEFAULT_MOCK_PUBKEY}:buzz`,
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

test("empty Projects keeps Create controls and offers an empty-state CTA", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await hideAllMockProjects(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();

  // The chrome (header + toolbar + create menu) must mount with zero projects.
  await expect(page.getByTestId("projects-empty-state")).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.getByTestId("projects-create-menu")).toBeVisible();

  // Entry point 1: the toolbar "+" create menu.
  await page.getByTestId("projects-create-menu").hover();
  await page.getByRole("menuitem", { name: "Project" }).click();
  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("create-project-dialog")).toBeHidden();

  // Entry point 2: the empty-state CTA.
  await page.getByTestId("projects-empty-create-project").click();
  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  await expect(page.getByTestId("create-project-name")).toBeVisible();
});
