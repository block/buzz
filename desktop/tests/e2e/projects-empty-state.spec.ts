import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { FEATURE_OVERRIDES_STORAGE_KEY } from "../helpers/features";

const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
// Locally hiding the seeded project and repository announcements is the only
// mock-bridge path to a zero-project relay.
const ALL_MOCK_PROJECT_COORDINATES = [
  `30621:${DEFAULT_MOCK_PUBKEY}:buzz`,
  `30617:${DEFAULT_MOCK_PUBKEY}:buzz`,
  `30617:${TEST_IDENTITIES.alice.pubkey}:relay-tools`,
  `30617:${TEST_IDENTITIES.bob.pubkey}:design-system`,
];

async function openEmptyProjectsView(page: import("@playwright/test").Page) {
  // Both overrides must land before the app mounts: projects is a preview
  // feature, and the hidden-card list is read while fetching projects.
  await page.addInitScript(
    ({ coordinates, featureOverridesStorageKey }) => {
      window.localStorage.setItem(
        featureOverridesStorageKey,
        JSON.stringify({ projects: true }),
      );
      window.localStorage.setItem(
        "buzz.projects.hidden-cards.v1",
        JSON.stringify(coordinates),
      );
    },
    {
      coordinates: ALL_MOCK_PROJECT_COORDINATES,
      featureOverridesStorageKey: FEATURE_OVERRIDES_STORAGE_KEY,
    },
  );
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
}

test("projects header and create menu stay mounted with zero projects", async ({
  page,
}) => {
  await openEmptyProjectsView(page);

  await expect(
    page.getByRole("heading", { level: 1, name: "Projects" }),
  ).toBeVisible();
  await expect(page.getByText("No projects yet")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Repositories", exact: true }),
  ).toBeVisible();

  // The regression: the create menu was unreachable until a project already
  // existed, so the first project could never be created from the UI.
  await page.getByTestId("projects-create-menu").hover();
  await page.getByRole("menuitem", { name: "Project" }).click();
  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  await expect(page.getByTestId("create-project-name")).toBeVisible();
});

test("empty state replaces only the content area on every filter", async ({
  page,
}) => {
  await openEmptyProjectsView(page);

  for (const filter of ["Repositories", "Pull Requests", "Issues"]) {
    await page.getByRole("button", { name: filter, exact: true }).click();
    await expect(page.getByText("No projects yet")).toBeVisible();
    await expect(page.getByTestId("projects-create-menu")).toBeVisible();
  }
});
