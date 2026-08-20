import { expect, test } from "@playwright/test";
import { parse as parseYaml } from "yaml";

import { installMockBridge } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

async function openCreateWorkflow(
  page: import("@playwright/test").Page,
  name: string,
) {
  await page.goto("/");
  await page.getByTestId("open-workflows-view").click();
  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  const channelList = page.getByTestId("channel-combobox-list");
  if (!(await channelList.isVisible())) {
    await dialog.getByRole("combobox", { name: "Channel" }).click();
  }
  await channelList
    .getByRole("option", { name: "agents", exact: true })
    .click();
  await dialog.getByRole("button", { name: "Edit workflow name" }).click();
  await dialog.getByRole("textbox", { name: "Workflow name" }).fill(name);
  await dialog.getByRole("button", { name: "Save workflow name" }).click();
  return dialog;
}

async function selectTrigger(
  page: import("@playwright/test").Page,
  dialog: import("@playwright/test").Locator,
  trigger: string,
) {
  await dialog.getByRole("button", { name: "Trigger event" }).click();
  await page.getByRole("menuitem", { name: trigger, exact: true }).click();
}

async function openTriggerInspector(
  dialog: import("@playwright/test").Locator,
  trigger: string,
) {
  const menu = dialog.getByRole("button", { name: "Trigger event" });
  if (!(await menu.isVisible())) {
    await dialog.getByRole("button", { name: `Trigger: ${trigger}` }).click();
  }
  await expect(menu).toBeVisible();
}

async function addMessageStep(
  page: import("@playwright/test").Page,
  dialog: import("@playwright/test").Locator,
) {
  await dialog.getByRole("button", { name: "Add step", exact: true }).click();
  await page.getByRole("menuitem", { name: "Send Message" }).click();
  await dialog.getByLabel("Message text").fill("Workflow notification");
}

async function reopenWorkflow(
  page: import("@playwright/test").Page,
  name: string,
) {
  const card = page
    .locator('[data-testid^="workflow-card-"]')
    .filter({ hasText: name });
  await card.getByRole("button", { name: "Workflow actions" }).click();
  await page.getByRole("menuitem", { name: "Edit" }).click();
  return page.getByRole("dialog", { name: "Edit workflow" });
}

test("round-trips schedule presets and saves a custom UTC cron", async ({
  page,
}) => {
  const name = `schedule_controls_${Date.now()}`;
  const dialog = await openCreateWorkflow(page, name);
  await selectTrigger(page, dialog, "Schedule");

  await expect(dialog.getByRole("radio", { name: "Daily" })).toBeChecked();
  await expect(dialog.getByLabel("Run time (UTC)")).toHaveValue("09:00");

  await dialog.getByText("Every 15 minutes", { exact: true }).click();
  await dialog.getByRole("tab", { name: "YAML" }).click();
  const yamlEditor = dialog.getByRole("textbox", { name: "Workflow YAML" });
  let definition = parseYaml(await yamlEditor.inputValue());
  expect(definition.trigger).toEqual({ on: "schedule", interval: "15m" });

  await dialog.getByRole("tab", { name: "Form" }).click();
  await openTriggerInspector(dialog, "Schedule");
  await expect(
    dialog.getByRole("radio", { name: "Every 15 minutes" }),
  ).toBeChecked();

  await dialog.getByText("Monthly", { exact: true }).click();
  await dialog.getByLabel("Day of month").selectOption("31");
  await expect(
    dialog.getByText("This schedule won’t run in some months."),
  ).toBeVisible();

  await dialog.getByText("Custom cron", { exact: true }).click();
  await dialog.getByRole("textbox", { name: "Minute", exact: true }).fill("5");
  await dialog.getByRole("textbox", { name: "Hour", exact: true }).fill("*/2");
  await dialog.getByRole("textbox", { name: "Day", exact: true }).fill("*");
  await dialog.getByRole("textbox", { name: "Month", exact: true }).fill("*");
  await dialog
    .getByRole("textbox", { name: "Weekday", exact: true })
    .fill("2-4");
  await expect(dialog.getByText(/UTC · Paste all 5 fields/)).toBeVisible();

  await dialog.getByRole("tab", { name: "YAML" }).click();
  definition = parseYaml(await yamlEditor.inputValue());
  expect(definition.trigger).toEqual({
    on: "schedule",
    cron: "5 */2 * * 2-4",
  });
  await dialog.getByRole("tab", { name: "Form" }).click();
  await openTriggerInspector(dialog, "Schedule");
  await expect(
    dialog.getByRole("radio", { name: "Custom cron" }),
  ).toBeChecked();
  await expect(
    dialog.getByRole("textbox", { name: "Weekday", exact: true }),
  ).toHaveValue("2-4");

  await addMessageStep(page, dialog);
  await dialog.getByRole("button", { name: "Create" }).click();
  const reopened = await reopenWorkflow(page, name);
  await openTriggerInspector(reopened, "Schedule");
  await expect(
    reopened.getByRole("radio", { name: "Custom cron" }),
  ).toBeChecked();
  await expect(
    reopened.getByRole("textbox", { name: "Minute", exact: true }),
  ).toHaveValue("5");
  await expect(
    reopened.getByRole("textbox", { name: "Weekday", exact: true }),
  ).toHaveValue("2-4");
});

