import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test("Projects view still shows the toolbar and create menu with zero projects", async ({
  page,
}) => {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "buzz-feature-overrides-v1",
      JSON.stringify({ projects: true }),
    );
    window.__BUZZ_E2E_HIDE_MOCK_PROJECTS__ = true;
  });
  await installMockBridge(page);
  await page.setViewportSize({ width: 1024, height: 720 });

  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByRole("button", { name: "Repositories", exact: true }).click();

  await expect(page.getByText("No projects yet")).toBeVisible();
  // The create menu (and the rest of the toolbar) must still mount so the
  // first project can be created from an empty relay.
  await expect(page.getByTestId("projects-create-menu")).toBeVisible();
});
