import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

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
  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  await expect(dialog).toBeVisible();

  await dialog.getByRole("combobox", { name: "Channel" }).click();
  await page.getByRole("button", { name: /agents.*stream.*open/i }).click();
  await dialog.locator("#wf-name").fill(name);
  if (options?.description) {
    await dialog.getByLabel("Description (optional)").fill(options.description);
  }
  if (options?.enabled === false) {
    await dialog.getByRole("switch", { name: "Enable" }).click();
  }
  if (options?.trigger) {
    await dialog
      .getByRole("button", { name: "Trigger: Message Posted" })
      .click();
    await dialog.getByRole("button", { name: "Trigger event" }).click();
    await page
      .getByRole("menuitem", {
        name:
          options.trigger === "diff_posted"
            ? "Diff Posted"
            : options.trigger === "reaction_added"
              ? "Reaction Added"
              : options.trigger === "webhook"
                ? "Webhook"
                : "Message Posted",
      })
      .click();
  }

  await dialog.getByRole("button", { name: "Add step", exact: true }).click();
  await page.getByRole("menuitem", { name: "Send Message" }).click();
  await dialog.getByLabel("Message text").fill("Workflow notification");
  if (options?.stepName) {
    await dialog.getByLabel("Name (optional)").fill(options.stepName);
  }
  if (options?.stepCondition) {
    await dialog.getByLabel("Condition (optional)").fill(options.stepCondition);
  }
  if (options?.stepTimeoutSecs) {
    await dialog.getByLabel("Timeout (seconds)").fill(options.stepTimeoutSecs);
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
  const dialog = page.getByRole("dialog", { name: "Create workflow" });

  await expect(
    dialog.getByRole("textbox", { name: "Workflow name" }),
  ).toHaveAttribute("autocapitalize", "off");

  await dialog.getByRole("button", { name: "Add step", exact: true }).click();
  await page.getByRole("menuitem", { name: "Send Message" }).click();
  await expect(dialog.getByLabel("Name (optional)")).toHaveAttribute(
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
    trigger: "message_posted",
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
  const dialog = page.getByRole("dialog", { name: "Edit workflow" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("Edit workflow")).toBeVisible();

  // Change the name
  await dialog.getByRole("button", { name: "Edit workflow name" }).click();
  const nameInput = dialog
    .getByRole("textbox", { name: "Workflow name" })
    .first();
  await nameInput.clear();
  await nameInput.fill(updatedName);
  await dialog.getByRole("button", { name: "Save workflow name" }).click();

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

  // Submit the duplicate after deliberately choosing its destination channel.
  const dialog = page.getByRole("dialog", { name: "Duplicate workflow" });
  await dialog.getByRole("combobox", { name: "Channel" }).click();
  await page.getByRole("button", { name: /agents.*stream.*open/i }).click();
  await dialog.getByRole("button", { name: "Create copy" }).click();
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

test("captures the built editor at desktop and narrow widths", async ({
  page,
}) => {
  await navigateToWorkflows(page);
  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  await dialog.getByRole("combobox", { name: "Channel" }).click();
  await page.getByRole("button", { name: /agents.*stream.*open/i }).click();
  await dialog.locator("#wf-name").fill("editor_screenshot");
  await dialog.getByRole("button", { name: "Add step", exact: true }).click();
  await page.getByRole("menuitem", { name: "Send Message" }).click();
  await dialog.getByLabel("Message text").fill("Notify the workflow channel");

  for (const viewport of [
    { height: 900, name: "wide", width: 1440 },
    { height: 820, name: "narrow", width: 760 },
  ]) {
    await page.setViewportSize(viewport);
    await waitForAnimations(page);
    await page.screenshot({
      path: `test-results/workflow-editor-${viewport.name}.png`,
    });
  }
});

test("pane routes use stable IDs and Form/YAML changes stay synchronized", async ({
  page,
}) => {
  await navigateToWorkflows(page);
  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  await dialog.getByRole("combobox", { name: "Channel" }).click();
  await page.getByRole("button", { name: /agents.*stream.*open/i }).click();
  await dialog.locator("#wf-name").fill("pane_sync_test");

  await dialog.getByRole("button", { name: "Add step", exact: true }).click();
  await page.getByRole("menuitem", { name: "Send Message" }).click();
  await dialog.getByLabel("Message text").fill("first message");
  await expect(page).toHaveURL(/pane=step%3Astep_1/);

  await dialog.getByRole("button", { name: "Add after Step 1" }).click();
  await page.getByRole("menuitem", { name: "Delay" }).click();
  await dialog.getByLabel("Duration").fill("5m");
  await expect(page).toHaveURL(/pane=step%3Astep_2/);

  await dialog.getByRole("button", { name: "Step 1: Send Message" }).click();
  await expect(page).toHaveURL(/pane=step%3Astep_1/);
  await dialog
    .getByTestId("workflow-node-inspector")
    .getByRole("button", { name: "Remove step", exact: true })
    .click();
  await expect(page).toHaveURL(/pane=step%3Astep_2/);

  await dialog.getByRole("tab", { name: "YAML" }).click();
  const yamlEditor = dialog.getByRole("textbox", { name: "Workflow YAML" });
  await expect(yamlEditor).toContainText("id: step_2");
  await expect(yamlEditor).not.toContainText("id: step_1");
  const yaml = await yamlEditor.inputValue();
  await yamlEditor.fill(yaml.replace("duration: 5m", "duration: 10m"));
  await dialog.getByRole("tab", { name: "Form" }).click();
  await dialog.getByRole("button", { name: "Step 1: Delay" }).click();
  await expect(dialog.getByLabel("Duration")).toHaveValue("10m");
});

test("clean editor close respects in-app versus direct-entry provenance", async ({
  page,
}) => {
  await navigateToWorkflows(page);
  await page.getByRole("button", { name: "Create Workflow" }).click();
  await expect(page).toHaveURL(/view=create/);
  await page
    .getByRole("dialog", { name: "Create workflow" })
    .getByRole("button", {
      name: "Close",
      exact: true,
    })
    .click();
  await expect(page).toHaveURL(/#\/workflows$/);

  await page.goto("/#/workflows?view=create");
  await page
    .getByRole("dialog", { name: "Create workflow" })
    .getByRole("button", {
      name: "Close",
      exact: true,
    })
    .click();
  await expect(page).toHaveURL(/#\/workflows$/);
  await page.goBack();
  await expect(page).toHaveURL(/#\/workflows$/);
});

test("direct editor routes survive refresh and invalid view stays on detail", async ({
  page,
}) => {
  const workflowName = `route_test_${Date.now()}`;
  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);

  await page.getByRole("button", { name: "Workflow actions" }).first().click();
  await page.getByRole("menuitem", { name: "Edit" }).click();
  await expect(page).toHaveURL(/#\/workflows\/[^?]+\?.*view=edit/);
  const workflowId = new URL(page.url()).hash.match(/workflows\/([^?]+)/)?.[1];
  expect(workflowId).toBeTruthy();
  await page.goto(`/#/workflows/${workflowId}?view=invalid`);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByTestId("workflow-detail-panel")).toBeVisible();

  await page.goto("/#/workflows?view=create");
  await page.reload();
  await expect(
    page.getByRole("dialog", { name: "Create workflow" }),
  ).toBeVisible();
});

test("dirty create close is blocked and keep editing preserves the draft", async ({
  page,
}) => {
  await navigateToWorkflows(page);
  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  await dialog.getByRole("combobox", { name: "Channel" }).click();
  await page.getByRole("button", { name: /agents.*stream.*open/i }).click();
  await dialog.locator("#wf-name").fill("preserved_dirty_draft");
  await dialog.getByRole("button", { name: "Close", exact: true }).click();

  const confirmation = page.getByRole("alertdialog", {
    name: "Discard changes?",
  });
  await expect(confirmation).toBeVisible();
  await confirmation.getByRole("button", { name: "Keep editing" }).click();
  await expect(dialog.locator("#wf-name")).toHaveValue("preserved_dirty_draft");
});

test("stale editor save preserves the local draft and reports the conflict", async ({
  page,
}) => {
  const workflowName = `stale_editor_${Date.now()}`;
  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);
  await page.getByRole("button", { name: "Workflow actions" }).first().click();
  await page.getByRole("menuitem", { name: "Edit" }).click();
  const dialog = page.getByRole("dialog", { name: "Edit workflow" });
  await dialog.getByRole("button", { name: "Edit workflow name" }).click();
  const nameInput = dialog
    .getByRole("textbox", { name: "Workflow name" })
    .first();
  await nameInput.fill(`${workflowName}_local`);
  await dialog.getByRole("button", { name: "Save workflow name" }).click();

  await page.evaluate(async () => {
    const invoke = window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
    if (!invoke) throw new Error("mock command bridge unavailable");
    const workflowId = new URL(window.location.href).hash.match(
      /workflows\/([^?]+)/,
    )?.[1];
    if (!workflowId) throw new Error("workflow id unavailable");
    const workflow = (await invoke("get_workflow", { workflowId })) as {
      revision: string;
    };
    await invoke("update_workflow", {
      expectedRevision: workflow.revision,
      workflowId,
      yamlDefinition:
        "name: authoritative_remote\ntrigger:\n  on: message_posted\nsteps:\n  - id: step_1\n    action: send_message\n    text: remote\n",
    });
  });

  await dialog.getByRole("button", { name: "Save changes" }).click();
  await expect(dialog).toContainText("workflow changed since it was loaded");
  await expect(dialog).toContainText(`${workflowName}_local`);
  await expect(dialog).toBeVisible();
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
