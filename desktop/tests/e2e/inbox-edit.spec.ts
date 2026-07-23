import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const CURRENT_PUBKEY = "deadbeef".repeat(8);
const OWN_MESSAGE_ID = "d1".repeat(32);
const FOREIGN_MESSAGE_ID = "e2".repeat(32);
const ATTACHMENT_URL = `https://mock.relay/media/${"a".repeat(64)}.pdf`;
const ATTACHMENT_FILENAME = "inbox-edit-proof.pdf";
const SHOTS = "test-results/inbox-edit";

type MockFeedItem = {
  category: "activity";
  channel_id: string;
  channel_name: string;
  content: string;
  created_at: number;
  id: string;
  kind: number;
  pubkey: string;
  tags: string[][];
};

type MockWindow = Window & {
  __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{
    command: string;
    payload: unknown;
  }>;
  __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
  __BUZZ_E2E_RELEASE_SEND_MESSAGE_LIVE_ECHO__?: () => number;
  __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
    command: string,
    payload?: Record<string, unknown>,
  ) => Promise<unknown>;
  __BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?: (item: MockFeedItem) => unknown;
};

async function openMoreActions(
  page: import("@playwright/test").Page,
  messageId: string,
) {
  const row = page.locator(`[data-message-id="${messageId}"]`);
  await row.hover();
  await page.getByTestId(`more-actions-${messageId}`).click();
  await expect(page.locator('[role="menuitem"]').first()).toBeVisible();
}

