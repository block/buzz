import { expect, test, type Locator, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

const GENERATED_NAME = /^[a-z]+-[a-z]+-[a-z]+$/;

async function navigateToWorkflows(page: Page) {
  await page.goto("/");
  await page.getByTestId("open-workflows-view").click();
  await expect(page.getByTestId("workflows-view")).toBeVisible();
}

function readWorkflowName(dialog: Locator) {
  return dialog
    .getByRole("button", { name: "Edit workflow name" })
    .evaluate((button) =>
      (button.previousElementSibling?.textContent ?? "").trim(),
    );
}

async function selectWorkflowChannel(page: Page, dialog: Locator) {
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

async function openTriggerPane(dialog: Locator) {
  await dialog.getByRole("button", { name: /^Trigger: / }).click();
  await expect(
    dialog.getByRole("button", { name: "Trigger event" }),
  ).toBeVisible();
}

async function openStepPane(dialog: Locator, index: number) {
  await dialog
    .getByRole("button", { name: new RegExp(`^Step ${index}: `) })
    .click();
  await expect(dialog.getByTestId("workflow-node-inspector")).toContainText(
    `Step ${index}`,
  );
}

/**
 * Inserts a step right after the trigger. The insertion control only becomes
 * interactive on hover once the trigger is no longer the terminal node.
 */
async function addStepAfterTrigger(page: Page, dialog: Locator) {
  const ingress = dialog.getByTestId("workflow-node-ingress").first();
  await ingress.hover();
  await ingress.getByRole("button", { name: "Add step", exact: true }).click();
  await page.getByRole("menuitem", { name: "Send Message" }).click();
}

async function finishRiskyActivationIfShown(page: Page, dialog: Locator) {
  const confirmation = page.getByRole("alertdialog", {
    name: "This workflow may run often",
  });
  await Promise.race([
    confirmation.waitFor({ state: "visible" }),
    dialog.waitFor({ state: "hidden" }),
  ]);
  if (await confirmation.isVisible()) {
    await confirmation.getByRole("button", { name: "Turn on" }).click();
  }
  await expect(dialog).toBeHidden();
}

async function createNamedWorkflow(page: Page, name: string) {
  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  await expect(dialog).toBeVisible();

  await selectWorkflowChannel(page, dialog);
  await dialog.getByRole("button", { name: "Edit workflow name" }).click();
  await dialog.getByRole("textbox", { name: "Workflow name" }).fill(name);
  await dialog.getByRole("button", { name: "Save workflow name" }).click();

  await dialog.getByRole("button", { name: "Add first step" }).click();
  await dialog.getByRole("tab", { name: "YAML" }).click();
  const yamlEditor = dialog.getByRole("textbox", { name: "Workflow YAML" });
  const yaml = await yamlEditor.inputValue();
  await yamlEditor.fill(
    yaml.replace(
      "    action: send_message",
      '    action: send_message\n    text: "Workflow notification"',
    ),
  );
  await dialog.getByRole("button", { name: "Create workflow" }).click();
  await finishRiskyActivationIfShown(page, dialog);

  await expect(page.getByTestId("workflows-view")).toContainText(name);
}

function workflowCard(page: Page, name: string) {
  return page
    .locator('[data-testid^="workflow-card-"]')
    .filter({ hasText: name })
    .first();
}

test("keeps the generated name while moving between trigger and step panes", async ({
  page,
}) => {
  await navigateToWorkflows(page);
  await page.getByRole("button", { name: "Create Workflow" }).click();

  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  await selectWorkflowChannel(page, dialog);
  await expect.poll(() => readWorkflowName(dialog)).toMatch(GENERATED_NAME);
  const generatedName = await readWorkflowName(dialog);

  // Adding the first step lands on the step pane while the step is still
  // incomplete — the header must keep showing the workflow name.
  await dialog.getByRole("button", { name: "Add first step" }).click();
  await expect(dialog.getByTestId("workflow-node-inspector")).toContainText(
    "Step 1",
  );
  expect(await readWorkflowName(dialog)).toBe(generatedName);
  await expect(
    dialog.getByRole("button", { name: "Edit workflow name" }),
  ).toBeEnabled();

  await openTriggerPane(dialog);
  expect(await readWorkflowName(dialog)).toBe(generatedName);

  await openStepPane(dialog, 1);
  expect(await readWorkflowName(dialog)).toBe(generatedName);

  // A second, still-empty step must not clear the name either.
  await addStepAfterTrigger(page, dialog);
  await expect(dialog.getByTestId("workflow-node-inspector")).toContainText(
    "Step 1",
  );
  expect(await readWorkflowName(dialog)).toBe(generatedName);

  await openStepPane(dialog, 2);
  expect(await readWorkflowName(dialog)).toBe(generatedName);
});

test("renames stay visible across pane navigation before the step is filled in", async ({
  page,
}) => {
  const name = `renamed_workflow_${Date.now()}`;

  await navigateToWorkflows(page);
  await page.getByRole("button", { name: "Create Workflow" }).click();

  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  await selectWorkflowChannel(page, dialog);
  await dialog.getByRole("button", { name: "Add first step" }).click();
  await expect(dialog.getByTestId("workflow-node-inspector")).toContainText(
    "Step 1",
  );

  await dialog.getByRole("button", { name: "Edit workflow name" }).click();
  await dialog.getByRole("textbox", { name: "Workflow name" }).fill(name);
  await dialog.getByRole("button", { name: "Save workflow name" }).click();
  expect(await readWorkflowName(dialog)).toBe(name);

  await openTriggerPane(dialog);
  expect(await readWorkflowName(dialog)).toBe(name);

  await openStepPane(dialog, 1);
  expect(await readWorkflowName(dialog)).toBe(name);

  // Completing the step re-serializes the definition from the builder's form
  // state, which must have picked the rename up rather than writing back the
  // generated name it was holding when the rename happened.
  await dialog
    .getByTestId("workflow-node-inspector")
    .getByLabel("Message text")
    .fill("Workflow notification");
  expect(await readWorkflowName(dialog)).toBe(name);

  await dialog.getByRole("button", { name: "Create workflow" }).click();
  await finishRiskyActivationIfShown(page, dialog);
  await expect(page.getByTestId("workflows-view")).toContainText(name);
});

test("keeps a header disable across a later form edit", async ({ page }) => {
  const name = `disabled_workflow_${Date.now()}`;

  await navigateToWorkflows(page);
  await createNamedWorkflow(page, name);

  await workflowCard(page, name)
    .getByRole("button", { name: "Workflow actions" })
    .click();
  await page.getByRole("menuitem", { name: "Edit" }).click();

  const dialog = page.getByRole("dialog", { name: "Edit workflow" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("tab", { name: "Form" }).click();

  const enableItem = page.getByRole("menuitemcheckbox", { name: "Enable" });
  // The open menu aria-hides the dialog behind it, so it has to be dismissed
  // before the dialog can be addressed again.
  const closeActionsMenu = async () => {
    await page.keyboard.press("Escape");
    await expect(enableItem).not.toBeVisible();
  };

  // Add an empty step so the definition stops validating, then disable from the
  // header. Completing the step re-serializes the definition from the builder's
  // form state, which must carry the disable rather than the enabled state it
  // held before the header wrote it.
  await addStepAfterTrigger(page, dialog);
  await expect(dialog.getByTestId("workflow-node-inspector")).toContainText(
    "Step 1",
  );

  await dialog.getByRole("button", { name: "Workflow actions" }).click();
  await expect(enableItem).toHaveAttribute("aria-checked", "true");
  await enableItem.click();
  await expect(enableItem).toHaveAttribute("aria-checked", "false");
  await closeActionsMenu();

  await dialog.getByLabel("Message text").fill("Updated notification");

  await dialog.getByRole("button", { name: "Workflow actions" }).click();
  await expect(enableItem).toHaveAttribute("aria-checked", "false");
  await closeActionsMenu();

  await dialog.getByRole("button", { name: "Save changes" }).click();
  await expect(workflowCard(page, name)).toContainText("disabled");
});

test("keeps the saved name while editing an existing workflow", async ({
  page,
}) => {
  const name = `edit_title_${Date.now()}`;

  await navigateToWorkflows(page);
  await createNamedWorkflow(page, name);

  await workflowCard(page, name)
    .getByRole("button", { name: "Workflow actions" })
    .click();
  await page.getByRole("menuitem", { name: "Edit" }).click();

  const dialog = page.getByRole("dialog", { name: "Edit workflow" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("tab", { name: "Form" }).click();
  expect(await readWorkflowName(dialog)).toBe(name);

  await openTriggerPane(dialog);
  expect(await readWorkflowName(dialog)).toBe(name);

  await openStepPane(dialog, 1);
  expect(await readWorkflowName(dialog)).toBe(name);

  await addStepAfterTrigger(page, dialog);
  await expect(dialog.getByTestId("workflow-node-inspector")).toContainText(
    "Step 1",
  );
  expect(await readWorkflowName(dialog)).toBe(name);
  await expect(
    dialog.getByRole("button", { name: "Edit workflow name" }),
  ).toBeEnabled();

  await openStepPane(dialog, 2);
  expect(await readWorkflowName(dialog)).toBe(name);
});

test("keeps the copy name while duplicating an existing workflow", async ({
  page,
}) => {
  const name = `dup_title_${Date.now()}`;

  await navigateToWorkflows(page);
  await createNamedWorkflow(page, name);

  await workflowCard(page, name)
    .getByRole("button", { name: "Workflow actions" })
    .click();
  await page.getByRole("menuitem", { name: "Duplicate" }).click();

  const dialog = page.getByRole("dialog", { name: "Duplicate workflow" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("tab", { name: "Form" }).click();
  expect(await readWorkflowName(dialog)).toBe(`${name} (copy)`);

  await openStepPane(dialog, 1);
  expect(await readWorkflowName(dialog)).toBe(`${name} (copy)`);

  await addStepAfterTrigger(page, dialog);
  await expect(dialog.getByTestId("workflow-node-inspector")).toContainText(
    "Step 1",
  );
  expect(await readWorkflowName(dialog)).toBe(`${name} (copy)`);

  await openTriggerPane(dialog);
  expect(await readWorkflowName(dialog)).toBe(`${name} (copy)`);
});
