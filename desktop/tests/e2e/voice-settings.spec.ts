import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

test("selects the streaming Siri backend and an installed voice", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-voice").click();

  await expect(page.getByTestId("settings-nav-voice")).toBeVisible();
  await expect(page.getByTestId("settings-voice")).toBeVisible();

  await page.getByRole("button", { name: "Pocket TTS" }).click();
  await page
    .getByRole("menuitemradio", { name: "Siri TTS (Experimental)" })
    .click();

  await expect(page.getByText("Aaron", { exact: true })).toBeVisible();
  await expect(page.getByText("Nora", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: /Aaron/ })).toBeEnabled();

  const log = await page.evaluate(() => window.__BUZZ_E2E_COMMAND_LOG__ ?? []);
  expect(log).toContainEqual({
    command: "set_tts_settings",
    payload: {
      settings: {
        backend: "siri",
        siri_voice: "Aaron",
        siri_language: "en-US",
        siri_rate: 1,
      },
    },
  });
});
