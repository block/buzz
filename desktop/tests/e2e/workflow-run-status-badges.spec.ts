import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    workflowRunStatuses: ["failed", "cancelled"],
  });
});

test("shows failed and cancelled run badges while history rows are collapsed", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("open-workflows-view").click();
  await expect(page.getByTestId("workflows-view")).toBeVisible();

  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Workflow name").fill("status_badges");
  await dialog.getByRole("button", { name: "Add step" }).click();
  await dialog.getByRole("button", { name: "Create" }).click();

  await page.getByRole("button", { name: "View status_badges" }).click();
  const panel = page.getByTestId("workflow-detail-panel");
  await expect(panel).toBeVisible();

  for (const status of ["failed", "cancelled"] as const) {
    await panel.getByRole("button", { name: "Trigger" }).click();
    await expect(
      panel.getByTestId(`workflow-run-status-${status}`),
    ).toBeVisible();
    await panel.getByTestId("workflow-selected-run").click();
  }

  await expect(panel.getByTestId("workflow-run-status-failed")).toHaveText(
    "failed",
  );
  await expect(panel.getByTestId("workflow-run-status-cancelled")).toHaveText(
    "cancelled",
  );
  await expect(panel.getByTestId("workflow-run-trace")).not.toBeVisible();

  if (process.env.BUZZ_WORKFLOW_STATUS_SCREENSHOTS === "1") {
    await waitForAnimations(page);
    await panel.screenshot({
      path: "test-results/workflow-run-status-badges.png",
    });
  }
});
