import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

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

// Regression for the projects empty state: when there are zero projects the
// only entry point to create one (the "+" menu in the toolbar) is not
// rendered, so the empty state itself must offer a create CTA. Otherwise the
// first project can only be created from the CLI.
test("empty projects state offers a create-project CTA", async ({ page }) => {
  await enableProjectsFeature(page);
  await installMockBridge(page, { emptyProjects: true });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  await page.getByTestId("open-projects-view").click();

  await expect(page.getByText("No projects yet")).toBeVisible();

  const createCta = page.getByTestId("projects-empty-create");
  await expect(createCta).toBeVisible();

  await createCta.click();

  // The CTA opens the same create-project dialog as the toolbar "+" menu.
  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  await expect(page.getByTestId("create-project-name")).toBeVisible();
});
