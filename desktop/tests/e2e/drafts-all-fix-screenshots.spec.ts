import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/drafts-all-fix";

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const MOCK_IDENTITY_PUBKEY = "deadbeef".repeat(8);

type MockFeedWindow = Window & {
  __BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?: (item: Record<string, unknown>) => void;
};

test.use({ viewport: { width: 1280, height: 720 } });

test("Inbox All hides drafts while the Drafts filter keeps them", async ({
  page,
}) => {
  const draftKey = `channel:${GENERAL_CHANNEL_ID}`;
  await page.addInitScript(
    ({ draftStoreKey, draftStorageKey, channelId }) => {
      const timestamp = new Date().toISOString();
      window.localStorage.setItem(
        draftStoreKey,
        JSON.stringify({
          [draftStorageKey]: {
            channelId,
            content: "Draft that must stay out of the All view",
            createdAt: timestamp,
            pendingImeta: [],
            selectionEnd: 41,
            selectionStart: 41,
            spoileredAttachmentUrls: [],
            status: "active",
            updatedAt: timestamp,
          },
        }),
      );
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      draftStorageKey: draftKey,
      draftStoreKey: `buzz-drafts.v2:ws://localhost:3000:${MOCK_IDENTITY_PUBKEY}`,
    },
  );
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(() => {
    const win = window as MockFeedWindow;
    return typeof win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ === "function";
  });

  const messageId = "drafts-all-fix-message";
  await page.evaluate(
    ({ channelId, currentPubkey, messageId: id, senderPubkey }) => {
      const pushFeedItem = (window as MockFeedWindow)
        .__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
      if (!pushFeedItem) throw new Error("Mock feed helper is not installed.");
      pushFeedItem({
        category: "mention",
        channel_id: channelId,
        channel_name: "general",
        channel_type: "stream",
        content: "Regular message that belongs in All",
        created_at: Math.floor(Date.now() / 1_000),
        id,
        kind: 9,
        pubkey: senderPubkey,
        tags: [
          ["h", channelId],
          ["p", currentPubkey],
        ],
      });
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      currentPubkey: MOCK_IDENTITY_PUBKEY,
      messageId,
      senderPubkey: TEST_IDENTITIES.alice.pubkey,
    },
  );

  // All view: the message is present, the draft row is not.
  await expect(page.getByTestId(`home-inbox-item-${messageId}`)).toBeVisible();
  await expect(page.getByTestId(`home-all-drafts-${draftKey}`)).toHaveCount(0);
  await waitForAnimations(page);
  await page
    .getByTestId("home-inbox")
    .screenshot({ path: `${SHOTS}/01-all-view-no-drafts.png` });

  // Drafts filter: the draft is still listed.
  await page.getByTestId("inbox-filter-trigger").click();
  await page.getByRole("menuitemradio", { name: "Drafts" }).click();
  await page.keyboard.press("Escape");
  const panel = page.getByTestId("home-inbox-drafts");
  await expect(panel).toBeVisible();
  await expect(
    panel.getByText("Draft that must stay out of the All view"),
  ).toBeVisible();
  await waitForAnimations(page);
  await page
    .getByTestId("home-inbox")
    .screenshot({ path: `${SHOTS}/02-drafts-filter-still-lists.png` });
});
