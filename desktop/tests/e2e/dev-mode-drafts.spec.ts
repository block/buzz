import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Composer text is a per-channel draft: switching channels leaves the text
// behind and restores it on return instead of carrying it along. The fresh
// (new-session) composer keeps its own slot.

async function openDevMode(page: import("@playwright/test").Page) {
  await installMockBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();
  return composer;
}

async function openChannel(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await page
    .getByTestId("dev-mode-channel-navigator")
    .getByText(`# ${channelName}`, { exact: true })
    .click();
  await expect(page.getByTestId("dev-mode-topbar-channel")).toContainText(
    channelName,
  );
  await page.getByTestId("dev-mode-transcript").waitFor();
}

async function createChannel(
  page: import("@playwright/test").Page,
  name: string,
) {
  await page.evaluate(async (channelName) => {
    const w = window as Window & {
      __TAURI_INTERNALS__?: {
        invoke: (command: string, payload: unknown) => Promise<unknown>;
      };
      __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
    };
    await w.__TAURI_INTERNALS__?.invoke("create_channel", {
      name: channelName,
      channelType: "stream",
      visibility: "open",
    });
    await w.__BUZZ_E2E_INVALIDATE_CHANNELS__?.();
  }, name);
}

test("drafts stay with their channel across switches", async ({ page }) => {
  const composer = await openDevMode(page);
  await createChannel(page, "drafts-two");

  await openChannel(page, "general");
  await composer.fill("draft for general");

  await openChannel(page, "drafts-two");
  await expect(composer).toHaveValue("");
  await composer.fill("draft for two");

  await openChannel(page, "general");
  await expect(composer).toHaveValue("draft for general");

  await openChannel(page, "drafts-two");
  await expect(composer).toHaveValue("draft for two");
});

test("the fresh composer keeps its own draft", async ({ page }) => {
  const composer = await openDevMode(page);

  // Typed in the fresh (new-session) composer, then a channel is opened.
  await composer.fill("spawn a new channel later");
  await openChannel(page, "general");
  await expect(composer).toHaveValue("");

  // Escape backs out of the channel — the fresh draft comes back.
  await composer.focus();
  await page.keyboard.press("Escape");
  await expect(composer).toHaveValue("spawn a new channel later");
});

test("switching channels mid-edit abandons the edit buffer", async ({
  page,
}) => {
  const composer = await openDevMode(page);
  await createChannel(page, "drafts-two");
  await openChannel(page, "general");

  await page.evaluate(() => {
    const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    if (!emit) throw new Error("mock bridge missing");
    emit({ channelName: "general", content: "original prompt text" });
  });
  const newestCard = page.getByTestId("dev-mode-prompt-card").last();
  await expect(newestCard).toContainText("original prompt text");

  // ArrowUp selects my prompt; `e` loads it into the composer for editing.
  await composer.focus();
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("e");
  await expect(composer).toHaveValue("original prompt text");

  // Leaving the channel cancels the edit; the message body must not be
  // stashed as #general's draft.
  await openChannel(page, "drafts-two");
  await expect(composer).toHaveValue("");
  await expect(page.getByTestId("dev-mode-draft-banner")).toHaveCount(0);

  await openChannel(page, "general");
  await expect(composer).toHaveValue("");
  await expect(page.getByTestId("dev-mode-draft-banner")).toHaveCount(0);
});

test("sending clears the channel draft", async ({ page }) => {
  const composer = await openDevMode(page);
  await createChannel(page, "drafts-two");
  await openChannel(page, "general");

  await composer.fill("shipped message");
  await page.keyboard.press("Enter");
  await expect(composer).toHaveValue("");

  await openChannel(page, "drafts-two");
  await openChannel(page, "general");
  await expect(composer).toHaveValue("");
});
