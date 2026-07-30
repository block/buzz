import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test("first repository can be created from the empty projects shell", async ({
  page,
}) => {
  await installMockBridge(page, { emptyProjects: true });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();

  await expect(
    page.getByRole("heading", { level: 1, name: "Projects" }),
  ).toBeVisible();
  await expect(page.getByTestId("projects-create-menu")).toBeVisible();
  await expect(page.getByText("No projects yet")).toBeVisible();

  await page.getByRole("button", { name: "Create repository" }).click();
  await expect(page.getByTestId("create-project-dialog")).toBeVisible();

  await page.getByTestId("create-project-name").fill("first-repository");
  await page.getByTestId("create-project-submit").click();

  await expect(page.getByTestId("create-project-dialog")).toBeHidden();
  await expect(
    page.getByText('Project "first-repository" created.'),
  ).toBeVisible();
  await expect(
    page.getByText("first-repository", { exact: true }),
  ).toBeVisible();
});
