import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const SHOTS = "test-results/agent-sandbox-settings";

async function openSandboxCard(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "agents");
  const card = page.getByTestId("settings-agent-sandbox");
  await expect(card).toBeVisible({ timeout: 10_000 });
  return card;
}

function sandboxCommands(page: import("@playwright/test").Page) {
  return page.evaluate(() =>
    (window.__BUZZ_E2E_COMMANDS__ ?? []).filter((command) =>
      command.includes("agent_sandbox"),
    ),
  );
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("shows the sandbox on with the editable profile path", async ({
  page,
}) => {
  const card = await openSandboxCard(page);

  await expect(page.getByTestId("agent-sandbox-toggle")).toBeChecked();
  await expect(page.getByTestId("agent-sandbox-profile-path")).toHaveText(
    "/Users/mock/.config/nono/profiles/buzz-agent-local.json",
  );
  await expect(page.getByTestId("agent-sandbox-problem")).toHaveCount(0);
  await waitForAnimations(page);
  await card.screenshot({ path: `${SHOTS}/01-sandbox-on.png` });
});

test("turning the sandbox off asks first and can be cancelled", async ({
  page,
}) => {
  await openSandboxCard(page);
  const toggle = page.getByTestId("agent-sandbox-toggle");

  await toggle.click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText("Run agents without a sandbox?");
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/02-confirm-off.png` });

  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(toggle).toBeChecked();
  expect(await sandboxCommands(page)).not.toContain(
    "set_agent_sandbox_enabled",
  );

  await toggle.click();
  await page.getByTestId("agent-sandbox-confirm").click();
  await expect(toggle).not.toBeChecked();
  expect(await sandboxCommands(page)).toContain("set_agent_sandbox_enabled");

  // Turning it back on needs no confirmation.
  await toggle.click();
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  await expect(toggle).toBeChecked();
});

test("toggle is reachable from the keyboard through its label", async ({
  page,
}) => {
  await openSandboxCard(page);
  const toggle = page.getByRole("switch", { name: "Run agents in a sandbox" });

  await toggle.focus();
  await page.keyboard.press("Space");
  await expect(page.getByRole("alertdialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(toggle).toBeChecked();
});

test("reset asks first, then reports how to apply it", async ({ page }) => {
  await openSandboxCard(page);

  await page.getByTestId("agent-sandbox-reset-profile").click();
  await expect(page.getByRole("alertdialog")).toContainText(
    "Reset the sandbox profile?",
  );
  await page.getByTestId("agent-sandbox-confirm").click();

  await expect(page.getByTestId("agent-sandbox-notice")).toHaveText(
    "Sandbox profile reset. Restart agents to apply it.",
  );
  expect(await sandboxCommands(page)).toContain("reset_agent_sandbox_profile");
});
