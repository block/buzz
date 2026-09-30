/**
 * E2E spec for the inline "Add custom harness…" entry in the agent dialogs.
 *
 * Registering a custom harness used to be reachable only from Settings →
 * Agents, so anyone whose first touchpoint was "New agent" never learned the
 * path existed. The harness dropdowns now carry the entry directly.
 *
 * Covers, on all three surfaces (create, edit definition, edit instance):
 *  - choosing the entry opens the registration form
 *  - saving registers the harness and selects it in the dropdown
 *
 * Create and instance edit additionally assert the sentinel never becomes the
 * selection; create alone covers dismissing the form leaving the previous
 * selection untouched. The three surfaces share the same routing, so those
 * checks are not repeated on every one.
 */
import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const ADD_ENTRY = "Add custom harness…";
const HARNESS_LABEL = "My Weird Agent";
const HARNESS_COMMAND = "my-weird-acp";

type Page = import("@playwright/test").Page;
type Locator = import("@playwright/test").Locator;

/** Open a PersonaDropdownField (button trigger + menuitemradio options). */
async function openDropdown(trigger: Locator) {
  await expect(trigger).toBeVisible({ timeout: 10_000 });
  await trigger.click();
}

/** Register a harness through the inline form and wait for it to close. */
async function registerHarness(page: Page) {
  const form = page.getByTestId("custom-harness-form");
  await expect(form).toBeVisible({ timeout: 8_000 });
  await page.fill("#ch-label", HARNESS_LABEL);
  await page.fill("#ch-command", HARNESS_COMMAND);
  // The id auto-derives from the label; it is what the dropdown selects on.
  await expect(page.locator("#ch-id")).toHaveValue("my-weird-agent");
  await form.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByTestId("add-custom-harness-dialog")).not.toBeVisible({
    timeout: 8_000,
  });
}

/** Open the create-agent dialog (AgentDefinitionDialog, create mode). */
async function openCreateDialog(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  const dialog = page.getByTestId("persona-dialog");
  await expect(dialog).toBeVisible({ timeout: 10_000 });
  await dialog.getByRole("tab", { name: "Customize for this agent" }).click();
  return dialog;
}

/** Open the edit dialog for a saved definition (same dialog, edit mode). */
async function openDefinitionEditDialog(page: Page, name: string) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-library-personas")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByRole("button", { name: `Open actions for ${name}` }).click();
  await page.getByRole("menuitem", { name: "Edit" }).click();
  const dialog = page.getByTestId("persona-dialog");
  await expect(dialog).toBeVisible({ timeout: 10_000 });
  await dialog.getByRole("tab", { name: "Customize for this agent" }).click();
  return dialog;
}

test.describe("inline add custom harness", () => {
  test("create dialog registers a harness inline and selects it", async ({
    page,
  }) => {
    await installMockBridge(page);
    const dialog = await openCreateDialog(page);

    const harness = dialog.locator("#persona-runtime");
    await openDropdown(harness);
    await page.getByRole("menuitemradio", { name: ADD_ENTRY }).click();

    // The sentinel opens the form; it must never become the selection.
    await expect(page.getByTestId("add-custom-harness-dialog")).toBeVisible({
      timeout: 8_000,
    });
    await expect(harness).not.toContainText(ADD_ENTRY);

    await registerHarness(page);

    // The saved harness is now the selected harness.
    await expect(harness).toContainText(HARNESS_LABEL, { timeout: 8_000 });
  });

  test("dismissing the form leaves the create dialog's harness unchanged", async ({
    page,
  }) => {
    await installMockBridge(page);
    const dialog = await openCreateDialog(page);

    const harness = dialog.locator("#persona-runtime");
    await expect(harness).toBeVisible({ timeout: 10_000 });
    const before = await harness.textContent();

    await openDropdown(harness);
    await page.getByRole("menuitemradio", { name: ADD_ENTRY }).click();
    await expect(page.getByTestId("add-custom-harness-dialog")).toBeVisible({
      timeout: 8_000,
    });

    await page.keyboard.press("Escape");
    await expect(
      page.getByTestId("add-custom-harness-dialog"),
    ).not.toBeVisible();

    // No harness was registered, so the prior selection must survive.
    await expect(harness).toHaveText(before ?? "");
  });

  test("definition edit dialog registers a harness inline and selects it", async ({
    page,
  }) => {
    await installMockBridge(page, {
      personas: [
        {
          displayName: "Editable Agent",
          systemPrompt: "An agent whose harness gets replaced.",
        },
      ],
    });
    const dialog = await openDefinitionEditDialog(page, "Editable Agent");

    const harness = dialog.locator("#persona-runtime");
    await openDropdown(harness);
    await page.getByRole("menuitemradio", { name: ADD_ENTRY }).click();
    await expect(page.getByTestId("add-custom-harness-dialog")).toBeVisible({
      timeout: 8_000,
    });

    await registerHarness(page);

    await expect(harness).toContainText(HARNESS_LABEL, { timeout: 8_000 });
  });

  test("instance edit dialog registers a harness inline and keeps Custom command", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey:
            "npub1e2e00000000000000000000000000000000000000000000000000000000",
          name: "Instance Agent",
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await page.getByTestId("open-agents-view").click();
    await page
      .getByRole("button", { name: "Instance Agent agent profile" })
      .click();
    await expect(page.getByTestId("user-profile-panel")).toBeVisible({
      timeout: 10_000,
    });
    await page.getByTestId("user-profile-edit-agent").click();
    await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
      timeout: 10_000,
    });

    const provider = page.locator("#edit-agent-runtime");
    await openDropdown(provider);

    // "Custom command" is a different feature (ad-hoc command override) and
    // must survive alongside the new entry.
    await expect(
      page.getByRole("menuitemradio", { name: "Custom command" }),
    ).toBeVisible();
    await page.getByRole("menuitemradio", { name: ADD_ENTRY }).click();
    await expect(page.getByTestId("add-custom-harness-dialog")).toBeVisible({
      timeout: 8_000,
    });
    await expect(provider).not.toContainText(ADD_ENTRY);

    await registerHarness(page);

    await expect(provider).toContainText(HARNESS_LABEL, { timeout: 8_000 });
  });
});
