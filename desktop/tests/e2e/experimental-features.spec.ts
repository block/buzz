import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { FEATURE_OVERRIDES_STORAGE_KEY } from "../helpers/features";
import { openSettings } from "../helpers/settings";

const DESCRIPTION =
  "Give each channel thread isolated agent context. Applies when managed agents next start; DMs stay conversation-scoped.";

test("thread-scoped ACP sessions is default-off, persists, and applies on reload", async ({
  page,
}) => {
  await installMockBridge(page, undefined, { seedPreviewFeatures: false });
  await page.goto("/");

  await expect
    .poll(() =>
      page.evaluate(() => {
        const call = window.__BUZZ_E2E_COMMAND_LOG__?.find(
          ({ command }) => command === "apply_workspace",
        );
        return (call?.payload as { threadScopedAcpSessions?: boolean } | null)
          ?.threadScopedAcpSessions;
      }),
    )
    .toBe(false);

  await openSettings(page, "experimental");
  const toggle = page.getByTestId("feature-toggle-threadScopedAcpSessions");

  await expect(
    page.getByText("Thread Scoped ACP Sessions", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText(DESCRIPTION, { exact: true })).toBeVisible();
  await expect(toggle).not.toBeChecked();
  await expect(page.getByTestId("feature-toggle-projects")).not.toBeChecked();
  await expect(page.getByTestId("feature-toggle-workflows")).not.toBeChecked();

  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const calls = window.__BUZZ_E2E_COMMAND_LOG__ ?? [];
        const call = calls.findLast(
          ({ command }) => command === "set_thread_scoped_acp_sessions",
        );
        return (call?.payload as { enabled?: boolean } | null)?.enabled;
      }),
    )
    .toBe(true);
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        FEATURE_OVERRIDES_STORAGE_KEY,
      ),
    )
    .toContain('"threadScopedAcpSessions":true');

  await page.reload();
  // Settings route/section state survives reload, so wait for that restored
  // view rather than trying to reopen settings from the app chrome.
  await expect(page.getByTestId("settings-view")).toBeVisible();
  await expect(
    page.getByTestId("feature-toggle-threadScopedAcpSessions"),
  ).toBeChecked();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const call = window.__BUZZ_E2E_COMMAND_LOG__?.find(
          ({ command }) => command === "apply_workspace",
        );
        return (call?.payload as { threadScopedAcpSessions?: boolean } | null)
          ?.threadScopedAcpSessions;
      }),
    )
    .toBe(true);
});
