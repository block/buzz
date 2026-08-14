import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

async function navigateToWorkflows(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-workflows-view").click();
  await expect(page).toHaveURL(/#\/workflows$/);
  await expect(page.getByTestId("workflows-view")).toBeVisible();
}

async function createWorkflow(
  page: import("@playwright/test").Page,
  name: string,
  options?: {
    description?: string;
    enabled?: boolean;
    trigger?: string;
    stepCondition?: string;
    stepName?: string;
    stepTimeoutSecs?: string;
  },
) {
  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();

  await dialog.getByLabel("Workflow name").fill(name);
  if (options?.description) {
    await dialog.getByLabel("Description (optional)").fill(options.description);
  }
  if (options?.enabled === false) {
    await dialog.getByLabel("Workflow is enabled").click();
  }
  if (options?.trigger) {
    await dialog.getByLabel("Trigger").selectOption(options.trigger);
  }

  await dialog.getByRole("button", { name: "Add step" }).click();
  if (options?.stepName) {
    await dialog.getByLabel("Step name (optional)").fill(options.stepName);
  }
  if (options?.stepCondition) {
    await dialog
      .getByLabel("Run condition (optional)")
      .fill(options.stepCondition);
  }
  if (options?.stepTimeoutSecs) {
    await dialog
      .getByLabel("Timeout seconds (optional)")
      .fill(options.stepTimeoutSecs);
  }

  await dialog.getByRole("button", { name: "Create" }).click();

  await expect(
    page.getByRole("heading", { name: "Create Workflow" }),
  ).not.toBeVisible();
}

test("navigates to workflows view and shows empty state", async ({ page }) => {
  await navigateToWorkflows(page);

  await expect(page.getByText("No workflows yet")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Create your first workflow" }),
  ).toBeVisible();
});

test("creates a workflow via the form builder", async ({ page }) => {
  const workflowName = `test_workflow_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);

  // Verify workflow appears in the list
  await expect(page.getByTestId("workflows-view")).toContainText(workflowName);
});

test("disables autocapitalization in the workflow form", async ({ page }) => {
  await navigateToWorkflows(page);

  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog");

  await expect(dialog.getByLabel("Workflow name")).toHaveAttribute(
    "autocapitalize",
    "off",
  );

  await dialog.getByRole("button", { name: "Add step" }).click();
  await expect(dialog.getByLabel("Step name (optional)")).toHaveAttribute(
    "autocapitalize",
    "off",
  );
});

test("captures disabled diff workflows in the list UI", async ({ page }) => {
  const workflowName = `diff_workflow_${Date.now()}`;
  const description = "Watches diff events for src/ changes";

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName, {
    description,
    enabled: false,
    trigger: "diff_posted",
    stepName: "Notify reviewers",
    stepCondition: 'str_contains(trigger_text, "src/")',
    stepTimeoutSecs: "45",
  });

  const card = page
    .locator('[data-testid^="workflow-card-"]')
    .filter({ hasText: workflowName })
    .first();
  await expect(card).toContainText(workflowName);
  await expect(card).toContainText(description);
  await expect(card).toContainText("Diff Posted");
  await expect(card).toContainText("disabled");
});

test("shows the webhook secret dialog after saving a webhook workflow", async ({
  page,
}) => {
  const workflowName = `webhook_workflow_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName, {
    trigger: "webhook",
  });

  await expect(page.getByText("Webhook Ready")).toBeVisible();
  await expect(page.getByRole("button", { name: "Copy URL" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Copy Secret" })).toBeVisible();

  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByText("Webhook Ready")).not.toBeVisible();
});

test("edits an existing workflow", async ({ page }) => {
  const originalName = `edit_test_${Date.now()}`;
  const updatedName = `${originalName}_updated`;

  await navigateToWorkflows(page);
  await createWorkflow(page, originalName);

  // Verify it exists
  await expect(page.getByTestId("workflows-view")).toContainText(originalName);

  // Open the dropdown menu and click Edit
  await page.getByRole("button", { name: "Workflow actions" }).first().click();
  await page.getByRole("menuitem", { name: "Edit" }).click();

  // Dialog should open in edit mode
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByText("Edit Workflow")).toBeVisible();

  // Change the name
  const nameInput = page.getByLabel("Workflow name");
  await nameInput.clear();
  await nameInput.fill(updatedName);

  // Save
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("dialog")).not.toBeVisible();

  // Verify the updated name appears
  await expect(page.getByTestId("workflows-view")).toContainText(updatedName);
});

test("duplicates a workflow", async ({ page }) => {
  const originalName = `dup_test_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, originalName);

  // Open the dropdown menu and click Duplicate
  await page.getByRole("button", { name: "Workflow actions" }).first().click();
  await page.getByRole("menuitem", { name: "Duplicate" }).click();

  // Dialog should open in duplicate mode with "(copy)" suffix
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByText("Duplicate Workflow")).toBeVisible();

  // Submit the duplicate
  await page.getByRole("button", { name: "Create Copy" }).click();
  await expect(page.getByRole("dialog")).not.toBeVisible();

  // Both the original and copy should exist
  await expect(page.getByTestId("workflows-view")).toContainText(originalName);
});

test("deletes a workflow with confirmation", async ({ page }) => {
  const workflowName = `delete_test_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);

  // Verify it exists
  await expect(page.getByTestId("workflows-view")).toContainText(workflowName);

  // Open the dropdown menu and click Delete
  await page.getByRole("button", { name: "Workflow actions" }).first().click();
  await page.getByRole("menuitem", { name: "Delete" }).click();

  // Confirmation dialog should appear with workflow name
  await expect(page.getByRole("alertdialog")).toBeVisible();
  await expect(page.getByRole("alertdialog")).toContainText(workflowName);

  // Confirm deletion
  await page.getByRole("button", { name: "Delete" }).click();
  await expect(page.getByRole("alertdialog")).not.toBeVisible();

  // Verify workflow is gone — back to empty state
  await expect(page.getByText("No workflows yet")).toBeVisible();
});

test("triggers a workflow from the detail panel", async ({ page }) => {
  const workflowName = `trigger_test_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);

  // Click on the workflow card to open the detail panel
  await page.getByRole("button", { name: `View ${workflowName}` }).click();
  await expect(page.getByTestId("workflow-detail-panel")).toBeVisible();

  // Click the Trigger button
  await page
    .getByTestId("workflow-detail-panel")
    .getByRole("button", { name: "Trigger" })
    .click();

  // Wait for the trigger to complete (button text changes back from "Triggering...")
  await expect(
    page
      .getByTestId("workflow-detail-panel")
      .getByRole("button", { name: "Trigger" }),
  ).toBeVisible();

  await expect(
    page
      .getByTestId("workflow-detail-panel")
      .getByTestId("workflow-selected-run"),
  ).toBeVisible();
  await expect(
    page.getByTestId("workflow-detail-panel").getByTestId("workflow-run-trace"),
  ).toContainText("step_1");
});
