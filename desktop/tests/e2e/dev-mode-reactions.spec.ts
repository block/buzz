import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Dev-mode reaction chips sit on the message header line next to the
// timestamp. Community (custom) emoji render as images through the loopback
// media proxy — resolved from the reaction's NIP-30 tag or, when a reaction
// arrives without one (e.g. sent via the CLI), from the community palette.
// Clicking a chip toggles the member's own reaction; the hover "+" opens the
// shared emoji picker.

const REACTION_TARGET_CONTENT = "React to me with a custom emoji";
const REACTION_TARGET_EVENT_ID = "d".repeat(64);
const REACTION_SHORTCODE = "react";
const MOCK_MEDIA_PROXY_PORT = 54321;
const BOB_PUBKEY =
  "bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260";

const PROXIED_EMOJI_SRC = new RegExp(
  `^http://127\\.0\\.0\\.1:${MOCK_MEDIA_PROXY_PORT}/media/[\\da-f]{64}\\.png$`,
);

async function openDevModeGeneral(page: import("@playwright/test").Page) {
  await installMockBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("dev-mode-composer").waitFor();
  await page
    .getByTestId("dev-mode-channel-navigator")
    .getByText("# general", { exact: true })
    .click();
  await page.getByTestId("dev-mode-transcript").waitFor();
}

function reactionTargetCard(page: import("@playwright/test").Page) {
  return page
    .getByTestId("dev-mode-prompt-card")
    .filter({ hasText: REACTION_TARGET_CONTENT })
    .last();
}

test("a custom-emoji reaction without a NIP-30 tag renders via the community palette", async ({
  page,
}) => {
  await openDevModeGeneral(page);
  await page.waitForFunction(
    () =>
      window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
        channelName: "general",
        kind: 7,
      }) === true,
  );

  // A bare `:react:` reaction — no `["emoji", shortcode, url]` tag — must
  // still resolve its image through the community emoji palette.
  await page.evaluate(
    ({ pubkey, targetId, shortcode }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `:${shortcode}:`,
        extraTags: [["e", targetId]],
        kind: 7,
        pubkey,
      });
    },
    {
      pubkey: BOB_PUBKEY,
      shortcode: REACTION_SHORTCODE,
      targetId: REACTION_TARGET_EVENT_ID,
    },
  );

  const card = reactionTargetCard(page);
  const chip = card.getByRole("button", {
    name: `Toggle :${REACTION_SHORTCODE}: reaction`,
  });
  await expect(chip).toBeVisible();

  // Rendered as an image, not shortcode text, and through the media proxy.
  const img = chip.locator(`img[alt=':${REACTION_SHORTCODE}:']`);
  await expect(img).toBeVisible();
  await expect(img).toHaveAttribute("src", PROXIED_EMOJI_SRC);
});

test("clicking a chip toggles my own reaction on and off", async ({ page }) => {
  await openDevModeGeneral(page);
  await page.waitForFunction(
    () =>
      window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
        channelName: "general",
        kind: 7,
      }) === true,
  );

  await page.evaluate(
    ({ pubkey, targetId }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: "🎉",
        extraTags: [["e", targetId]],
        kind: 7,
        pubkey,
      });
    },
    { pubkey: BOB_PUBKEY, targetId: REACTION_TARGET_EVENT_ID },
  );

  const card = reactionTargetCard(page);
  const chip = card.getByRole("button", { name: "Toggle 🎉 reaction" });
  await expect(chip).toBeVisible();
  await expect(chip).toHaveAttribute("aria-pressed", "false");

  // Join bob's reaction: the chip becomes mine and counts both of us.
  await chip.click();
  await expect(chip).toHaveAttribute("aria-pressed", "true");
  await expect(chip).toContainText("2");

  // Toggle back off: bob's reaction remains, mine is withdrawn.
  await chip.click();
  await expect(chip).toHaveAttribute("aria-pressed", "false");
  await expect(chip).not.toContainText("2");
});

test("the hover + affordance adds a reaction through the emoji picker", async ({
  page,
}) => {
  await openDevModeGeneral(page);

  const card = reactionTargetCard(page);
  await card.hover();
  await card
    .getByTestId(`dev-mode-add-reaction-${REACTION_TARGET_EVENT_ID}`)
    .click();

  // emoji-mart renders inside a Shadow DOM web component. Search by shortcode
  // to surface the community emoji, then click it.
  const picker = page.locator("em-emoji-picker");
  await picker.locator("input[type='search']").fill(REACTION_SHORTCODE);
  await picker
    .getByRole("button", { name: `:${REACTION_SHORTCODE}:` })
    .first()
    .click();

  const chip = card.getByRole("button", {
    name: `Toggle :${REACTION_SHORTCODE}: reaction`,
  });
  await expect(chip).toBeVisible();
  await expect(chip).toHaveAttribute("aria-pressed", "true");
  await expect(chip.locator("img")).toHaveAttribute("src", PROXIED_EMOJI_SRC);
});
