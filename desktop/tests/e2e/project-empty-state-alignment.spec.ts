import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const ALIGNMENT_TOLERANCE_PX = 2;

async function enableProjectsFeature(page: import("@playwright/test").Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "buzz-feature-overrides-v1",
      JSON.stringify({ projects: true }),
    );
  });
}

async function addProjectToSidebar(
  page: import("@playwright/test").Page,
  dtag: string,
) {
  await page.getByTestId("sidebar-projects-section-label").hover();
  await page.getByTestId("sidebar-projects-create").click();
  const browser = page.getByTestId("project-browser-dialog");
  await browser.getByRole("searchbox", { name: "Search projects" }).fill(dtag);
  await browser.getByTestId(`project-browser-result-${dtag}`).click();
  await expect(browser).toBeHidden();
  await expect(page.getByTestId(`sidebar-project-${dtag}`)).toBeVisible();
}

test("first-time project empty state opens project creation", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    window.__BUZZ_E2E_EMPTY_PROJECTS__ = true;
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();

  await expect(
    page.getByRole("main").getByText("No projects yet"),
  ).toBeVisible();
  await page.getByRole("button", { name: "Create project" }).click();
  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
});

test("project home context aligns with the channel header", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await addProjectToSidebar(page, "buzz");
  await waitForAnimations(page);

  const [headerTitleBox, tasksBox] = await Promise.all([
    page.getByTestId("chat-title").boundingBox(),
    page.getByTestId("project-home-context-tasks").boundingBox(),
  ]);
  expect(headerTitleBox).not.toBeNull();
  expect(tasksBox).not.toBeNull();
  const headerTitleCenter =
    (headerTitleBox?.y ?? 0) + (headerTitleBox?.height ?? 0) / 2;
  const tasksCenter = (tasksBox?.y ?? 0) + (tasksBox?.height ?? 0) / 2;
  expect(Math.abs(headerTitleCenter - tasksCenter)).toBeLessThanOrEqual(
    ALIGNMENT_TOLERANCE_PX,
  );
});
