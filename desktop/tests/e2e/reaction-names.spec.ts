import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const REACTION_TARGET_CONTENT = "React to me with a custom emoji";
const REACTION_TARGET_EVENT_ID = "d".repeat(64);
const BOB_PUBKEY =
  "bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260";

function reactionTargetRow(page: import("@playwright/test").Page) {
  return page
    .getByTestId("message-row")
    .filter({ hasText: REACTION_TARGET_CONTENT })
    .last();
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("reaction popover resolves a reactor with no authored message in the window", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
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

  const row = reactionTargetRow(page);
  const pill = row.getByRole("button", { name: "Toggle 🎉 reaction" });
  await expect(pill).toBeVisible();
  await pill.hover();
  await expect(page.getByText("bob reacted with")).toBeVisible();
});
