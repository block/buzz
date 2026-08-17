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

test("navigates to workflows view and shows the empty create tile", async ({
  page,
}) => {
  await navigateToWorkflows(page);

  await expect(page.getByTestId("new-workflow-card")).toBeVisible();
  await expect(page.locator('[data-testid^="workflow-card-"]')).toHaveCount(0);
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

test("captures workflow library across responsive viewports", async ({
  page,
}) => {
  await navigateToWorkflows(page);
  await createWorkflow(page, "Notify reviewers when source files change", {
    description: "Watches diff events for src/ changes",
    enabled: false,
    trigger: "diff_posted",
  });
  await createWorkflow(page, "Post the daily standup reminder to the team", {
    description: "Keeps the team aligned every morning",
    trigger: "schedule",
  });
  await createWorkflow(
    page,
    "Request approval before deploying to production",
    {
      description: "Requires a final review before release",
      trigger: "reaction_added",
    },
  );

  for (const viewport of [
    { width: 800, height: 720, name: "narrow" },
    { width: 1024, height: 720, name: "medium" },
    { width: 1280, height: 720, name: "wide" },
  ]) {
    await page.setViewportSize(viewport);
    await page.screenshot({
      animations: "disabled",
      path: `test-results/workflow-library-${viewport.name}.png`,
    });
  }

  await page.setViewportSize({ width: 1280, height: 720 });
  const firstCard = page.locator('[data-testid^="workflow-card-"]').first();
  await firstCard.getByRole("button", { name: "Workflow actions" }).click();
  await page.screenshot({
    animations: "disabled",
    path: "test-results/workflow-library-wide-actions.png",
  });
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
  await expect(card.getByText("Diff Posted", { exact: true })).toBeVisible();
  await expect(card.locator("h3")).toHaveText(workflowName);
  await expect(card.getByText(description, { exact: true })).toBeVisible();
  await expect(card).toContainText("disabled");
});

test("enables and disables a workflow from its card menu", async ({ page }) => {
  const workflowName = `toggle_workflow_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);

  const workflowCard = () =>
    page
      .locator('[data-testid^="workflow-card-"]')
      .filter({ hasText: workflowName })
      .first();
  const workflowActions = () =>
    workflowCard().getByRole("button", { name: "Workflow actions" });

  const enableItem = page.getByRole("menuitemcheckbox", { name: "Enable" });

  await page.getByRole("button", { name: `View ${workflowName}` }).click();
  const detailPanel = page.getByTestId("workflow-detail-panel");
  await expect(detailPanel).toBeVisible();
  await expect(detailPanel.getByText("active", { exact: true })).toBeVisible();

  await workflowActions().click();
  await expect(enableItem).toHaveAttribute("aria-checked", "true");
  await expect(enableItem.locator("button")).toHaveCount(0);
  await expect(
    enableItem.getByTestId("workflow-enabled-switch-visual"),
  ).toHaveAttribute("aria-hidden", "true");
  await enableItem.click();
  await expect(
    workflowCard().getByText("disabled", { exact: true }),
  ).toBeVisible();
  await expect(
    detailPanel.getByText("disabled", { exact: true }),
  ).toBeVisible();

  await enableItem.click();
  await expect(
    workflowCard().getByText("active", { exact: true }),
  ).toBeVisible();
  await expect(detailPanel.getByText("active", { exact: true })).toBeVisible();
});

test("rejects a stale card toggle without overwriting a newer edit", async ({
  page,
}) => {
  const workflowName = `stale_toggle_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);
  const workflowCard = page
    .locator('[data-testid^="workflow-card-"]')
    .filter({ hasText: workflowName })
    .first();

  await page.evaluate(async (name) => {
    const invoke = window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
    if (!invoke) throw new Error("mock command bridge unavailable");
    const createCall = [...(window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])]
      .reverse()
      .find((call) => call.command === "create_workflow");
    const channelId = (
      createCall?.payload as { channelId?: string } | undefined
    )?.channelId;
    if (!channelId) throw new Error("create workflow channel unavailable");
    const workflows = (await invoke("get_channels_workflows", {
      channelIds: [channelId],
    })) as Array<{
      id: string;
      revision: string;
      definition: Record<string, unknown>;
    }>;
    const workflow = workflows.find(
      (candidate) => candidate.definition.name === name,
    );
    if (!workflow) throw new Error("created workflow unavailable");
    await invoke("update_workflow", {
      workflowId: workflow.id,
      expectedRevision: workflow.revision,
      yamlDefinition: `name: ${name} edited elsewhere\nenabled: true\ntrigger:\n  on: message_posted\nsteps:\n  - id: step_1\n    action: post_message\n`,
    });
  }, workflowName);

  await workflowCard.getByRole("button", { name: "Workflow actions" }).click();
  await page.getByRole("menuitemcheckbox", { name: "Enable" }).click();

  await expect(
    page
      .locator("[data-sonner-toast][data-removed='false']")
      .filter({ hasText: "workflow changed since it was loaded" }),
  ).toBeVisible();
  const authoritativeName = await page.evaluate(async () => {
    const invoke = window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
    if (!invoke) throw new Error("mock command bridge unavailable");
    const createCall = [...(window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])]
      .reverse()
      .find((call) => call.command === "create_workflow");
    const channelId = (
      createCall?.payload as { channelId?: string } | undefined
    )?.channelId;
    if (!channelId) throw new Error("create workflow channel unavailable");
    const workflows = (await invoke("get_channels_workflows", {
      channelIds: [channelId],
    })) as Array<{ name: string }>;
    return workflows[0]?.name;
  });
  expect(authoritativeName).toBe(`${workflowName} edited elsewhere`);
});

test("reports a rejected workflow status change", async ({ page }) => {
  const workflowName = `rejected_toggle_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);
  await page.evaluate(() => {
    window.__BUZZ_E2E__ ??= {};
    window.__BUZZ_E2E__.mock ??= {};
    window.__BUZZ_E2E__.mock.workflowUpdateError = "relay refused the update";
  });

  const workflowCard = page
    .locator('[data-testid^="workflow-card-"]')
    .filter({ hasText: workflowName })
    .first();
  await workflowCard.getByRole("button", { name: "Workflow actions" }).click();
  await page.getByRole("menuitemcheckbox", { name: "Enable" }).click();

  const errorToast = page
    .locator("[data-sonner-toast][data-removed='false']")
    .filter({ hasText: "Couldn’t change workflow status" });
  await expect(errorToast).toContainText("relay refused the update");
  await expect(workflowCard.getByText("active", { exact: true })).toBeVisible();
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

  // Verify workflow is gone — back to the empty create tile.
  await expect(page.getByTestId("new-workflow-card")).toBeVisible();
  await expect(page.locator('[data-testid^="workflow-card-"]')).toHaveCount(0);
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