test("round-trips and reopens structured message-text conditions", async ({
  page,
}) => {
  const name = `message_condition_${Date.now()}`;
  const text = 'deploy "buzz"\\path';
  const expression = 'str_ends_with(trigger_text, "deploy \\"buzz\\"\\\\path")';
  const dialog = await openCreateWorkflow(page, name);

  await dialog.getByText("ends with", { exact: true }).click();
  await dialog.getByLabel("Text to match").fill(text);
  await dialog.getByRole("tab", { name: "YAML" }).click();
  const yamlEditor = dialog.getByRole("textbox", { name: "Workflow YAML" });
  const definition = parseYaml(await yamlEditor.inputValue());
  expect(definition.trigger.filter).toBe(expression);

  await dialog.getByRole("tab", { name: "Form" }).click();
  await openTriggerInspector(dialog, "Message Posted");
  await expect(dialog.getByText("ends with", { exact: true })).toBeVisible();
  await expect(dialog.getByLabel("Text to match")).toHaveValue(text);
  await dialog.getByRole("tab", { name: "Advanced" }).click();
  await expect(dialog.getByLabel("Advanced expression")).toHaveValue(
    expression,
  );
  await dialog.getByRole("tab", { name: "Basic" }).click();

  await addMessageStep(page, dialog);
  await dialog.getByRole("button", { name: "Create" }).click();
  const reopened = await reopenWorkflow(page, name);
  await openTriggerInspector(reopened, "Message Posted");
  await expect(reopened.getByText("ends with", { exact: true })).toBeVisible();
  await expect(reopened.getByLabel("Text to match")).toHaveValue(text);
});

test("hides and clears message-text step conditions for schedule triggers", async ({
  page,
}) => {
  const dialog = await openCreateWorkflow(
    page,
    `schedule_step_condition_${Date.now()}`,
  );
  await addMessageStep(page, dialog);
  await dialog.getByRole("button", { name: "Run controls" }).click();
  await dialog.getByLabel("Text to match").fill("deploy");

  await dialog.getByRole("button", { name: "Trigger: Message Posted" }).click();
  await selectTrigger(page, dialog, "Schedule");
  await dialog.getByRole("button", { name: "Step 1: Send Message" }).click();
  await dialog.getByRole("button", { name: "Run controls" }).click();

  await expect(dialog.getByLabel("Text to match")).toHaveCount(0);
  await expect(
    dialog.getByRole("textbox", { name: "Timeout", exact: true }),
  ).toBeVisible();

  await dialog.getByRole("tab", { name: "YAML" }).click();
  const definition = parseYaml(
    await dialog.getByRole("textbox", { name: "Workflow YAML" }).inputValue(),
  );
  expect(definition.steps[0].if).toBeUndefined();
});

test("keeps advanced and malformed definitions lossless", async ({ page }) => {
  const advanced =
    'str_contains(trigger_text, "deploy") && trigger_author == "abc"';
  const dialog = await openCreateWorkflow(page, `lossless_${Date.now()}`);
  await dialog.getByRole("tab", { name: "YAML" }).click();
  const yamlEditor = dialog.getByRole("textbox", { name: "Workflow YAML" });
  const initialYaml = await yamlEditor.inputValue();
  await yamlEditor.fill(
    initialYaml.replace(
      "trigger:\n  on: message_posted",
      `trigger:\n  on: message_posted\n  filter: '${advanced}'`,
    ),
  );
  await dialog.getByRole("tab", { name: "Form" }).click();
  await openTriggerInspector(dialog, "Message Posted");
  await expect(dialog.getByRole("tab", { name: "Advanced" })).toHaveAttribute(
    "data-state",
    "active",
  );
  await expect(dialog.getByLabel("Advanced expression")).toHaveValue(advanced);
  await dialog.getByRole("tab", { name: "Basic" }).click();
  await expect(
    dialog.getByText(/advanced expression is active/i),
  ).toBeVisible();

  await dialog.getByRole("switch", { name: "Enable workflow" }).click();
  await dialog.getByRole("tab", { name: "YAML" }).click();
  expect(parseYaml(await yamlEditor.inputValue()).trigger.filter).toBe(
    advanced,
  );

  const malformedYaml = (await yamlEditor.inputValue()).replace(
    /trigger:\n {2}on: message_posted\n {2}filter:.*\n/,
    'trigger:\n  on: schedule\n  cron: "0 9 * * *"\n  interval: 1h\n',
  );
  await yamlEditor.fill(malformedYaml);
  await dialog.getByRole("tab", { name: "Form" }).click();
  await expect(
    dialog.getByText(/cannot specify both cron and interval/i),
  ).toBeVisible();
  await expect(yamlEditor).toHaveValue(malformedYaml);
});
