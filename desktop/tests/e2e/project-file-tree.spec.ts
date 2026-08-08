import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function enableProjectsFeature(page: import("@playwright/test").Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "buzz-feature-overrides-v1",
      JSON.stringify({ projects: true }),
    );
  });
}

async function openBuzzProject(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByRole("button", { name: "Repositories", exact: true }).click();
  const projectEntry = page
    .locator(
      '[data-testid="repository-card-buzz"], [data-testid="repository-row-buzz"]',
    )
    .first();
  await expect(projectEntry).toBeVisible({ timeout: 10_000 });
  await projectEntry.click();
}

test("repository files disclose when the backend tree is truncated", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page, { projectRepoTotalFileCount: 661 });
  await openBuzzProject(page);

  await page.getByRole("tab", { name: "Files", exact: true }).click();

  await expect(
    page.getByText("· 4 of 661 files", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "Showing the first 4 of 661 files. Some files and folders are not included.",
      { exact: true },
    ),
  ).toBeVisible();
});