test("editing an immediate attachment reply preserves its media tags", async ({
  page,
}) => {
  await installMockBridge(page, {
    deferSendMessageLiveEcho: true,
    uploadDescriptors: [
      {
        url: ATTACHMENT_URL,
        sha256: "a".repeat(64),
        size: 12345,
        type: "application/pdf",
        uploaded: Math.floor(Date.now() / 1000),
        filename: ATTACHMENT_FILENAME,
      },
    ],
  });
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  await page.waitForFunction(
    () =>
      typeof (window as MockWindow).__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ ===
      "function",
  );

  await page.evaluate(
    ({ channelId, messageId, pubkey }) => {
      const pushFeedItem = (window as MockWindow)
        .__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
      if (!pushFeedItem) {
        throw new Error("Mock feed helper is not installed.");
      }

      pushFeedItem({
        category: "activity",
        channel_id: channelId,
        channel_name: "general",
        content: "Inbox thread root.",
        created_at: Math.floor(Date.now() / 1_000),
        id: messageId,
        kind: 9,
        pubkey,
        tags: [["h", channelId]],
      });
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      messageId: OWN_MESSAGE_ID,
      pubkey: CURRENT_PUBKEY,
    },
  );

  await page.getByTestId(`home-inbox-item-${OWN_MESSAGE_ID}`).click();
  const detail = page.getByTestId("home-inbox-detail");
  await expect(detail).toContainText("Inbox thread root.");

  await detail.getByRole("button", { name: "Attach image" }).click();
  await expect(detail.getByTestId("message-composer")).toContainText(
    ATTACHMENT_FILENAME,
  );
  const input = detail.getByTestId("message-input");
  await input.fill("Attachment reply before editing.");
  await detail.getByTestId("send-message").click();
  await expect(detail.getByText("Sending")).toHaveCount(0);

  const sendPayload = await page.evaluate(() => {
    const payloads = (window as MockWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [];
    return payloads.findLast(
      (entry) => entry.command === "send_channel_message",
    )?.payload;
  });
  expect(sendPayload).toEqual(
    expect.objectContaining({
      mediaTags: [
        [
          "imeta",
          `url ${ATTACHMENT_URL}`,
          "m application/pdf",
          `x ${"a".repeat(64)}`,
          "size 12345",
          `filename ${ATTACHMENT_FILENAME}`,
        ],
      ],
      parentEventId: OWN_MESSAGE_ID,
    }),
  );

  const reply = detail
    .locator('[data-testid="home-inbox-context-message"]')
    .filter({ hasText: "Attachment reply before editing." });
  await expect(reply).toBeVisible();
  await expect(
    reply.getByRole("link", { name: ATTACHMENT_FILENAME }),
  ).toHaveAttribute("href", ATTACHMENT_URL);
  const replyId = await reply.getAttribute("data-message-id");
  expect(replyId).not.toBeNull();
  const replyRow = detail.locator(`[data-message-id="${replyId}"]`);

  await openMoreActions(page, replyId as string);
  await page.getByTestId(`edit-message-${replyId}`).click();
  await expect(detail.getByTestId("edit-target")).toBeVisible();
  await expect(detail.getByTestId("message-composer")).toContainText(
    ATTACHMENT_FILENAME,
  );

  await input.fill("Attachment reply after editing.");
  await page.keyboard.press("Enter");
  await expect(detail.getByTestId("edit-target")).toBeHidden();

  const editPayload = await page.evaluate(() => {
    const payloads = (window as MockWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [];
    return payloads.findLast((entry) => entry.command === "edit_message")
      ?.payload;
  });
  expect(editPayload).toEqual(
    expect.objectContaining({
      eventId: replyId,
      mediaTags: [
        [
          "imeta",
          `url ${ATTACHMENT_URL}`,
          "m application/pdf",
          `x ${"a".repeat(64)}`,
          "size 12345",
          `filename ${ATTACHMENT_FILENAME}`,
        ],
      ],
    }),
  );

  const releasedEchoes = await page.evaluate(() => {
    const release = (window as MockWindow)
      .__BUZZ_E2E_RELEASE_SEND_MESSAGE_LIVE_ECHO__;
    if (!release) {
      throw new Error("Mock send-echo release helper is not installed.");
    }
    return release();
  });
  expect(releasedEchoes).toBe(1);

  await expect(replyRow).toContainText("Attachment reply after editing.");
  await expect(
    replyRow.getByRole("link", { name: ATTACHMENT_FILENAME }),
  ).toHaveAttribute("href", ATTACHMENT_URL);
});

test("Inbox offers a working Edit action only for manageable messages", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  await page.waitForFunction(
    () =>
      typeof (window as MockWindow).__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ ===
      "function",
  );

  await page.evaluate(
    ({
      channelId,
      currentPubkey,
      foreignPubkey,
      foreignMessageId,
      ownMessageId,
    }) => {
      const pushFeedItem = (window as MockWindow)
        .__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
      if (!pushFeedItem) {
        throw new Error("Mock feed helper is not installed.");
      }

      const createdAt = Math.floor(Date.now() / 1_000);
      const messages = [
        {
          content: "My Inbox message before editing.",
          createdAt,
          id: ownMessageId,
          pubkey: currentPubkey,
        },
        {
          content: "Another person's Inbox message.",
          createdAt: createdAt - 1,
          id: foreignMessageId,
          pubkey: foreignPubkey,
        },
      ];

      for (const message of messages) {
        pushFeedItem({
          category: "activity",
          channel_id: channelId,
          channel_name: "general",
          content: message.content,
          created_at: message.createdAt,
          id: message.id,
          kind: 9,
          pubkey: message.pubkey,
          tags: [["h", channelId]],
        });
      }
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      currentPubkey: CURRENT_PUBKEY,
      foreignMessageId: FOREIGN_MESSAGE_ID,
      foreignPubkey: TEST_IDENTITIES.alice.pubkey,
      ownMessageId: OWN_MESSAGE_ID,
    },
  );

  await page.getByTestId(`home-inbox-item-${OWN_MESSAGE_ID}`).click();
  const detail = page.getByTestId("home-inbox-detail");
  await expect(detail).toContainText("My Inbox message before editing.");

  await openMoreActions(page, OWN_MESSAGE_ID);
  await expect(
    page.getByTestId(`edit-message-${OWN_MESSAGE_ID}`),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/01-edit-action.png` });
  await page.getByTestId(`edit-message-${OWN_MESSAGE_ID}`).click();
  await expect(detail.getByTestId("edit-target")).toBeVisible();

  const input = detail.getByTestId("message-input");
  await expect(input).not.toBeEmpty();
  await input.click();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.type("My Inbox message after editing.");
  await page.keyboard.press("Enter");

  await expect(detail.getByTestId("edit-target")).toBeHidden();
  await expect(
    detail.locator(`[data-message-id="${OWN_MESSAGE_ID}"]`),
  ).toContainText("My Inbox message after editing.");
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/02-edited-message.png` });

  await page.getByTestId(`home-inbox-item-${FOREIGN_MESSAGE_ID}`).click();
  await expect(detail).toContainText("Another person's Inbox message.");
  await openMoreActions(page, FOREIGN_MESSAGE_ID);
  await expect(
    page.getByTestId(`edit-message-${FOREIGN_MESSAGE_ID}`),
  ).toHaveCount(0);
  await page.keyboard.press("Escape");

  await page.evaluate(async (channelId) => {
    const testWindow = window as MockWindow;
    const invoke = testWindow.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
    const invalidateChannels = testWindow.__BUZZ_E2E_INVALIDATE_CHANNELS__;
    if (!invoke || !invalidateChannels) {
      throw new Error("Mock channel helpers are not installed.");
    }

    await invoke("archive_channel", { channelId });
    await invalidateChannels();
  }, GENERAL_CHANNEL_ID);

  await page.getByTestId(`home-inbox-item-${OWN_MESSAGE_ID}`).click();
  await expect(detail).toContainText("My Inbox message after editing.");
  await openMoreActions(page, OWN_MESSAGE_ID);
  await expect(page.getByTestId(`edit-message-${OWN_MESSAGE_ID}`)).toHaveCount(
    0,
  );
});
