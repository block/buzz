/**
 * Screenshot spec for the Maple (OpenSecret) provider entry in the persona
 * provider picker.
 *
 *  01 – Provider dropdown open, "Maple (encrypted)" listed between
 *       OpenRouter and Buzz shared compute.
 *  02 – Maple selected: the dialog shows the "Maple API Key" credential
 *       field and the model picker (models arrive through the
 *       provider-generic "buzz-agent models" discovery path).
 */
import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/screenshots-maple";

async function openPersonaEditDialog(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-library-personas")).toBeVisible({
    timeout: 10_000,
  });

  const actionsBtn = page.getByRole("button", {
    name: "Open actions for Encrypted Agent",
  });
  await expect(actionsBtn).toBeVisible({ timeout: 8_000 });
  await actionsBtn.click();
  await page.getByRole("menuitem", { name: "Edit" }).click();

  const dialog = page.getByTestId("persona-dialog");
  await expect(dialog).toBeVisible({ timeout: 10_000 });
  await dialog.getByRole("tab", { name: "Customize for this agent" }).click();
  return dialog;
}

test.describe("maple provider UI screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test("01-maple-provider-option", async ({ page }) => {
    await installMockBridge(page, {
      personas: [
        {
          displayName: "Encrypted Agent",
          systemPrompt: "An agent for capturing the provider picker.",
        },
      ],
    });
    const dialog = await openPersonaEditDialog(page);

    const providerSelect = dialog.locator("#persona-llm-provider");
    await expect(providerSelect).toBeVisible({ timeout: 8_000 });
    await providerSelect.click();

    const mapleOption = page.getByRole("menuitemradio", {
      name: "Maple (encrypted)",
    });
    await expect(mapleOption).toBeVisible({ timeout: 5_000 });
    await expect(mapleOption).toHaveText("Maple (encrypted)");

    await waitForAnimations(page);

    // The option popover is portaled outside the dialog, so the clip must
    // cover the union of the dialog and the popover rectangles.
    const dialogBox = await dialog.boundingBox();
    const optionBox = await mapleOption.boundingBox();
    const x = Math.min(dialogBox.x, optionBox.x) - 16;
    const y = Math.min(dialogBox.y, optionBox.y) - 16;
    await page.screenshot({
      path: `${SHOTS}/01-maple-provider-option.png`,
      clip: {
        x: Math.max(0, x),
        y: Math.max(0, y),
        width:
          Math.max(
            dialogBox.x + dialogBox.width,
            optionBox.x + optionBox.width,
          ) -
          Math.max(0, x) +
          16,
        height:
          Math.max(
            dialogBox.y + dialogBox.height,
            optionBox.y + optionBox.height,
          ) -
          Math.max(0, y) +
          16,
      },
    });
  });

  test("02-maple-api-key-field", async ({ page }) => {
    await installMockBridge(page, {
      personas: [
        {
          displayName: "Encrypted Agent",
          systemPrompt: "An agent for capturing the provider picker.",
        },
      ],
    });
    const dialog = await openPersonaEditDialog(page);

    const providerSelect = dialog.locator("#persona-llm-provider");
    await expect(providerSelect).toBeVisible({ timeout: 8_000 });
    await providerSelect.click();

    await page
      .getByRole("menuitemradio", { name: "Maple (encrypted)" })
      .click();
    await expect(providerSelect).toContainText("Maple (encrypted)", {
      timeout: 5_000,
    });

    // Selecting Maple swaps the credential field to the Maple API Key input.
    await expect(dialog.getByText("Maple API Key")).toBeVisible({
      timeout: 5_000,
    });

    // Close any popover so the shot captures the dialog's resting state.
    await page.keyboard.press("Escape");
    await waitForAnimations(page);

    await dialog.screenshot({ path: `${SHOTS}/02-maple-api-key-field.png` });
  });
});
