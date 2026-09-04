import { expect, test } from "@playwright/test";
import { parse as parseYaml } from "yaml";

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

async function selectWorkflowChannel(
  page: import("@playwright/test").Page,
  dialog: import("@playwright/test").Locator,
) {
  const channelList = page.getByTestId("channel-combobox-list");
  if (!(await channelList.isVisible())) {
    await dialog.getByRole("combobox", { name: "Channel" }).click();
  }
  await expect(channelList).toBeVisible();
  await waitForAnimations(page);
  await channelList
    .getByRole("option", { name: "agents", exact: true })
    .click();
}

async function editWorkflowName(
  dialog: import("@playwright/test").Locator,
  name: string,
) {
  await dialog.getByRole("button", { name: "Edit workflow name" }).click();
  const input = dialog.getByRole("textbox", { name: "Workflow name" });
  await input.fill(name);
  await dialog.getByRole("button", { name: "Save workflow name" }).click();
  await expect(input).not.toBeVisible();
  await expect(dialog).toContainText(name);
  await expect(
    dialog.page().getByTestId("channel-combobox-list"),
  ).not.toBeVisible();
}

async function createWorkflow(
  page: import("@playwright/test").Page,
  name: string,
  options?: {
    description?: string;
    enabled?: boolean;
    trigger?: string;
    stepConditionText?: string;
    stepName?: string;
    stepTimeoutSecs?: string;
  },
) {
  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  await expect(dialog).toBeVisible();

  await selectWorkflowChannel(page, dialog);
  await editWorkflowName(dialog, name);
  if (options?.description) {
    await dialog.getByRole("tab", { name: "YAML" }).click();
    const yamlEditor = dialog.getByRole("textbox", { name: "Workflow YAML" });
    const yaml = await yamlEditor.inputValue();
    await yamlEditor.fill(
      yaml.replace(
        `name: ${name}`,
        `name: ${name}\ndescription: ${JSON.stringify(options.description)}`,
      ),
    );
    await dialog.getByRole("tab", { name: "Form" }).click();
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
    await dialog.getByRole("button", { name: "Step details" }).click();
    await dialog.getByLabel("Name (optional)").fill(options.stepName);
  }
  if (options?.stepConditionText || options?.stepTimeoutSecs) {
    await dialog.getByRole("button", { name: "Run controls" }).click();
  }
  if (options?.stepConditionText) {
    await dialog.getByLabel("Text to match").fill(options.stepConditionText);
  }
  if (options?.stepTimeoutSecs) {
    await dialog
      .getByRole("textbox", { name: "Timeout", exact: true })
      .fill(options.stepTimeoutSecs);
  }

  await dialog.getByRole("button", { name: "Create" }).click();
  if (!options?.trigger || options.trigger === "message_posted") {
    const activationConfirmation = page.getByRole("alertdialog", {
      name: "This workflow may run often",
    });
    await activationConfirmation
      .getByRole("button", {
        name: options?.enabled === false ? "Keep off" : "Turn on",
      })
      .click();
  }

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

test("creates a narrowly triggered workflow without an activation warning", async ({
  page,
}) => {
  await navigateToWorkflows(page);
  await createWorkflow(page, `safe_webhook_${Date.now()}`, {
    trigger: "webhook",
  });

  await expect(
    page.getByTestId("workflow-activation-confirmation"),
  ).toHaveCount(0);
  const yaml = await page.evaluate(() => {
    const call = [...(window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])]
      .reverse()
      .find((candidate) => candidate.command === "create_workflow");
    return (call?.payload as { yamlDefinition?: string } | undefined)
      ?.yamlDefinition;
  });
  expect(parseYaml(yaml ?? "").enabled).not.toBe(false);
});

test("creation reveals the trigger pane only after a one-shot channel pick", async ({
  page,
}) => {
  await navigateToWorkflows(page);
  await page.getByRole("button", { name: "Create Workflow" }).click();

  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  const channelList = page.getByTestId("channel-combobox-list");
  await expect(channelList).toBeVisible();
  await expect(dialog.getByTestId("workflow-node-inspector")).toHaveCount(0);

  await page.keyboard.press("Escape");
  await expect(channelList).not.toBeVisible();
  await expect
    .poll(() => dialog.getByText("Untitled workflow", { exact: true }).count())
    .toBe(0);
  await expect(channelList).not.toBeVisible();

  await dialog.getByRole("combobox", { name: "Channel" }).click();
  await channelList
    .getByRole("option", { name: "agents", exact: true })
    .click();
  await expect(dialog.getByTestId("workflow-node-inspector")).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "Trigger event" }),
  ).toBeVisible();
});

