import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const HUDDLE_CHANNEL_ID = "11111111-1111-4111-8111-111111111111";
const HUDDLE_PARENT_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

test("renders a compact controller for the active huddle", async ({ page }) => {
  await page.setViewportSize({ width: 376, height: 148 });
  await installMockBridge(page, {
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [
        { pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" },
        { pubkey: TEST_IDENTITIES.alice.pubkey, role: "bot" },
      ],
      transcriptionEnabled: true,
    },
  });

  await page.goto("/#/voice-overlay");

  const overlay = page.getByTestId("voice-overlay");
  await expect(overlay).toBeVisible();
  await expect(overlay.getByText("Buzz Voice")).toBeVisible();
  await expect(overlay.getByText("2 participants · 1 agent")).toBeVisible();
  await expect(
    overlay.getByRole("button", { name: "Stop transcript" }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    overlay.getByRole("button", { name: "Hear agent voice" }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    overlay.getByRole("button", { name: "Leave huddle" }),
  ).toBeVisible();
});

test("clears the floating controller when the huddle becomes idle", async ({
  page,
}) => {
  await installMockBridge(page, {
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
    },
  });

  await page.goto("/#/voice-overlay");
  const overlay = page.getByTestId("voice-overlay");
  await expect(overlay.getByText("1 participant · 0 agents")).toBeVisible();

  await page.evaluate(async () => {
    await window.__BUZZ_E2E_SET_MOCK_HUDDLE_SNAPSHOT__?.({
      members: [{ pubkey: "test-participant", role: "member" }],
      transcriptionEnabled: false,
      phase: "idle",
    });
  });

  await expect(overlay.getByText("No active huddle")).toBeVisible();
  await expect(
    overlay.getByRole("button", { name: "Leave huddle" }),
  ).toBeDisabled();
});

test("shows a matching action failure and ignores an unrelated result", async ({
  page,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(window.crypto, "randomUUID", {
      configurable: true,
      value: () => "00000000-0000-4000-8000-000000000001",
    });
  });
  await installMockBridge(page, {
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
    },
  });

  await page.goto("/#/voice-overlay");
  const overlay = page.getByTestId("voice-overlay");
  await overlay.getByRole("button", { name: "Start transcript" }).click();

  await page.evaluate(async () => {
    await window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.(
      "buzz://voice-overlay/action-result",
      {
        version: 1,
        requestId: "unrelated-request",
        ok: false,
        error: "Wrong request",
      },
    );
  });
  await expect(overlay.getByRole("alert")).toHaveCount(0);

  await page.evaluate(async () => {
    await window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.(
      "buzz://voice-overlay/action-result",
      {
        version: 1,
        requestId: "00000000-0000-4000-8000-000000000001",
        ok: false,
        error: "Transcript failed",
      },
    );
  });
  await expect(overlay.getByRole("alert")).toHaveText(
    "Voice action failed: Transcript failed",
  );
});

test("routes typed overlay actions through the main huddle owner", async ({
  page,
}) => {
  await installMockBridge(page, {
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
      transcriptionEnabled: true,
    },
  });

  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Open floating voice controls" }),
  ).toBeVisible();
  await page.evaluate(async () => {
    await window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.(
      "buzz://voice-overlay/action",
      {
        version: 1,
        requestId: "e2e-toggle-transcription",
        type: "toggle_transcription",
      },
    );
  });
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
          (entry) => entry.command === "set_huddle_transcription_enabled",
        ),
      ),
    )
    .toEqual([
      {
        command: "set_huddle_transcription_enabled",
        payload: { enabled: false },
      },
    ]);
});

test("opens the floating controller from the active huddle bar", async ({
  page,
}) => {
  await installMockBridge(page, {
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
    },
  });

  await page.goto("/");
  await page
    .getByRole("button", { name: "Open floating voice controls" })
    .click();

  await expect
    .poll(() =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
          (entry) => entry.command === "voice_overlay_window",
        ),
      ),
    )
    .toEqual([
      { command: "voice_overlay_window", payload: { action: "open" } },
    ]);
});
