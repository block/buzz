import { expect, test } from "@playwright/test";
import { parse as parseYaml, stringify as stringifyYaml } from "yaml";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

declare global {
  interface Window {
    __BUZZ_WORKFLOW_BATCH_CALLS__?: {
      eventBatches: number[];
      userBatches: number[];
    };
  }
}

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
) {
  const menu = dialog.getByRole("button", { name: "Trigger event" });
  if (!(await menu.isVisible())) {
    await dialog.getByRole("button", { name: /^Trigger:/ }).click();
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

async function createEnabled(
  page: import("@playwright/test").Page,
  dialog: import("@playwright/test").Locator,
) {
  await dialog.getByRole("button", { name: "Create" }).click();
  const confirmation = page.getByRole("alertdialog", {
    name: "This workflow may run often",
  });
  if (await confirmation.isVisible()) {
    await confirmation.getByRole("button", { name: "Turn on" }).click();
  }
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

test("confirms activation after create and preserves a safe disabled path", async ({
  page,
}) => {
  const name = `activation_confirmation_${Date.now()}`;
  const dialog = await openCreateWorkflow(page, name);
  await addMessageStep(page, dialog);

  await expect(
    dialog.getByRole("switch", { name: "Enable workflow" }),
  ).toHaveCount(0);
  await dialog.getByRole("button", { name: "Create" }).click();
  const confirmation = page.getByRole("alertdialog", {
    name: "This workflow may run often",
  });
  await expect(confirmation).toBeVisible();
  await expect(
    confirmation.getByRole("button", { name: "Back" }),
  ).toBeFocused();

  await confirmation.getByRole("button", { name: "Back" }).click();
  await expect(confirmation).toBeHidden();
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
    .toBe(0);

  await dialog.getByRole("button", { name: "Create" }).click();
  await confirmation.getByRole("button", { name: "Keep off" }).click();
  await expect(dialog).toBeHidden();

  const yaml = await page.evaluate(() => {
    const call = [...(window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])]
      .reverse()
      .find((candidate) => candidate.command === "create_workflow");
    return (call?.payload as { yamlDefinition?: string } | undefined)
      ?.yamlDefinition;
  });
  expect(parseYaml(yaml ?? "").enabled).toBe(false);
  const card = page
    .locator('[data-testid^="workflow-card-"]')
    .filter({ hasText: name })
    .first();
  await expect(
    card.getByRole("switch", { name: "Enable workflow" }),
  ).not.toBeChecked();
});

test("inserts template variables with keyboard control and restores the caret", async ({
  page,
}) => {
  const dialog = await openCreateWorkflow(page, "template_variables_keyboard");
  await dialog.getByRole("button", { name: "Add step", exact: true }).click();
  await page.getByRole("menuitem", { name: "Send Message" }).click();

  const textarea = dialog.getByLabel("Message text");
  const listbox = page.getByRole("listbox");
  await textarea.fill("Hello {{trig");
  await expect(listbox).toBeVisible();
  await expect(listbox.getByRole("option")).toHaveCount(5);
  await waitForAnimations(page);
  expect(await page.locator("body").screenshot()).toMatchSnapshot(
    "workflow-template-variable-autocomplete.png",
  );

  await textarea.press("ArrowUp");
  await expect(textarea).toHaveAttribute(
    "aria-activedescendant",
    /-variables-option-4$/,
  );
  await textarea.press("ArrowDown");
  await expect(textarea).toHaveAttribute(
    "aria-activedescendant",
    /-variables-option-0$/,
  );
  await textarea.press("Enter");
  await expect(textarea).toHaveValue("Hello {{trigger.text}}");
  await expect(textarea).toBeFocused();
  await expect
    .poll(() =>
      textarea.evaluate((element) =>
        element instanceof HTMLTextAreaElement ? element.selectionStart : -1,
      ),
    )
    .toBe("Hello {{trigger.text}}".length);

  await textarea.fill("Keep {{auth");
  await expect(listbox).toBeVisible();
  await textarea.press("Escape");
  await expect(listbox).toBeHidden();
  await expect(textarea).toBeFocused();
  await expect(textarea).toHaveValue("Keep {{auth");

  await textarea.fill("By {{auth");
  await textarea.press("Tab");
  await expect(textarea).toHaveValue("By {{trigger.author}}");
  await expect(textarea).toBeFocused();
});

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
  await openTriggerInspector(dialog);
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
  await openTriggerInspector(dialog);
  await expect(
    dialog.getByRole("radio", { name: "Custom cron" }),
  ).toBeChecked();
  await expect(
    dialog.getByRole("textbox", { name: "Weekday", exact: true }),
  ).toHaveValue("2-4");

  await addMessageStep(page, dialog);
  await createEnabled(page, dialog);
  const reopened = await reopenWorkflow(page, name);
  await openTriggerInspector(reopened);
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

  await dialog
    .getByRole("group", { name: "Match" })
    .getByRole("button", {
      name: "ends with",
    })
    .click();
  await dialog.getByLabel("Message text").fill(text);
  await dialog.getByRole("tab", { name: "YAML" }).click();
  const yamlEditor = dialog.getByRole("textbox", { name: "Workflow YAML" });
  const definition = parseYaml(await yamlEditor.inputValue());
  expect(definition.trigger.filter).toBe(expression);

  await dialog.getByRole("tab", { name: "Form" }).click();
  await openTriggerInspector(dialog);
  const matchControls = dialog.getByRole("group", { name: "Match" });
  const operatorButtons = matchControls.getByRole("button");
  const firstOperatorBox = await operatorButtons.nth(0).boundingBox();
  const secondOperatorBox = await operatorButtons.nth(1).boundingBox();
  const thirdOperatorBox = await operatorButtons.nth(2).boundingBox();
  expect(firstOperatorBox).not.toBeNull();
  expect(secondOperatorBox).not.toBeNull();
  expect(thirdOperatorBox).not.toBeNull();
  expect(secondOperatorBox?.x).toBeGreaterThan(firstOperatorBox?.x ?? 0);
  expect(
    Math.abs((secondOperatorBox?.y ?? 0) - (firstOperatorBox?.y ?? 0)),
  ).toBeLessThan(1);
  expect(thirdOperatorBox?.y).toBeGreaterThan(firstOperatorBox?.y ?? 0);
  await waitForAnimations(page);
  await matchControls.screenshot({
    path: "test-results/workflow-message-condition-operators.png",
  });
  await expect(
    matchControls.getByRole("button", { name: "ends with" }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(dialog.getByLabel("Message text")).toHaveValue(text);
  await dialog.getByRole("tab", { name: "Advanced" }).click();
  await expect(dialog.getByLabel("Advanced expression")).toHaveValue(
    expression,
  );
  await dialog.getByRole("tab", { name: "Basic" }).click();

  await addMessageStep(page, dialog);
  await createEnabled(page, dialog);
  const reopened = await reopenWorkflow(page, name);
  await openTriggerInspector(reopened);
  await expect(
    reopened
      .getByRole("group", { name: "Match" })
      .getByRole("button", { name: "ends with" }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(reopened.getByLabel("Message text")).toHaveValue(text);
});

test("renders deterministic trigger, step, and workflow-card summaries", async ({
  page,
}) => {
  const name = `semantic_summaries_${Date.now()}`;
  await page.addInitScript((workflowName) => {
    const positions: number[] = [];
    Object.defineProperty(window, "__WORKFLOW_METADATA_POSITIONS__", {
      configurable: true,
      value: positions,
    });

    const findTarget = () => {
      const target = Array.from(
        document.querySelectorAll<HTMLElement>(
          '[data-testid="workflow-card-name"]',
        ),
      ).find((element) => element.textContent === workflowName);
      if (!target) {
        requestAnimationFrame(findTarget);
        return;
      }

      let remainingFrames = 12;
      const sample = () => {
        positions.push(target.getBoundingClientRect().y);
        remainingFrames -= 1;
        if (remainingFrames > 0) requestAnimationFrame(sample);
      };
      sample();
    };
    requestAnimationFrame(findTarget);
  }, name);
  const dialog = await openCreateWorkflow(page, name);

  await dialog.getByLabel("Message text").fill("deploy");
  const triggerNode = dialog.getByRole("button", {
    name: "Trigger: Message contains “deploy”",
  });
  await expect(triggerNode).toContainText("Trigger");
  await expect(triggerNode).toContainText("Message contains “deploy”");

  await addMessageStep(page, dialog);
  await dialog.getByRole("tab", { name: "YAML" }).click();
  const yamlEditor = dialog.getByRole("textbox", { name: "Workflow YAML" });
  const definition = parseYaml(await yamlEditor.inputValue());
  definition.steps[0].channel = "94a444a4-c0a3-5966-ab05-530c6ddc2301";
  await yamlEditor.fill(stringifyYaml(definition));
  await dialog.getByRole("tab", { name: "Form" }).click();
  const stepNode = dialog.getByRole("button", {
    name: "Step 1: “Workflow notification” in #agents",
  });
  await expect(stepNode).toContainText("Send Message");
  await expect(stepNode).toContainText("“Workflow notification” in #agents");
  await createEnabled(page, dialog);

  const card = page
    .locator('[data-testid^="workflow-card-"]')
    .filter({ hasText: name })
    .first();
  const semanticLabel = card.getByTestId("workflow-card-semantic-label");
  const workflowName = card.getByTestId("workflow-card-name");
  const channelName = card.getByTestId("workflow-card-channel");
  await expect(card.getByTestId("workflow-card-trigger-summary")).toHaveCount(
    0,
  );
  await expect(semanticLabel).toHaveText(
    "When a message contains “deploy”, send “Workflow notification”",
  );
  await expect(workflowName).toHaveText(name);
  await expect(channelName).toHaveText("#agents");
  const semanticBox = await semanticLabel.boundingBox();
  const nameBox = await workflowName.boundingBox();
  const channelBox = await channelName.boundingBox();
  expect(semanticBox).not.toBeNull();
  expect(nameBox).not.toBeNull();
  expect(channelBox).not.toBeNull();
  expect(semanticBox?.y).toBeLessThan(nameBox?.y ?? 0);
  expect(channelBox?.y).toBeLessThan(nameBox?.y ?? 0);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as typeof window & {
              __WORKFLOW_METADATA_POSITIONS__?: number[];
            }
          ).__WORKFLOW_METADATA_POSITIONS__?.length ?? 0,
      ),
    )
    .toBe(12);
  const metadataPositions = await page.evaluate(
    () =>
      (
        window as typeof window & {
          __WORKFLOW_METADATA_POSITIONS__?: number[];
        }
      ).__WORKFLOW_METADATA_POSITIONS__ ?? [],
  );
  expect(
    Math.max(...metadataPositions) - Math.min(...metadataPositions),
  ).toBeLessThan(0.5);
});

test("Escape closes a filter picker, restores disclosure focus, then closes its inspector", async ({
  page,
}) => {
  for (const width of [760, 1280]) {
    await page.setViewportSize({ width, height: 820 });
    await page.goto(
      "/#/workflows?view=create&channel=94a444a4-c0a3-5966-ab05-530c6ddc2301&pane=trigger",
    );

    const dialog = page.getByRole("dialog", { name: "Create workflow" });
    const inspector = dialog.getByTestId("workflow-node-inspector");
    await selectTrigger(page, dialog, "Reaction Added");
    for (const field of [
      {
        label: "Author",
        picker: "workflow-author-picker",
        search: "Search authors or paste a public key",
      },
      {
        label: "Message",
        picker: "workflow-message-picker",
        search: "Search messages or paste a message ID",
      },
    ]) {
      const disclosure = dialog
        .getByText(field.label, { exact: true })
        .locator("..");
      await disclosure.click();
      const picker = dialog.getByTestId(field.picker);
      const search = dialog.getByRole("combobox", { name: field.search });
      await expect(picker).toBeVisible();

      await search.press("Escape");
      await expect(picker).toBeHidden();
      await expect(disclosure).toBeFocused();
      await expect(inspector).toBeVisible();
    }

    await page.keyboard.press("Escape");
    await expect(inspector).toBeHidden();
    await expect(dialog).toBeVisible();
  }
});

test("workflow grid batches card author and message presentation reads", async ({
  page,
}) => {
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__TAURI_INTERNALS__?.invoke === "function",
  );
  await page.evaluate(async () => {
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    if (!invoke) throw new Error("mock invoke bridge unavailable");
    for (let index = 0; index < 40; index += 1) {
      const messageId = (index + 1).toString(16).padStart(64, "0");
      const author = (index + 101).toString(16).padStart(64, "0");
      await invoke("create_workflow", {
        channelId: "94a444a4-c0a3-5966-ab05-530c6ddc2301",
        yamlDefinition: JSON.stringify({
          name: `batched_card_${index}`,
          trigger: {
            on: "reaction_added",
            filter: `trigger_author == "${author}" && trigger_message_id == "${messageId}"`,
          },
          steps: [],
        }),
      });
    }

    const calls = { eventBatches: [], userBatches: [] };
    window.__BUZZ_WORKFLOW_BATCH_CALLS__ = calls;
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "get_events") {
        calls.eventBatches.push(
          (args?.eventIds as unknown[] | undefined)?.length ?? 0,
        );
      }
      if (command === "get_users_batch") {
        calls.userBatches.push(
          (args?.pubkeys as unknown[] | undefined)?.length ?? 0,
        );
      }
      return invoke(command, args);
    };
  });

  await page.getByTestId("open-workflows-view").click();
  await expect(
    page.locator('[data-testid^="workflow-card-mock-wf-"]'),
  ).toHaveCount(40);
  await expect
    .poll(() =>
      page.evaluate(() => {
        const calls = window.__BUZZ_WORKFLOW_BATCH_CALLS__;
        return {
          eventBatchCount: calls?.eventBatches.filter((size) => size === 40)
            .length,
          userBatchCount: calls?.userBatches.filter((size) => size === 40)
            .length,
        };
      }),
    )
    .toEqual({ eventBatchCount: 1, userBatchCount: 1 });
});

