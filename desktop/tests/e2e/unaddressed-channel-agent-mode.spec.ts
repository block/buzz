import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

test.describe("unaddressed channel agent mode", () => {
  test.beforeEach(async ({ page }) => {
    await installMockBridge(page);
  });

  test("settings agents section exposes unaddressed mode radios", async ({
    page,
  }) => {
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "agents");

    const agents = page.getByTestId("settings-agents");
    await agents.scrollIntoViewIfNeeded();
    await expect(agents).toBeVisible();

    const group = page.getByTestId("unaddressed-channel-agent-mode");
    await expect(group).toBeVisible();

    const allAgents = page.getByTestId("unaddressed-mode-all-channel-agents");
    const mentionsOnly = page.getByTestId("unaddressed-mode-mentions-only");
    await expect(allAgents).toBeChecked();
    await expect(mentionsOnly).not.toBeChecked();

    await mentionsOnly.check();
    await expect(mentionsOnly).toBeChecked();
    await expect(allAgents).not.toBeChecked();

    // Persist across settings re-open (device-local).
    await page.keyboard.press("Escape");
    await openSettings(page, "agents");
    await expect(
      page.getByTestId("unaddressed-mode-mentions-only"),
    ).toBeChecked();

    await waitForAnimations(page);
  });
});
