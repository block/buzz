import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

test("Agents keeps bootstrap error visible and reconnect reruns workspace initialization", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await expect(page.getByTestId("open-agents-view")).toBeVisible();
  await page.evaluate(() => {
    const original = window.__TAURI_INTERNALS__.invoke;
    let incomplete = true;
    window.__TAURI_INTERNALS__.invoke = async (command, args, options) => {
      if (command === "get_managed_agent_sync_error") {
        return incomplete
          ? "managed-agent history exceeds bootstrap page limit"
          : null;
      }
      if (command === "apply_workspace") incomplete = false;
      return original(command, args, options);
    };
  });
  await page.getByTestId("open-agents-view").click();
  const warning = page
    .getByRole("alert")
    .filter({ hasText: "Agent sync is incomplete" });
  await expect(warning).toContainText("page limit");
  await expect(warning).toContainText("community operator");
  await waitForAnimations(page);
  await page.screenshot({
    path:
      process.env.BUZZ_SYNC_WARNING_SCREENSHOT ??
      "test-results/agent-sync-warning.png",
  });
  await warning.getByRole("button", { name: "Reconnect community" }).click();
  await expect(page.getByTestId("open-agents-view")).toBeVisible();
  // Community remount may retain or reset the current route.
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-page-content")).toBeVisible();
  await expect(warning).toHaveCount(0);
});