test("does not select stale picker results when Enter outruns deferred filtering", async ({
  page,
}) => {
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );
  await page.evaluate(() => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "agents",
      content: "Deferred message candidate",
      id: "d".repeat(64),
    });
  });
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
  await selectTrigger(page, dialog, "Reaction Added");

  await dialog.getByText("Author", { exact: true }).locator("..").click();
  const authorSearch = dialog.getByRole("combobox", {
    name: "Search authors or paste a public key",
  });
  await expect(dialog.getByRole("option").first()).toBeVisible();
  await authorSearch.evaluate((input: HTMLInputElement) => {
    input.value = "no-such-author";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(
      new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }),
    );
  });

  await dialog.getByText("Message", { exact: true }).locator("..").click();
  const messageSearch = dialog.getByRole("combobox", {
    name: "Search messages or paste a message ID",
  });
  await expect(
    dialog.getByRole("option", { name: /Deferred message/ }),
  ).toBeVisible();
  await messageSearch.evaluate((input: HTMLInputElement) => {
    input.value = "no-such-message";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(
      new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }),
    );
  });

  await dialog.getByRole("tab", { name: "YAML" }).click();
  const definition = parseYaml(
    await dialog.getByRole("textbox", { name: "Workflow YAML" }).inputValue(),
  );
  expect(definition.trigger.filter).toBeUndefined();
});

