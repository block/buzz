import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/persona-model-combobox";

async function waitForInvokeBridge(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () => {
      const w = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        __TAURI_INTERNALS__?: { invoke?: unknown };
      };
      return (
        typeof w.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function" ||
        typeof w.__TAURI_INTERNALS__?.invoke === "function"
      );
    },
    null,
    { timeout: 8_000 },
  );
}

/**
 * Open the Agents view, click "New agent", and open the persona create dialog.
 * Returns the dialog locator.
 */
async function openNewPersonaDialog(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForInvokeBridge(page);

  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-library-personas")).toBeVisible({
    timeout: 8_000,
  });

  await page.getByTestId("new-agent-card").click();
  await page.getByRole("menuitem", { name: "New agent" }).click();

  const dialog = page.getByTestId("persona-dialog");
  await expect(dialog).toBeVisible({ timeout: 8_000 });
  await dialog.getByRole("tab", { name: "Customize for this agent" }).click();
  return dialog;
}

/**
 * The mock bridge selects Goose as the default runtime (available + preferred).
 * Wait for model discovery to populate the combobox options so the list is
 * non-trivial before opening the popover.
 */
async function waitForModelCombobox(
  dialog: import("@playwright/test").Locator,
) {
  // The model field only appears after a runtime is selected. Goose is the
  // mock default; the field should appear without manual interaction.
  const trigger = dialog.getByRole("combobox", { name: /model/i });
  await expect(trigger).toBeVisible({ timeout: 8_000 });
  return trigger;
}

test.describe("persona model combobox screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (err) => {
      console.error(
        "PAGE ERROR:",
        err.message,
        err.stack?.split("\n").slice(0, 5).join("\n"),
      );
    });
    page.on("console", (msg) => {
      if (msg.type() === "error") {
        console.error("CONSOLE ERROR:", msg.text().slice(0, 500));
      }
    });
    await installMockBridge(page);
  });

  test("01 — closed trigger (model not yet selected)", async ({ page }) => {
    const dialog = await openNewPersonaDialog(page);
    await waitForModelCombobox(dialog);
    await waitForAnimations(page);

    await dialog.screenshot({ path: `${SHOTS}/01-closed-trigger.png` });
  });

  test("02 — open popover with full model list", async ({ page }) => {
    const dialog = await openNewPersonaDialog(page);
    const trigger = await waitForModelCombobox(dialog);

    await trigger.click();

    // Wait for the search input to appear (popover is open).
    await expect(page.getByPlaceholder("Search models…")).toBeVisible({
      timeout: 5_000,
    });

    // Wait for model discovery to populate at least one non-loading row.
    await expect(
      page.getByRole("button", { name: /claude|gpt|default/i }).first(),
    ).toBeVisible({ timeout: 8_000 });

    await waitForAnimations(page);
    await dialog.screenshot({ path: `${SHOTS}/02-open-full-list.png` });
  });

  test("03 — filtered results (query: gpt)", async ({ page }) => {
    const dialog = await openNewPersonaDialog(page);
    const trigger = await waitForModelCombobox(dialog);

    await trigger.click();

    const searchInput = page.getByPlaceholder("Search models…");
    await expect(searchInput).toBeVisible({ timeout: 5_000 });
    await expect(
      page.getByRole("button", { name: /claude|gpt|default/i }).first(),
    ).toBeVisible({ timeout: 8_000 });

    await searchInput.fill("gpt");

    // At least one GPT option should be visible; Claude options gone.
    await expect(
      page.getByRole("button", { name: /gpt/i }).first(),
    ).toBeVisible({ timeout: 3_000 });

    await waitForAnimations(page);
    await dialog.screenshot({ path: `${SHOTS}/03-filtered-gpt.png` });
  });

  test("04 — empty state (no models match)", async ({ page }) => {
    const dialog = await openNewPersonaDialog(page);
    const trigger = await waitForModelCombobox(dialog);

    await trigger.click();

    const searchInput = page.getByPlaceholder("Search models…");
    await expect(searchInput).toBeVisible({ timeout: 5_000 });
    await expect(
      page.getByRole("button", { name: /claude|gpt|default/i }).first(),
    ).toBeVisible({ timeout: 8_000 });

    await searchInput.fill("zzznomatch");

    await expect(page.getByText("No models match")).toBeVisible({
      timeout: 3_000,
    });
    await waitForAnimations(page);
    await dialog.screenshot({ path: `${SHOTS}/04-empty-state.png` });
  });
});
