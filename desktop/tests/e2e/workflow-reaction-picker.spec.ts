import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

// The Reaction Added trigger stores the reaction event's content verbatim — a
// native glyph or a custom `:shortcode:`. These cover the picker interaction and
// that the chosen value survives into canonical YAML and back out of a reload,
// which a free-text field silently got wrong (a typed `thumbsup` never matches).

const EMOJI = "🚀";
const EMOJI_SHORTCODE = ":rocket:";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

async function openReactionTrigger(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-workflows-view").click();
  await expect(page.getByTestId("workflows-view")).toBeVisible();

  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  await expect(dialog).toBeVisible();

  const channelList = page.getByTestId("channel-combobox-list");
  if (!(await channelList.isVisible())) {
    await dialog.getByRole("combobox", { name: "Channel" }).click();
  }
  await expect(channelList).toBeVisible();
  await waitForAnimations(page);
  await channelList
    .getByRole("option", { name: "agents", exact: true })
    .click();

  await dialog.getByRole("button", { name: "Trigger: Message Posted" }).click();
  await dialog.getByRole("button", { name: "Trigger event" }).click();
  await page.getByRole("menuitem", { name: "Reaction Added" }).click();

  return dialog;
}

async function pickEmoji(
  page: import("@playwright/test").Page,
  dialog: import("@playwright/test").Locator,
  query: string,
  glyph: string,
) {
  await dialog.getByRole("button", { name: "Choose trigger emoji" }).click();
  const picker = page.locator("em-emoji-picker");
  await expect(picker).toBeVisible();
  await picker.locator("input[type='search']").fill(query);
  await picker.locator(`button[aria-label='${glyph}']`).first().click();
}

test("reaction trigger uses the emoji picker instead of a free-text field", async ({
  page,
}) => {
  const dialog = await openReactionTrigger(page);

  // The old plain text input is gone; the picker trigger stands in its place.
  const pickerButton = dialog.getByRole("button", {
    name: "Choose trigger emoji",
  });
  await expect(pickerButton).toBeVisible();
  await expect(pickerButton).toHaveText(/Choose a reaction/);
  await expect(
    dialog.getByRole("textbox", { name: "Emoji filter (optional)" }),
  ).toHaveCount(0);

  // Nothing to clear while the optional filter is empty.
  await expect(
    dialog.getByRole("button", { name: "Clear trigger emoji" }),
  ).toHaveCount(0);
  await expect(
    dialog
      .getByRole("button", { name: "Trigger: Reaction Added" })
      .getByTestId("workflow-node-icon")
      .locator("svg"),
  ).toHaveCount(1);

  await pickEmoji(page, dialog, "rocket", EMOJI);

  const triggerNode = dialog.getByRole("button", {
    name: "Trigger: 🚀 reaction added",
  });
  await expect(triggerNode.getByTestId("workflow-node-icon")).toHaveText(EMOJI);
  await expect(triggerNode).toContainText("Reaction added");
  await expect(triggerNode).not.toContainText(`${EMOJI} reaction added`);
  await expect(
    triggerNode.getByTestId("workflow-node-icon").locator("svg"),
  ).toHaveCount(0);
  await waitForAnimations(page);
  await triggerNode.screenshot({
    path: "test-results/workflow-reaction-trigger-node.png",
  });

  // Selecting closes the popover and shows the chosen reaction by name.
  await expect(page.locator("em-emoji-picker")).toHaveCount(0);
  await expect(pickerButton).toContainText(EMOJI_SHORTCODE);
  await expect(pickerButton).not.toContainText("Choose a reaction");
});

test("add-reaction step replaces its sequence number with the configured emoji", async ({
  page,
}) => {
  const dialog = await openReactionTrigger(page);
  await dialog.getByRole("tab", { name: "YAML" }).click();
  const yamlEditor = dialog.getByRole("textbox", { name: "Workflow YAML" });
  await yamlEditor.fill(
    (await yamlEditor.inputValue()).replace(
      "steps: []",
      "steps:\n  - id: step_1\n    action: add_reaction\n    emoji: ✅",
    ),
  );

  await dialog.getByRole("tab", { name: "Form" }).click();
  const stepNode = dialog.getByRole("button", { name: "Step 1: ✅" });
  await expect(stepNode.getByTestId("workflow-node-icon")).toHaveText("✅");
  await expect(stepNode).toContainText("Add Reaction");
  await expect(stepNode).not.toContainText("✅✅");
  await expect(stepNode.getByTestId("workflow-node-icon")).not.toHaveText("1");
  await waitForAnimations(page);
  await stepNode.screenshot({
    path: "test-results/workflow-add-reaction-step-node.png",
  });
});