test("round-trips manual author and reaction message IDs through save and reopen", async ({
  page,
}) => {
  const name = `manual_trigger_ids_${Date.now()}`;
  const author = "a".repeat(64);
  const messageId = "b".repeat(64);
  const dialog = await openCreateWorkflow(page, name);
  await selectTrigger(page, dialog, "Reaction Added");
  await addMessageStep(page, dialog);
  await dialog.getByRole("button", { name: /^Trigger:/ }).click();
  await openTriggerInspector(dialog);

  await dialog.getByText("Author", { exact: true }).locator("..").click();
  const authorResults = dialog.getByTestId("workflow-author-picker-results");
  await expect
    .poll(() =>
      authorResults.evaluate((results) => {
        const template = results.querySelector<HTMLElement>("[role=option]");
        if (!template) return false;
        for (let index = 0; index < 20; index += 1) {
          const clone = template.cloneNode(true) as HTMLElement;
          clone.dataset.scrollFixture = "true";
          clone.removeAttribute("id");
          results.append(clone);
        }
        return results.scrollHeight > results.clientHeight;
      }),
    )
    .toBe(true);
  await authorResults.hover();
  await page.mouse.wheel(0, 240);
  await expect
    .poll(() => authorResults.evaluate((node) => node.scrollTop))
    .toBeGreaterThan(0);
  await authorResults.evaluate((results) => {
    for (const fixture of results.querySelectorAll("[data-scroll-fixture]")) {
      fixture.remove();
    }
    results.scrollTop = 0;
  });
  const authorSearch = dialog.getByRole("combobox", {
    name: "Search authors or paste a public key",
  });
  await authorSearch.fill(author);
  await expect(
    dialog.getByRole("option", { name: new RegExp(author.slice(0, 8)) }),
  ).toBeVisible();
  await authorSearch.press("Enter");
  await expect(
    dialog.getByRole("option", { name: new RegExp(author.slice(0, 8)) }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(dialog.getByRole("button", { name: "Create" })).toBeEnabled();
  await dialog.getByRole("tab", { name: "YAML" }).click();
  const correctionEditor = dialog.getByRole("textbox", {
    name: "Workflow YAML",
  });
  const correctedDefinition = parseYaml(await correctionEditor.inputValue());
  delete correctedDefinition.trigger.filter;
  await correctionEditor.fill(stringifyYaml(correctedDefinition));
  await expect(dialog.getByRole("button", { name: "Create" })).toBeEnabled();
  await dialog.getByRole("tab", { name: "Form" }).click();
  await openTriggerInspector(dialog);
  await dialog.getByText("Author", { exact: true }).locator("..").click();
  const correctedAuthorSearch = dialog.getByRole("combobox", {
    name: "Search authors or paste a public key",
  });
  await correctedAuthorSearch.fill(author);
  await expect(
    dialog.getByRole("option", { name: new RegExp(author.slice(0, 8)) }),
  ).toBeVisible();
  await correctedAuthorSearch.press("Enter");
  await dialog
    .getByRole("group", { name: "Match" })
    .getByRole("button", { name: "is not", exact: true })
    .click();

  await dialog.getByText("Message", { exact: true }).locator("..").click();
  const messageSearch = dialog.getByRole("combobox", {
    name: "Search messages or paste a message ID",
  });
  await messageSearch.fill(messageId);
  await messageSearch.press("Enter");
  await expect(
    dialog.getByRole("option", { name: new RegExp(messageId.slice(0, 12)) }),
  ).toHaveAttribute("aria-selected", "true");

  await dialog.getByRole("tab", { name: "YAML" }).click();
  const yamlEditor = dialog.getByRole("textbox", { name: "Workflow YAML" });
  let definition = parseYaml(await yamlEditor.inputValue());
  expect(definition.trigger).toEqual({
    on: "reaction_added",
    filter: `trigger_author != "${author}" && trigger_message_id == "${messageId}"`,
  });

  await dialog.getByRole("tab", { name: "Form" }).click();
  await openTriggerInspector(dialog);
  await createEnabled(page, dialog);

  const reopened = await reopenWorkflow(page, name);
  await openTriggerInspector(reopened);
  await reopened.getByText("Author", { exact: true }).locator("..").click();
  await expect(
    reopened.getByRole("option", { name: new RegExp(author.slice(0, 8)) }),
  ).toHaveAttribute("aria-selected", "true");
  await reopened.getByText("Message", { exact: true }).locator("..").click();
  await expect(
    reopened.getByRole("option", {
      name: new RegExp(messageId.slice(0, 12)),
    }),
  ).toHaveAttribute("aria-selected", "true");
  await reopened.getByRole("tab", { name: "YAML" }).click();
  definition = parseYaml(
    await reopened.getByRole("textbox", { name: "Workflow YAML" }).inputValue(),
  );
  expect(definition.trigger.filter).toContain(messageId);
});

test("toggles selected author and message filters while preserving sibling conditions", async ({
  page,
}) => {
  const author = "a".repeat(64);
  const replacementAuthor = "c".repeat(64);
  const messageId = "b".repeat(64);
  const dialog = await openCreateWorkflow(
    page,
    `toggle_trigger_ids_${Date.now()}`,
  );
  await selectTrigger(page, dialog, "Reaction Added");
  await dialog.getByRole("tab", { name: "YAML" }).click();
  const yamlEditor = dialog.getByRole("textbox", { name: "Workflow YAML" });
  const definition = parseYaml(await yamlEditor.inputValue());
  definition.trigger.filter =
    `trigger_emoji == "👍" && trigger_author == "${author.toUpperCase()}" && ` +
    `trigger_message_id == "${messageId.toUpperCase()}"`;
  await yamlEditor.fill(stringifyYaml(definition));
  await dialog.getByRole("tab", { name: "Form" }).click();
  await openTriggerInspector(dialog);
  const authorField = dialog
    .getByText("Author", { exact: true })
    .locator("..")
    .locator("..");
  await authorField.getByText("Author", { exact: true }).locator("..").click();
  const authorOption = dialog.getByRole("option", {
    name: new RegExp(author.slice(0, 8)),
  });
  await expect(authorOption).toHaveAttribute("aria-selected", "true");
  await expect(
    authorField.getByRole("button", { name: "Clear filter" }),
  ).toHaveCount(0);
  await authorOption.click();

  const authorSearch = dialog.getByRole("combobox", {
    name: "Search authors or paste a public key",
  });
  await authorSearch.fill(replacementAuthor);
  const replacementAuthorOption = dialog.getByRole("option", {
    name: new RegExp(replacementAuthor.slice(0, 8)),
  });
  await expect(replacementAuthorOption).toBeVisible();
  await authorSearch.press("Enter");
  await expect(replacementAuthorOption).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(authorSearch).toHaveValue(replacementAuthor);
  await authorSearch.press("Enter");
  await expect(authorSearch).toHaveValue(replacementAuthor);

  const messageField = dialog
    .getByText("Message", { exact: true })
    .locator("..")
    .locator("..");
  await messageField
    .getByText("Message", { exact: true })
    .locator("..")
    .click();
  const messageSearch = dialog.getByRole("combobox", {
    name: "Search messages or paste a message ID",
  });
  await expect(
    dialog.getByRole("option", { name: new RegExp(messageId.slice(0, 12)) }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(
    messageField.getByRole("button", { name: "Clear filter" }),
  ).toHaveCount(0);
  await messageSearch.fill(messageId);
  await messageSearch.press("Enter");
  await expect(messageSearch).toHaveValue(messageId);
  await messageSearch.press("Enter");
  const selectedMessage = dialog.getByRole("option", {
    name: new RegExp(messageId.slice(0, 12)),
  });
  await expect(selectedMessage).toHaveAttribute("aria-selected", "true");
  await selectedMessage.click();
  await expect(messageSearch).toHaveValue(messageId);

  await dialog.getByRole("button", { name: "Reaction emoji" }).click();
  await expect(
    dialog.getByRole("button", { name: "Clear filter" }),
  ).toBeVisible();

  await dialog.getByRole("tab", { name: "YAML" }).click();
  expect(parseYaml(await yamlEditor.inputValue()).trigger.filter).toBe(
    'trigger_emoji == "👍"',
  );
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

  await dialog.getByRole("button", { name: /^Trigger:/ }).click();
  await selectTrigger(page, dialog, "Schedule");
  await dialog.getByRole("button", { name: /^Step 1:/ }).click();
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
  await openTriggerInspector(dialog);
  await expect(dialog.getByRole("tab", { name: "Advanced" })).toHaveAttribute(
    "data-state",
    "active",
  );
  await expect(dialog.getByLabel("Advanced expression")).toHaveValue(advanced);
  await dialog.getByRole("tab", { name: "Basic" }).click();
  await expect(
    dialog.getByText(/advanced expression is active/i),
  ).toBeVisible();

  await expect(
    dialog.getByRole("switch", { name: "Enable workflow" }),
  ).toHaveCount(0);
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
