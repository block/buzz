import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// While an agent has an active turn in a channel, dev mode renders that
// channel's name as per-character wavy text — in the tab strip and in the
// channel navigator — and reverts to plain text when the turn ends.

// alice — agent member of #general in the mock bridge.
const ALICE_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";
const CHANNEL_GENERAL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

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

function seedTurnEvent(
  page: import("@playwright/test").Page,
  kind: "turn_started" | "turn_completed",
) {
  return page.evaluate(
    ({ pubkey, channelId, eventKind }) => {
      window.__BUZZ_E2E_SEED_ACTIVE_TURNS__?.({
        agentPubkey: pubkey,
        channelId,
        turnId: "turn-wavy-1",
        kind: eventKind,
      });
    },
    { pubkey: ALICE_PUBKEY, channelId: CHANNEL_GENERAL, eventKind: kind },
  );
}

test("channel names wave while an agent turn is active", async ({ page }) => {
  await openDevModeGeneral(page);

  // Idle — plain labels everywhere.
  await expect(page.getByTestId("dev-mode-wavy-text")).toHaveCount(0);

  await seedTurnEvent(page, "turn_started");

  const tab = page.getByTestId("dev-mode-channel-tab").first();
  const wavyTab = tab.getByTestId("dev-mode-wavy-text");
  await expect(wavyTab).toBeVisible();
  await expect(wavyTab).toHaveText("main");
  // Per-character spans carry the animation; the label text is unchanged.
  await expect(wavyTab.locator(".dev-wavy-char")).toHaveCount(4);

  const navigatorWavy = page
    .getByTestId("dev-mode-channel-navigator")
    .getByTestId("dev-mode-wavy-text");
  await expect(navigatorWavy).toHaveText("general");

  await seedTurnEvent(page, "turn_completed");
  await expect(page.getByTestId("dev-mode-wavy-text")).toHaveCount(0);
  await expect(tab).toHaveText("main");
});