test("Escape closes the selected inspector before the workflow modal", async ({
  page,
}) => {
  for (const width of [760, 1280]) {
    await page.setViewportSize({ width, height: 820 });
    await page.goto(
      "/#/workflows?view=create&channel=94a444a4-c0a3-5966-ab05-530c6ddc2301&pane=trigger",
    );

    const dialog = page.getByRole("dialog", { name: "Create workflow" });
    const inspector = dialog.getByTestId("workflow-node-inspector");
    await expect(dialog).toBeVisible();
    await expect(inspector).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(inspector).not.toBeVisible();
    await expect(dialog).toBeVisible();
    await expect(page).toHaveURL(/view=create/);

    await page.keyboard.press("Escape");
    await expect(dialog).not.toBeVisible();
    await expect(page).toHaveURL(/#\/workflows$/);
  }
});

test("creates a workflow via the form builder", async ({ page }) => {
  const workflowName = `test_workflow_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);

  // Verify workflow appears in the list
  await expect(page.getByTestId("workflows-view")).toContainText(workflowName);
});

test("accepts the generated name through the first form mutation", async ({
  page,
}) => {
  await navigateToWorkflows(page);
  await page.getByRole("button", { name: "Create Workflow" }).click();

  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  await selectWorkflowChannel(page, dialog);
  const generatedName = (
    await dialog
      .getByRole("button", { name: "Edit workflow name" })
      .evaluate((button) => button.previousElementSibling?.textContent ?? "")
  ).trim();
  expect(generatedName).toMatch(/^[a-z]+-[a-z]+-[a-z]+$/);

  await dialog.getByRole("button", { name: "Add first step" }).click();
  await dialog.getByRole("button", { name: "Create workflow" }).click();
  const activationConfirmation = page.getByRole("alertdialog", {
    name: "This workflow may run often",
  });
  await activationConfirmation.getByRole("button", { name: "Turn on" }).click();

  await expect(page.getByTestId("workflows-view")).toContainText(generatedName);
});

test("disables autocapitalization in the workflow form", async ({ page }) => {
  await navigateToWorkflows(page);

  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog", { name: "Create workflow" });

  await selectWorkflowChannel(page, dialog);
  await dialog.getByRole("button", { name: "Edit workflow name" }).click();
  await expect(
    dialog.getByRole("textbox", { name: "Workflow name" }),
  ).toHaveAttribute("autocapitalize", "off");

  await dialog.getByRole("button", { name: "Add step", exact: true }).click();
  await page.getByRole("menuitem", { name: "Send Message" }).click();
  await dialog.getByRole("button", { name: "Step details" }).click();
  await expect(dialog.getByLabel("Name (optional)")).toHaveAttribute(
    "autocapitalize",
    "off",
  );
});

test("shows executable guidance for diff trigger conditions", async ({
  page,
}) => {
  await navigateToWorkflows(page);

  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  await selectWorkflowChannel(page, dialog);
  await dialog.getByRole("button", { name: "Trigger event" }).click();
  await page
    .getByRole("menuitem", { name: "Diff Posted", exact: true })
    .click();

  await expect(dialog.getByLabel("Diff text")).toHaveAttribute(
    "placeholder",
    "e.g. deploy",
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

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName, {
    enabled: false,
    trigger: "diff_posted",
    stepName: "Notify reviewers",
    stepConditionText: "src/",
    stepTimeoutSecs: "45s",
  });

  const card = page
    .locator('[data-testid^="workflow-card-"]')
    .filter({ hasText: workflowName })
    .first();
  await expect(card.locator("h3")).toHaveText(
    "When a diff is posted, send “Workflow notification”",
  );
  await expect(card).toContainText(workflowName);
  await expect(
    card.getByRole("switch", { name: "Disable workflow" }),
  ).toBeChecked();
});

test("enables and disables a workflow from its card status toggle", async ({
  page,
}) => {
  const workflowName = `toggle_workflow_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);

  const workflowCard = () =>
    page
      .locator('[data-testid^="workflow-card-"]')
      .filter({ hasText: workflowName })
      .first();

  const disable = workflowCard().getByRole("switch", {
    name: "Disable workflow",
  });
  await expect(disable).toBeChecked();
  await disable.click();
  const enable = workflowCard().getByRole("switch", {
    name: "Enable workflow",
  });
  await expect(enable).not.toBeChecked();
  await enable.click();
  const activationConfirmation = page.getByRole("alertdialog", {
    name: "This workflow may run often",
  });
  await expect(activationConfirmation).toBeVisible();
  await activationConfirmation.getByRole("button", { name: "Turn on" }).click();
  await expect(
    workflowCard().getByRole("switch", { name: "Disable workflow" }),
  ).toBeChecked();

  await workflowCard()
    .getByRole("button", { name: "Workflow actions" })
    .click();
  await expect(
    page.getByRole("menuitemcheckbox", { name: "Enable" }),
  ).toHaveCount(0);
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

  await workflowCard.getByRole("switch", { name: "Disable workflow" }).click();

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
  await workflowCard.getByRole("switch", { name: "Disable workflow" }).click();

  const errorToast = page
    .locator("[data-sonner-toast][data-removed='false']")
    .filter({ hasText: "Couldn’t change workflow status" });
  await expect(errorToast).toContainText("relay refused the update");
  await expect(
    workflowCard.getByRole("switch", { name: "Disable workflow" }),
  ).toBeChecked();
});

