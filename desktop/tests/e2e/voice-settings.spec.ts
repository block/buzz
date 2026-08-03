import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const SCREENSHOT_PATH = "test-results/voice-settings/pocket-voices.png";

test.describe("Pocket voice settings", () => {
  test.use({ viewport: { width: 1100, height: 760 } });

  test("selects and retains a bundled voice while text to speech is off", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "voice");

    const card = page.getByTestId("settings-voice");
    await expect(card).toBeVisible();
    await expect(
      page.getByText("Agent text to speech", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("Pocket TTS voice", { exact: true }),
    ).toBeVisible();
    await expect(card).not.toContainText("April INT8");

    await page.getByTestId("pocket-voice-selector").click();
    await expect(page.getByRole("menuitemradio")).toHaveCount(12);
    await page.getByRole("menuitemradio", { name: "Eve" }).click();
    await expect(page.getByTestId("pocket-voice-selector")).toContainText(
      "Eve",
    );
    await expect(
      page.getByRole("button", { name: "Pocket TTS voice: Eve" }),
    ).toBeVisible();

    await page.getByTestId("agent-text-to-speech-toggle").click();
    await expect(
      page.getByTestId("agent-text-to-speech-toggle"),
    ).toHaveAttribute("aria-checked", "false");
    await expect(page.getByTestId("pocket-voice-controls")).toHaveAttribute(
      "aria-disabled",
      "true",
    );
    await expect(page.getByTestId("pocket-voice-selector")).toContainText(
      "Eve",
    );

    const savedCommands = await page.evaluate(() =>
      (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
        .filter((entry) =>
          ["set_pocket_voice", "set_tts_enabled"].includes(entry.command),
        )
        .map((entry) => ({ command: entry.command, payload: entry.payload })),
    );
    expect(savedCommands).toEqual([
      {
        command: "set_pocket_voice",
        payload: { voiceKey: "pocket:eve" },
      },
      {
        command: "set_tts_enabled",
        payload: { enabled: false },
      },
    ]);
  });

  test("captures the complete VCTK preset settings surface", async ({
    page,
  }) => {
    await installMockBridge(page, {
      ttsSettings: {
        version: 1,
        agentTextToSpeech: true,
        voicePreferences: ["pocket:eve"],
      },
    });
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "voice");

    const card = page.getByTestId("settings-voice");
    await expect(card).toBeVisible();
    await expect(page.getByTestId("pocket-voice-selector")).toContainText(
      "Eve",
    );
    await page.getByTestId("pocket-voice-selector").click();
    await expect(page.getByRole("menuitemradio")).toHaveCount(12);
    const menu = page.getByRole("menu");
    await expect(menu).toBeVisible();
    await waitForAnimations(page);
    const cardBox = await card.boundingBox();
    const menuBox = await menu.boundingBox();
    const viewport = page.viewportSize();
    if (!cardBox || !menuBox || !viewport) {
      throw new Error("Voice settings screenshot bounds are unavailable");
    }
    const x = Math.max(0, Math.min(cardBox.x, menuBox.x) - 16);
    const y = Math.max(0, Math.min(cardBox.y, menuBox.y) - 16);
    const right = Math.min(
      viewport.width,
      Math.max(cardBox.x + cardBox.width, menuBox.x + menuBox.width) + 16,
    );
    const bottom = Math.min(
      viewport.height,
      Math.max(cardBox.y + cardBox.height, menuBox.y + menuBox.height) + 16,
    );
    await page.screenshot({
      path: SCREENSHOT_PATH,
      clip: {
        x: Math.floor(x),
        y: Math.floor(y),
        width: Math.ceil(right - x),
        height: Math.ceil(bottom - y),
      },
    });
  });

  test("imports, selects, and safely deletes a local voice", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "voice");

    await page.getByTestId("pocket-voice-import").click();
    await expect(page.getByTestId("pocket-voice-selector")).toContainText(
      "My voice",
    );
    await expect(page.getByTestId("pocket-voice-delete")).toBeVisible();
    await page.getByRole("button", { name: "Preview" }).click();

    await page.getByTestId("pocket-voice-delete").click();
    await expect(page.getByText("Delete imported voice?")).toBeVisible();
    await page.getByTestId("confirm-pocket-voice-delete").click();
    await expect(page.getByTestId("pocket-voice-selector")).toContainText(
      "Mary",
    );
    await expect(page.getByTestId("pocket-voice-delete")).toBeHidden();
    await page.getByRole("button", { name: "Preview" }).click();

    const mutations = await page.evaluate(() =>
      (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
        .filter((entry) =>
          [
            "import_pocket_voice",
            "preview_pocket_voice",
            "delete_pocket_voice",
          ].includes(entry.command),
        )
        .map((entry) => ({ command: entry.command, payload: entry.payload })),
    );
    expect(mutations).toEqual([
      { command: "import_pocket_voice", payload: {} },
      {
        command: "preview_pocket_voice",
        payload: { voiceKey: `pocket:imported:${"1".repeat(64)}` },
      },
      {
        command: "delete_pocket_voice",
        payload: { voiceKey: `pocket:imported:${"1".repeat(64)}` },
      },
      {
        command: "preview_pocket_voice",
        payload: { voiceKey: "pocket:mary" },
      },
    ]);
  });

  test("keeps the selected voice unchanged when the native picker is cancelled", async ({
    page,
  }) => {
    await installMockBridge(page, { pocketVoiceImportResult: "cancel" });
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "voice");

    await expect(page.getByTestId("pocket-voice-selector")).toContainText(
      "Mary",
    );
    await page.getByTestId("pocket-voice-import").click();
    await expect(page.getByTestId("pocket-voice-selector")).toContainText(
      "Mary",
    );
    await expect(page.getByTestId("pocket-voice-delete")).toBeHidden();
    await expect(page.getByTestId("voice-settings-error")).toBeHidden();

    const audioCommands = await page.evaluate(() =>
      (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter((entry) =>
        ["preview_pocket_voice", "delete_pocket_voice"].includes(entry.command),
      ),
    );
    expect(audioCommands).toEqual([]);
  });

  test("surfaces invalid or unsupported WAV errors without changing selection", async ({
    page,
  }) => {
    await installMockBridge(page, { pocketVoiceImportResult: "invalid" });
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "voice");

    await page.getByTestId("pocket-voice-import").click();
    await expect(page.getByTestId("voice-settings-error")).toContainText(
      "Voice WAV must contain PCM or 32-bit float audio",
    );
    await expect(page.getByTestId("pocket-voice-selector")).toContainText(
      "Mary",
    );
    await expect(page.getByTestId("pocket-voice-delete")).toBeHidden();
  });
});
