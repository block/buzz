import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

/**
 * Navigate to the Agents screen the way a user does.
 *
 * The app is not hash-routed at load time — going straight to `/#/agents`
 * races the bridge and the community gate, so the app renders its
 * "Community connection failed" screen instead. Wait for the mocked invoke
 * bridge, then click through, matching `agents.spec.ts`.
 */
async function gotoAgents(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(() => {
    const w = window as Window & {
      __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
      __TAURI_INTERNALS__?: { invoke?: unknown };
    };
    return (
      typeof w.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function" ||
      typeof w.__TAURI_INTERNALS__?.invoke === "function"
    );
  });
  await page.getByTestId("open-agents-view").click();
  await expect(
    page.locator("section", { hasText: "Server agents" }),
  ).toBeVisible({ timeout: 15_000 });
}

// A spawner pubkey the user has already pointed this device at. Seeded into
// localStorage so the section renders its configured state rather than the
// setup card.
const SPAWNER = "5c".repeat(32);

const SCREENSHOT_DIR = "test-results/server-agents";

test.describe("server agents section", () => {
  test("explains that a spawner need not be the relay machine", async ({
    page,
  }) => {
    await installMockBridge(page);
    await gotoAgents(page);

    const section = page.locator("section", { hasText: "Server agents" });
    await expect(section).toBeVisible();
    // The empty state is where the feature is discovered, so it has to explain
    // itself rather than just showing an input.
    await expect(section).toContainText("keep working when Buzz is closed");
    // The location-independence point users ask about first.
    await expect(section).toContainText(
      "does not have to be the relay machine",
    );
    await expect(
      section.getByPlaceholder("Spawner public key (64 hex characters)"),
    ).toBeVisible();

    await waitForAnimations(page);
    await section.screenshot({ path: `${SCREENSHOT_DIR}/01-setup-card.png` });
  });

  test("rejects a malformed spawner public key", async ({ page }) => {
    await installMockBridge(page);
    await gotoAgents(page);

    const section = page.locator("section", { hasText: "Server agents" });
    await section
      .getByPlaceholder("Spawner public key (64 hex characters)")
      .fill("not-a-pubkey");
    await section.getByRole("button", { name: "Connect" }).click();

    // Fails closed: the section must stay on the setup card rather than storing
    // a value the relay could never route to.
    await expect(
      section.getByPlaceholder("Spawner public key (64 hex characters)"),
    ).toBeVisible();
  });

  test("renders the configured state with a deploy menu", async ({ page }) => {
    // localStorage must be seeded BEFORE the bridge installs — React reads the
    // store on mount and the bridge is what triggers mount.
    await page.addInitScript((spawner) => {
      window.localStorage.setItem(
        "buzz:spawner-pubkeys",
        JSON.stringify([spawner]),
      );
    }, SPAWNER);
    await installMockBridge(page);
    await gotoAgents(page);

    const section = page.locator("section", { hasText: "Server agents" });
    await expect(section).toBeVisible();
    await expect(section).toContainText("Disconnect");
    await expect(section).toContainText("No agents here yet");

    const deploy = section.getByRole("button", { name: "Deploy agent" });
    await expect(deploy).toBeEnabled();

    await waitForAnimations(page);
    await section.screenshot({
      path: `${SCREENSHOT_DIR}/02-configured.png`,
    });

    // The menu offers the built-in personas, which is how an agent gets onto
    // the server at all.
    await deploy.click();
    const menu = page.getByRole("menu");
    await expect(menu).toBeVisible();
    await expect(menu).toContainText("Fizz");
    await waitForAnimations(page);
    await page.screenshot({
      path: `${SCREENSHOT_DIR}/03-deploy-menu.png`,
      clip: { x: 300, y: 1200, width: 980, height: 620 },
    });
  });

  test("lets the user disconnect a spawner", async ({ page }) => {
    await page.addInitScript((spawner) => {
      window.localStorage.setItem(
        "buzz:spawner-pubkeys",
        JSON.stringify([spawner]),
      );
    }, SPAWNER);
    await installMockBridge(page);
    await gotoAgents(page);

    const section = page.locator("section", { hasText: "Server agents" });
    await section.getByRole("button", { name: "Disconnect" }).click();

    // Disconnecting is local only: the connect field stays, and the deploy
    // action disappears because there is nowhere to deploy to.
    await expect(
      section.getByPlaceholder("Spawner public key (64 hex characters)"),
    ).toBeVisible();
    await expect(section).not.toContainText("Disconnect");
  });
});