test("shows the webhook secret dialog after saving a webhook workflow", async ({
  page,
}) => {
  const workflowName = `webhook_workflow_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName, {
    trigger: "webhook",
  });

  await expect(page.getByText("Webhook ready")).toBeVisible();
  await expect(page.getByRole("button", { name: "Copy URL" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Copy Secret" })).toBeVisible();

  await expect(page.getByText(/•{24}/)).toBeVisible();
  await page.goBack();
  const confirmation = page.getByRole("alertdialog", {
    name: "Continue without this secret?",
  });
  await expect(confirmation).toBeVisible();
  await confirmation.getByRole("button", { name: "Go back" }).click();
  await expect(page.getByText("Webhook ready")).toBeVisible();

  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await confirmation.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByText("Webhook ready")).not.toBeVisible();
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
  await selectWorkflowChannel(page, dialog);
  await dialog.getByRole("button", { name: "Create copy" }).click();
  const activationConfirmation = page.getByRole("alertdialog", {
    name: "This workflow may run often",
  });
  await expect(activationConfirmation).toBeVisible();
  await activationConfirmation.getByRole("button", { name: "Back" }).click();
  await expect(dialog).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
            (call) => call.command === "create_workflow",
          ).length,
      ),
    )
    .toBe(1);
  await dialog.getByRole("button", { name: "Create copy" }).click();
  await activationConfirmation
    .getByRole("button", { name: "Keep off" })
    .click();
  await expect(page.getByRole("dialog")).not.toBeVisible();

  const copiedYaml = await page.evaluate(() => {
    const calls = (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
      (call) => call.command === "create_workflow",
    );
    return (calls.at(-1)?.payload as { yamlDefinition?: string } | undefined)
      ?.yamlDefinition;
  });
  expect(parseYaml(copiedYaml ?? "").enabled).toBe(false);

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

test("deleting the open workflow closes its editor", async ({ page }) => {
  const workflowName = `delete_open_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);
  await page.getByRole("button", { name: `View ${workflowName}` }).click();

  const detailDialog = page.getByRole("dialog", { name: "Edit workflow" });
  await expect(detailDialog).toBeVisible();
  await detailDialog.getByRole("button", { name: "Workflow actions" }).click();
  await page.getByRole("menuitem", { name: "Delete" }).click();
  await expect(page.getByRole("alertdialog")).toContainText(workflowName);
  await page.getByRole("button", { name: "Delete" }).click();

  await expect(detailDialog).toHaveCount(0);
  await expect(page).toHaveURL(/#\/workflows(?:\?|$)/);
  await expect(page.getByTestId("new-workflow-card")).toBeVisible();
  await expect(page.locator('[data-testid^="workflow-card-"]')).toHaveCount(0);
});

