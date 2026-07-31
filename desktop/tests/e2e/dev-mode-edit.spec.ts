import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

// `e` on a keyboard-selected prompt card loads your own message into the
// composer for editing (a kind:40003 edit event — the relay supports edits,
// so no delete-and-recompose). Enter saves; Escape cancels and lands back on
// the card. Someone else's prompt is not editable — `e` is a silent no-op.

const ORIGINAL_TEXT = "ship the release notes tonight";
const UPDATED_TEXT = "ship the release notes tomorrow morning";

async function openDevModeGeneral(page: import("@playwright/test").Page) {
  await installMockBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();
  await page
    .getByTestId("dev-mode-channel-navigator")
    .getByText("# general", { exact: true })
    .click();
  await page.getByTestId("dev-mode-transcript").waitFor();
  return composer;
}

/** Seed a root prompt into #general; no pubkey → authored by the current user. */
async function seedRoot(
  page: import("@playwright/test").Page,
  content: string,
  pubkey?: string,
) {
  await page.evaluate(
    ({ content, pubkey }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) throw new Error("mock bridge missing");
      emit({ channelName: "general", content, pubkey });
    },
    { content, pubkey },
  );
}

test("`e` loads my selected prompt into the composer; Enter saves the edit", async ({
  page,
}) => {
  const composer = await openDevModeGeneral(page);
  await seedRoot(page, ORIGINAL_TEXT);

  const newestCard = page.getByTestId("dev-mode-prompt-card").last();
  await expect(newestCard).toContainText(ORIGINAL_TEXT);

  // ArrowUp selects the newest card (mine); `e` starts editing it.
  await composer.focus();
  await page.keyboard.press("ArrowUp");
  await expect(newestCard).toContainText("⏎ side chat");
  await page.keyboard.press("e");

  await expect(composer).toHaveValue(ORIGINAL_TEXT);
  await expect(page.getByTestId("dev-mode-draft-banner")).toContainText(
    "editing message",
  );

  await composer.fill(UPDATED_TEXT);
  await page.keyboard.press("Enter");

  // The card re-renders with the edited content and an "(edited)" marker;
  // the composer returns to plain sending.
  await expect(newestCard).toContainText(UPDATED_TEXT);
  await expect(newestCard).toContainText("(edited)");
  await expect(newestCard).not.toContainText(ORIGINAL_TEXT);
  await expect(page.getByTestId("dev-mode-draft-banner")).toHaveCount(0);
  await expect(composer).toHaveValue("");
});

test("Escape cancels the edit and lands back on the card, unchanged", async ({
  page,
}) => {
  const composer = await openDevModeGeneral(page);
  await seedRoot(page, ORIGINAL_TEXT);

  const newestCard = page.getByTestId("dev-mode-prompt-card").last();
  await expect(newestCard).toContainText(ORIGINAL_TEXT);

  await composer.focus();
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("e");
  await expect(composer).toHaveValue(ORIGINAL_TEXT);

  await page.keyboard.press("Escape");

  await expect(page.getByTestId("dev-mode-draft-banner")).toHaveCount(0);
  await expect(composer).toHaveValue("");
  // Back on the card it came from, content untouched.
  await expect(newestCard).toContainText("⏎ side chat");
  await expect(newestCard).toContainText(ORIGINAL_TEXT);
  await expect(newestCard).not.toContainText("(edited)");
});

test("`e` on someone else's prompt is a silent no-op", async ({ page }) => {
  const composer = await openDevModeGeneral(page);
  await seedRoot(
    page,
    "bob's prompt: rotate the API keys",
    TEST_IDENTITIES.bob.pubkey,
  );

  const newestCard = page.getByTestId("dev-mode-prompt-card").last();
  await expect(newestCard).toContainText("rotate the API keys");

  await composer.focus();
  await page.keyboard.press("ArrowUp");
  await expect(newestCard).toContainText("⏎ side chat");
  await page.keyboard.press("e");

  // No edit mode: no banner, empty composer, card still selected.
  await expect(page.getByTestId("dev-mode-draft-banner")).toHaveCount(0);
  await expect(composer).toHaveValue("");
  await expect(newestCard).toContainText("⏎ side chat");
});