test("picked reaction persists into canonical YAML and clears back out", async ({
  page,
}) => {
  const dialog = await openReactionTrigger(page);
  await pickEmoji(page, dialog, "rocket", EMOJI);

  await dialog.getByRole("tab", { name: "YAML" }).click();
  const yamlEditor = dialog.getByRole("textbox", { name: "Workflow YAML" });
  const yaml = await yamlEditor.inputValue();
  expect(yaml).toContain("on: reaction_added");
  // Stored verbatim in the local filter expression the executor compares.
  expect(yaml).toContain(`trigger_emoji == "${EMOJI}"`);

  // Clearing the optional filter removes the key entirely rather than
  // writing an empty string, so the trigger matches any reaction again.
  await dialog.getByRole("tab", { name: "Form" }).click();
  // Switching editor modes drops the inspector selection, so re-open the
  // trigger node before reaching for its fields.
  await dialog
    .getByRole("button", { name: `Trigger: ${EMOJI} reaction added` })
    .click();
  await dialog.getByRole("button", { name: "Clear trigger emoji" }).click();
  await expect(
    dialog.getByRole("button", { name: "Choose trigger emoji" }),
  ).toHaveText(/Choose a reaction/);

  await dialog.getByRole("tab", { name: "YAML" }).click();
  const clearedYaml = await yamlEditor.inputValue();
  expect(clearedYaml).toContain("on: reaction_added");
  expect(clearedYaml).not.toContain("trigger_emoji");
});

test("legacy free-text emoji values survive a trip through the picker", async ({
  page,
}) => {
  // The field used to be free text, so existing definitions can hold a bare
  // shortcode. Swapping in a picker must not silently drop a value it cannot
  // render as a glyph — the user has to be able to see and re-pick it.
  const dialog = await openReactionTrigger(page);

  await dialog.getByRole("tab", { name: "YAML" }).click();
  const yamlEditor = dialog.getByRole("textbox", { name: "Workflow YAML" });
  const seeded = (await yamlEditor.inputValue()).replace(
    "trigger:\n  on: reaction_added",
    "trigger:\n  on: reaction_added\n  emoji: thumbsup",
  );
  await yamlEditor.fill(seeded);

  await dialog.getByRole("tab", { name: "Form" }).click();
  await dialog.getByRole("button", { name: "Trigger: Reaction Added" }).click();
  const pickerButton = dialog.getByRole("button", {
    name: "Choose trigger emoji",
  });
  await expect(pickerButton).toContainText("thumbsup");
  await expect(pickerButton).not.toContainText("Choose a reaction");

  await dialog.getByRole("tab", { name: "YAML" }).click();
  expect(await yamlEditor.inputValue()).toContain("emoji: thumbsup");
});

test("saved reaction filter round-trips back into the picker", async ({
  page,
}) => {
  const name = `reaction_picker_${Date.now()}`;
  const dialog = await openReactionTrigger(page);

  await dialog.getByRole("button", { name: "Edit workflow name" }).click();
  await dialog.getByRole("textbox", { name: "Workflow name" }).fill(name);
  await dialog.getByRole("button", { name: "Save workflow name" }).click();

  await pickEmoji(page, dialog, "rocket", EMOJI);

  await dialog.getByRole("button", { name: "Add step", exact: true }).click();
  await page.getByRole("menuitem", { name: "Send Message" }).click();
  await dialog.getByLabel("Message text").fill("Workflow notification");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("dialog")).not.toBeVisible();

  await page.getByRole("button", { name: "Workflow actions" }).first().click();
  await page.getByRole("menuitem", { name: "Edit" }).click();

  const editDialog = page.getByRole("dialog", { name: "Edit workflow" });
  await expect(editDialog).toBeVisible();
  await editDialog
    .getByRole("button", { name: `Trigger: ${EMOJI} reaction added` })
    .click();

  // Reopening rehydrates the stored glyph into the picker, not an empty field.
  await expect(
    editDialog.getByRole("button", { name: "Choose trigger emoji" }),
  ).toContainText(EMOJI_SHORTCODE);
});

test("emoji picker opens within the viewport at narrow width", async ({
  page,
}) => {
  // Reach the trigger inspector at desktop width — below 58rem the inspector
  // becomes an absolute overlay covering the canvas nodes — then narrow the
  // window with the inspector already open and drive the picker there.
  const dialog = await openReactionTrigger(page);
  await page.setViewportSize({ width: 800, height: 720 });
  await waitForAnimations(page);

  await dialog.getByRole("button", { name: "Choose trigger emoji" }).click();
  const picker = page.locator("em-emoji-picker");
  await expect(picker).toBeVisible();
  await waitForAnimations(page);

  // Radix collision handling must keep the popover on-screen, otherwise the
  // picker is unreachable at this width.
  const box = await picker.boundingBox();
  expect(box).not.toBeNull();
  if (box) {
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(800);
  }

  await picker.locator("input[type='search']").fill("rocket");
  await picker.locator(`button[aria-label='${EMOJI}']`).first().click();
  await expect(
    dialog.getByRole("button", { name: "Choose trigger emoji" }),
  ).toContainText(EMOJI_SHORTCODE);
});
