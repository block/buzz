import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { FEATURE_OVERRIDES_STORAGE_KEY } from "../helpers/features";

test("legacy thread-session experiment migrates definitions exactly once", async ({
  page,
}) => {
  await page.addInitScript(
    ({ key }) => {
      if (window.localStorage.getItem(key) === null) {
        window.localStorage.setItem(
          key,
          JSON.stringify({
            workflows: true,
            threadScopedAcpSessions: true,
          }),
        );
      }
    },
    { key: FEATURE_OVERRIDES_STORAGE_KEY },
  );
  await installMockBridge(page, undefined, { seedPreviewFeatures: false });
  await page.goto("/");

  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const call = window.__BUZZ_E2E_COMMAND_LOG__?.find(
          ({ command }) => command === "apply_workspace",
        );
        return (
          call?.payload as {
            migrateLegacyThreadScopedAcpSessions?: boolean;
          }
        )?.migrateLegacyThreadScopedAcpSessions;
      }),
    )
    .toBe(true);

  const migrated = await page.evaluate(async () => {
    const invoke = window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
    return (await invoke?.("list_personas", {})) as
      | Array<{ session_policy?: string }>
      | undefined;
  });
  expect(migrated).toBeDefined();
  expect(
    migrated?.every(({ session_policy }) => session_policy === "thread"),
  ).toBe(true);
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        FEATURE_OVERRIDES_STORAGE_KEY,
      ),
    )
    .toBe(JSON.stringify({ workflows: true }));

  await page.reload();
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const call = window.__BUZZ_E2E_COMMAND_LOG__?.find(
          ({ command }) => command === "apply_workspace",
        );
        return (
          call?.payload as {
            migrateLegacyThreadScopedAcpSessions?: boolean;
          }
        )?.migrateLegacyThreadScopedAcpSessions;
      }),
    )
    .toBe(false);
});
