import { expect, test } from "@playwright/test";

import { installMockBridge, type MockBridgeOptions } from "../helpers/bridge";

// Command-palette ranking and post-archive navigation: typing an action
// verb like "archive" ranks that action above channels whose names merely
// contain it, and archiving/leaving a chat lands on the most recently
// active non-pinned chat instead of the fresh composer.

async function openDevModeChannel(
  page: import("@playwright/test").Page,
  channelName: string,
  mock?: MockBridgeOptions,
) {
  await installMockBridge(page, mock);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();
  await composer.focus();

  const topBar = page.getByTestId("dev-mode-topbar-channel");
  for (let step = 0; step < 20; step += 1) {
    await page.keyboard.press("ArrowUp");
    const previewed = (await topBar.innerText()).replace(/^#\s*/, "").trim();
    if (previewed === channelName) break;
  }
  await expect(topBar).toContainText(channelName);
  await page.keyboard.press("Enter");
  await page.getByTestId("dev-mode-transcript").waitFor();
}

test("typed action verb outranks channel-name matches in the palette", async ({
  page,
}) => {
  await openDevModeChannel(page, "general");

  // A channel whose name contains the verb would otherwise outrank the
  // action (channels normally rank first while typing).
  await page.evaluate(async () => {
    const w = window as Window & {
      __TAURI_INTERNALS__?: {
        invoke: (command: string, payload: unknown) => Promise<unknown>;
      };
      __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
    };
    await w.__TAURI_INTERNALS__?.invoke("create_channel", {
      name: "team-archives",
      channelType: "stream",
      visibility: "open",
    });
    await w.__BUZZ_E2E_INVALIDATE_CHANNELS__?.();
  });

  await page.keyboard.press("Meta+k");
  const palette = page.getByTestId("dev-mode-palette");
  await expect(palette).toBeVisible();
  await page.getByTestId("dev-mode-palette-input").pressSequentially("archiv");

  const entries = page.getByTestId("dev-mode-palette-entry");
  await expect(entries.first()).toContainText("archive # general");
  await expect(
    entries.filter({ hasText: "# team-archives" }).first(),
  ).toBeVisible();
});

test("palette searches open channels the user hasn't joined and joins on enter", async ({
  page,
}) => {
  await openDevModeChannel(page, "general");

  // Seed a non-member open channel: create one, then leave it (the mock
  // bridge drops membership without archiving).
  await page.evaluate(async () => {
    const w = window as Window & {
      __TAURI_INTERNALS__?: {
        invoke: (command: string, payload: unknown) => Promise<unknown>;
      };
      __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
    };
    const created = (await w.__TAURI_INTERNALS__?.invoke("create_channel", {
      name: "growth-experiments",
      channelType: "stream",
      visibility: "open",
    })) as { id: string };
    await w.__TAURI_INTERNALS__?.invoke("leave_channel", {
      channelId: created.id,
    });
    await w.__BUZZ_E2E_INVALIDATE_CHANNELS__?.();
  });

  await page.keyboard.press("Meta+k");
  const palette = page.getByTestId("dev-mode-palette");
  await expect(palette).toBeVisible();
  await page.getByTestId("dev-mode-palette-input").pressSequentially("growth");

  const entry = page
    .getByTestId("dev-mode-palette-entry")
    .filter({ hasText: "# growth-experiments" })
    .first();
  await expect(entry).toContainText("not joined");
  await page.keyboard.press("Enter");

  // Joined and opened: the palette closes and the channel is now current.
  await expect(palette).not.toBeVisible();
  await expect(page.getByTestId("dev-mode-topbar-channel")).toContainText(
    "growth-experiments",
  );
});

test("create channel action makes a named channel and opens it", async ({
  page,
}) => {
  await openDevModeChannel(page, "general");

  await page.keyboard.press("Meta+k");
  const palette = page.getByTestId("dev-mode-palette");
  await expect(palette).toBeVisible();
  await page
    .getByTestId("dev-mode-palette-input")
    .pressSequentially("create channel");
  const entries = page.getByTestId("dev-mode-palette-entry");
  await expect(entries.first()).toContainText("create channel");
  await page.keyboard.press("Enter");

  // Now in create mode: the typed text becomes the channel name.
  await page
    .getByTestId("dev-mode-palette-input")
    .pressSequentially("Launch Plans!");
  await expect(entries.first()).toContainText("create # launch-plans");
  await page.keyboard.press("Enter");

  await expect(palette).not.toBeVisible();
  await expect(page.getByTestId("dev-mode-topbar-channel")).toContainText(
    "launch-plans",
  );
});

test("copy link to channel puts a buzz:// message link on the clipboard", async ({
  page,
}) => {
  await openDevModeChannel(page, "general");

  await page.keyboard.press("Meta+k");
  const palette = page.getByTestId("dev-mode-palette");
  await expect(palette).toBeVisible();
  await page
    .getByTestId("dev-mode-palette-input")
    .pressSequentially("copy link");
  const entries = page.getByTestId("dev-mode-palette-entry");
  await expect(entries.first()).toContainText("copy link to # general");
  await page.keyboard.press("Enter");
  await expect(palette).not.toBeVisible();

  const payload = await page.evaluate(() => {
    const log = (
      window as Window & {
        __BUZZ_E2E_COMMAND_LOG__?: Array<{
          command: string;
          payload: Record<string, unknown> | null;
        }>;
      }
    ).__BUZZ_E2E_COMMAND_LOG__;
    return log?.findLast(({ command }) => command === "copy_text_to_clipboard")
      ?.payload;
  });

  // The deep-link protocol has no channel-only form, so the channel link
  // targets its newest root message.
  expect(payload?.text).toMatch(
    /^buzz:\/\/message\?channel=9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50&id=.+/,
  );
});

test("archiving a chat lands on the most recent non-pinned chat", async ({
  page,
}) => {
  await openDevModeChannel(page, "general");

  await page.keyboard.press("Meta+k");
  await page.getByTestId("dev-mode-palette-input").pressSequentially("archive");
  const entries = page.getByTestId("dev-mode-palette-entry");
  await expect(entries.first()).toContainText("archive # general");
  await page.keyboard.press("Enter");

  // Landed in another chat, not the fresh composer and not the archived one.
  await expect(page.getByTestId("dev-mode-palette")).not.toBeVisible();
  await page.getByTestId("dev-mode-transcript").waitFor();
  const topBar = page.getByTestId("dev-mode-topbar-channel");
  await expect(topBar).not.toContainText("general");
  await expect(topBar).toContainText("#");
});

test("archive updates the UI optimistically, before the relay resolves", async ({
  page,
}) => {
  // The mock relay sleeps 5s before acknowledging the archive; the palette
  // must close and navigation must land elsewhere well before that.
  await openDevModeChannel(page, "general", { archiveChannelDelayMs: 5_000 });

  await page.keyboard.press("Meta+k");
  await page.getByTestId("dev-mode-palette-input").pressSequentially("archive");
  const entries = page.getByTestId("dev-mode-palette-entry");
  await expect(entries.first()).toContainText("archive # general");

  const start = Date.now();
  await page.keyboard.press("Enter");

  await expect(page.getByTestId("dev-mode-palette")).not.toBeVisible();
  const topBar = page.getByTestId("dev-mode-topbar-channel");
  await expect(topBar).not.toContainText("general");
  await expect(topBar).toContainText("#");
  expect(Date.now() - start).toBeLessThan(4_000);
});