test("rejected deletion keeps the confirmation, editor, and draft", async ({
  page,
}) => {
  const workflowName = `delete_rejected_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);
  await page.getByRole("button", { name: `View ${workflowName}` }).click();
  const editor = page.getByRole("dialog", { name: "Edit workflow" });
  await editor.getByRole("tab", { name: "YAML" }).click();
  const yamlEditor = editor.getByRole("textbox", { name: "Workflow YAML" });
  await yamlEditor.fill(
    (await yamlEditor.inputValue()).replace(
      workflowName,
      `${workflowName}_draft`,
    ),
  );
  await page.evaluate(() => {
    window.__BUZZ_E2E__ ??= {};
    window.__BUZZ_E2E__.mock ??= {};
    window.__BUZZ_E2E__.mock.workflowDeleteError = "relay refused deletion";
  });

  await editor.getByRole("button", { name: "Workflow actions" }).click();
  await page.getByRole("menuitem", { name: "Delete" }).click();
  const confirmation = page.getByRole("alertdialog", {
    name: "Delete workflow?",
  });
  await confirmation.getByRole("button", { name: "Delete" }).click();

  await expect(confirmation).toBeVisible();
  await expect(confirmation.getByRole("alert")).toContainText(
    "relay refused deletion",
  );
  await confirmation.getByRole("button", { name: "Cancel" }).click();
  await expect(editor).toBeVisible();
  await expect(yamlEditor).toContainText(`${workflowName}_draft`);
});

test("captures the built editor at desktop and narrow widths", async ({
  page,
}) => {
  await navigateToWorkflows(page);
  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  await selectWorkflowChannel(page, dialog);
  await editWorkflowName(dialog, "editor_screenshot");
  await dialog.getByRole("button", { name: "Add step", exact: true }).click();
  await page.getByRole("menuitem", { name: "Send Message" }).click();
  await dialog.getByLabel("Message text").fill("Notify the workflow channel");
  const inspector = dialog.getByTestId("workflow-node-inspector");

  for (const viewport of [
    { height: 900, name: "wide", width: 1440 },
    { height: 820, name: "narrow", width: 760 },
  ]) {
    await page.setViewportSize(viewport);
    await inspector.getByRole("button", { name: "Run controls" }).click();
    await waitForAnimations(page);
    await page.screenshot({
      path: `test-results/workflow-editor-${viewport.name}-run-controls.png`,
    });
    await inspector.getByRole("button", { name: "Step details" }).click();
    await waitForAnimations(page);
    await page.screenshot({
      path: `test-results/workflow-editor-${viewport.name}-step-details.png`,
    });
    await inspector.getByRole("button", { name: "Step details" }).click();
  }
});

test("preserves final sequence affordances and responsive inspector behavior", async ({
  page,
}) => {
  await navigateToWorkflows(page);
  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  await selectWorkflowChannel(page, dialog);
  await editWorkflowName(dialog, "sequence_parity_test");
  await dialog.getByRole("button", { name: "Add step", exact: true }).click();
  await page.getByRole("menuitem", { name: "Send Message" }).click();

  const inspector = dialog.getByTestId("workflow-node-inspector");
  const runControls = inspector.getByRole("button", { name: "Run controls" });
  const stepDetails = inspector.getByRole("button", { name: "Step details" });
  await expect(runControls).toHaveAttribute("aria-expanded", "false");
  await expect(stepDetails).toHaveAttribute("aria-expanded", "false");
  await expect(inspector.getByLabel("Text to match")).toHaveCount(0);
  await expect(inspector.getByLabel("Name (optional)")).toHaveCount(0);

  await runControls.click();
  await expect(runControls).toHaveAttribute("aria-expanded", "true");
  await inspector.getByLabel("Text to match").fill("deploy");
  const timeoutField = inspector.getByRole("textbox", {
    name: "Timeout",
    exact: true,
  });
  const timeoutSlider = inspector.getByRole("slider", {
    name: "Timeout slider",
  });
  await expect(timeoutSlider).toHaveAttribute("aria-valuetext", "5m");
  await timeoutField.fill("1h 2s");
  await expect(timeoutSlider).toHaveAttribute("aria-valuetext", "1h");
  await timeoutField.fill("45s");
  await expect(runControls).toContainText("Conditional · 45s");

  await stepDetails.click();
  await expect(runControls).toHaveAttribute("aria-expanded", "false");
  await expect(stepDetails).toHaveAttribute("aria-expanded", "true");
  await expect(inspector.getByLabel("Text to match")).toHaveCount(0);
  await inspector.getByLabel("Name (optional)").fill("Notify deployers");
  await expect(stepDetails).toContainText("Notify deployers");
  await expect(inspector.getByLabel("Step ID")).toHaveValue("step_1");

  await expect(dialog.getByText("End", { exact: true })).toHaveCount(0);
  const ingresses = dialog.getByTestId("workflow-node-ingress");
  await expect(ingresses).toHaveCount(2);
  await expect(
    ingresses.first().locator("svg.lucide-arrow-down"),
  ).toBeVisible();
  await expect(ingresses.last().locator("svg.lucide-arrow-down")).toHaveCount(
    0,
  );
  await expect(
    ingresses.last().getByRole("button", { name: "Add after Step 1" }),
  ).toBeVisible();

  const stepNode = dialog.getByRole("button", { name: /^Step 1:/ });
  const removeStep = dialog.getByRole("button", { name: "Remove Step 1" });
  await expect(removeStep).toHaveClass(/-right-8/);
  await stepNode.hover();
  await expect(removeStep).toBeVisible();

  await page.setViewportSize({ width: 760, height: 820 });
  await expect(
    dialog.getByTestId("workflow-node-inspector-backdrop"),
  ).toBeVisible();
  await dialog.getByTestId("workflow-node-inspector-backdrop").click({
    position: { x: 20, y: 20 },
  });
  await expect(dialog.getByTestId("workflow-node-inspector")).not.toBeVisible();
});

test("pane routes use stable IDs and Form/YAML changes stay synchronized", async ({
  page,
}) => {
  await navigateToWorkflows(page);
  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  await selectWorkflowChannel(page, dialog);
  await editWorkflowName(dialog, "pane_sync_test");

  await dialog.getByRole("button", { name: "Add step", exact: true }).click();
  await page.getByRole("menuitem", { name: "Send Message" }).click();
  await dialog.getByLabel("Message text").fill("first message");
  await expect(page).toHaveURL(/pane=step%3Astep_1/);

  await dialog.getByRole("button", { name: "Add after Step 1" }).click();
  await page.getByRole("menuitem", { name: "Delay" }).click();
  await dialog.getByLabel("Duration").fill("5m");
  await expect(page).toHaveURL(/pane=step%3Astep_2/);

  await dialog.getByRole("button", { name: /^Step 1:/ }).click();
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
  await dialog.getByRole("button", { name: /^Step 1:/ }).click();
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

test("direct workflow routes survive refresh and invalid view opens detail", async ({
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
  const detailDialog = page.getByRole("dialog", { name: "Edit workflow" });
  await expect(detailDialog).toBeVisible();
  await expect(detailDialog).toContainText(workflowName);
  await expect(
    detailDialog.getByRole("button", { name: "Run history" }),
  ).toHaveCount(0);

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
  await selectWorkflowChannel(page, dialog);
  await editWorkflowName(dialog, "preserved_dirty_draft");
  await dialog.getByRole("button", { name: "Close", exact: true }).click();

  const confirmation = page.getByRole("alertdialog", {
    name: "Discard changes?",
  });
  await expect(confirmation).toBeVisible();
  await confirmation.getByRole("button", { name: "Keep editing" }).click();
  await expect(dialog).toContainText("preserved_dirty_draft");
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

test("unsupported workflow opens canonical YAML without a fabricated sequence", async ({
  page,
}) => {
  const workflowName = `unsupported_detail_${Date.now()}`;
  await navigateToWorkflows(page);
  const workflowId = await page.evaluate(async (name) => {
    const invoke = window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
    if (!invoke) throw new Error("mock command bridge unavailable");
    const workflow = (await invoke("create_workflow", {
      channelId: "94a444a4-c0a3-5966-ab05-530c6ddc2301",
      yamlDefinition: `name: ${name}\ntrigger:\n  on: message_posted\n  legacy_filter:\n    author: alice\nsteps:\n  - id: legacy_step\n    action: send_message\n    text: preserve me\n`,
    })) as { id: string };
    return workflow.id;
  }, workflowName);

  await page.goto(`/#/workflows/${workflowId}`);
  const detailDialog = page.getByRole("dialog", { name: "Edit workflow" });
  await expect(detailDialog).toBeVisible();
  await expect(detailDialog).toContainText(workflowName);
  await expect(detailDialog.getByRole("tab", { name: "YAML" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  const yamlEditor = detailDialog.getByRole("textbox", {
    name: "Workflow YAML",
  });
  await expect(yamlEditor).toContainText("legacy_filter:");
  await expect(yamlEditor).toContainText("author: alice");
  await expect(yamlEditor).toContainText("id: legacy_step");
  await expect(yamlEditor).toContainText("text: preserve me");
  await expect(
    detailDialog.getByRole("button", { name: "Trigger: Message Posted" }),
  ).toHaveCount(0);
  await expect(
    detailDialog.getByRole("button", { name: "Step 1: Send Message" }),
  ).toHaveCount(0);
});

test("card click opens the animated trigger inspector, triggers a run, and hides unsupported run history", async ({
  page,
}) => {
  const workflowName = `card_editor_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);

  const inspectorEntryAnimation = page.evaluate(
    () =>
      new Promise<boolean>((resolve) => {
        const timeoutId = window.setTimeout(() => {
          observer.disconnect();
          resolve(false);
        }, 1_000);
        const observer = new MutationObserver(() => {
          const inspector = document.querySelector<HTMLElement>(
            '[data-testid="workflow-node-inspector"]',
          );
          if (!inspector) return;

          observer.disconnect();
          window.clearTimeout(timeoutId);
          const initialRect = inspector.getBoundingClientRect();
          const initialStyles = getComputedStyle(inspector);
          const initial = {
            opacity: initialStyles.opacity,
            transform: initialStyles.transform,
            width: initialRect.width,
          };
          requestAnimationFrame(() => {
            requestAnimationFrame(() => {
              const nextRect = inspector.getBoundingClientRect();
              const nextStyles = getComputedStyle(inspector);
              resolve(
                nextRect.width !== initial.width ||
                  nextStyles.opacity !== initial.opacity ||
                  nextStyles.transform !== initial.transform,
              );
            });
          });
        });
        observer.observe(document.body, { childList: true, subtree: true });
      }),
  );
  await page.getByRole("button", { name: `View ${workflowName}` }).click();

  await expect(page).toHaveURL(/#\/workflows\/[^?]+\?pane=trigger$/);
  const detailDialog = page.getByRole("dialog", { name: "Edit workflow" });
  await expect(detailDialog).toBeVisible();
  await expect(detailDialog).toContainText(workflowName);
  await expect(inspectorEntryAnimation).resolves.toBe(true);
  await expect(page.getByTestId("workflow-node-inspector")).toBeVisible();
  await expect(
    detailDialog.getByRole("button", { name: "Trigger: Message Posted" }),
  ).toHaveAttribute("aria-pressed", "true");
  const triggerCallsBefore = await page.evaluate(
    () =>
      window.__BUZZ_E2E_COMMAND_PAYLOADS__?.filter(
        (call) => call.command === "trigger_workflow",
      ).length ?? 0,
  );
  await detailDialog.getByRole("button", { name: "Workflow actions" }).click();
  await page.getByRole("menuitem", { name: "Trigger" }).click();
  await expect
    .poll(async () =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_COMMAND_PAYLOADS__?.filter(
            (call) => call.command === "trigger_workflow",
          ).length ?? 0,
      ),
    )
    .toBe(triggerCallsBefore + 1);
  await expect(page.getByTestId("workflow-detail-panel")).toHaveCount(0);
  await expect(
    detailDialog.getByRole("button", { name: "Run history" }),
  ).toHaveCount(0);
});

test("missing workflow routes show an unavailable modal with close and retry", async ({
  page,
}) => {
  await navigateToWorkflows(page);
  await page.goto("/#/workflows/missing-workflow?view=edit");

  const unavailable = page.getByRole("dialog", {
    name: "Workflow unavailable",
  });
  await expect(unavailable).toBeVisible();
  await expect(
    unavailable.getByRole("button", { name: "Retry" }),
  ).toBeVisible();
  await unavailable.getByRole("button", { name: "Close" }).click();
  await expect(page).toHaveURL(/#\/workflows$/);
});
