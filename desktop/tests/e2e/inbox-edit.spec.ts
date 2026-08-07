import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const CURRENT_PUBKEY = "deadbeef".repeat(8);
const OWN_MESSAGE_ID = "d1".repeat(32);
const FOREIGN_MESSAGE_ID = "e2".repeat(32);
const EMPTY_DELETE_ROOT_ID = "a1".repeat(32);
const EMPTY_DELETE_REPLY_ID = "b2".repeat(32);
const EMPTY_DELETE_ROOT_CONTENT = "Inbox empty-edit root message.";
const EMPTY_DELETE_REPLY_CONTENT =
  "Selected Inbox reply remains after root deletion.";
const COLD_ROOT_ID = "c3".repeat(32);
const COLD_SELECTED_REPLY_ID = "f4".repeat(32);
const COLD_DELETED_EDIT_ID = "0a".repeat(32);
const COLD_SIBLING_REPLY_ID = "1b".repeat(32);
const COLD_SIBLING_EDIT_ID = "2c".repeat(32);
const COLD_ROOT_CONTENT = "Cold Inbox thread root.";
const COLD_SELECTED_ORIGINAL_CONTENT = "Cold Inbox reply original text.";
const COLD_SELECTED_RETRACTED_CONTENT = "Cold Inbox reply retracted edit text.";
const COLD_SIBLING_ORIGINAL_CONTENT = "Cold Inbox sibling reply original text.";
const COLD_SIBLING_EDITED_CONTENT = "Cold Inbox sibling reply edited text.";
const ATTACHMENT_URL = `https://mock.relay/media/${"a".repeat(64)}.pdf`;
const ATTACHMENT_FILENAME = "inbox-edit-proof.pdf";
const SHOTS = "test-results/inbox-edit";

type MockFeedItem = {
  category: "mention";
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
  __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
    channelName: string;
    content: string;
    parentEventId?: string | null;
    pubkey?: string;
    mentionPubkeys?: string[];
    id?: string;
    kind?: number;
    extraTags?: string[][];
  }) => {
    content: string;
    created_at: number;
    id: string;
    kind: number;
    pubkey: string;
    tags: string[][];
  };
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

async function seedEmptyDeleteThread(page: import("@playwright/test").Page) {
  await page.waitForFunction(() => {
    const win = window as MockWindow;
    return (
      typeof win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ === "function"
    );
  });
  await page.evaluate(
    ({
      channelId,
      currentPubkey,
      replyContent,
      replyId,
      rootContent,
      rootId,
    }) => {
      const win = window as MockWindow;
      const emit = win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      const push = win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
      if (!emit || !push)
        throw new Error("Mock bridge helpers are unavailable.");
      const root = emit({
        channelName: "general",
        content: rootContent,
        id: rootId,
        pubkey: currentPubkey,
      });
      const reply = emit({
        channelName: "general",
        content: replyContent,
        id: replyId,
        mentionPubkeys: [currentPubkey],
        parentEventId: root.id,
        pubkey: currentPubkey,
      });
      push({
        category: "mention",
        channel_id: channelId,
        channel_name: "general",
        content: reply.content,
        created_at: reply.created_at,
        id: reply.id,
        kind: reply.kind,
        pubkey: reply.pubkey,
        tags: reply.tags,
      });
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      currentPubkey: CURRENT_PUBKEY,
      replyContent: EMPTY_DELETE_REPLY_CONTENT,
      replyId: EMPTY_DELETE_REPLY_ID,
      rootContent: EMPTY_DELETE_ROOT_CONTENT,
      rootId: EMPTY_DELETE_ROOT_ID,
    },
  );
  await page.getByTestId(`home-inbox-item-${EMPTY_DELETE_REPLY_ID}`).click();
  const detail = page.getByTestId("home-inbox-detail");
  const rootRow = detail.locator(`[data-message-id="${EMPTY_DELETE_ROOT_ID}"]`);
  await expect(rootRow).toContainText(EMPTY_DELETE_ROOT_CONTENT);
  await expect(
    detail.locator(`[data-message-id="${EMPTY_DELETE_REPLY_ID}"]`),
  ).toContainText(EMPTY_DELETE_REPLY_CONTENT);
  return { detail, rootRow };
}

async function submitEmptyEdit(
  page: import("@playwright/test").Page,
  detail: import("@playwright/test").Locator,
  messageId: string,
) {
  await openMoreActions(page, messageId);
  const editAction = page.getByTestId(`edit-message-${messageId}`);
  await expect(editAction).toBeVisible();
  await editAction.click();
  await expect(detail.getByTestId("edit-target")).toBeVisible();
  const input = detail.getByTestId("message-input");
  await expect(input).not.toBeEmpty();
  await input.fill("");
  await expect(input).toBeEmpty();
  await page.keyboard.press("Enter");
}

function editCommandCount(page: import("@playwright/test").Page) {
  return page.evaluate(
    () =>
      ((window as MockWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
        (entry) => entry.command === "edit_message",
      ).length,
  );
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
        category: "mention",
        channel_id: channelId,
        channel_name: "general",
        content: "Inbox thread root.",
        created_at: Math.floor(Date.now() / 1_000),
        id: messageId,
        kind: 9,
        pubkey,
        tags: [
          ["h", channelId],
          ["p", pubkey],
        ],
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

  await detail.getByRole("button", { name: "Attach file" }).click();
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
      input: expect.objectContaining({
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
          category: "mention",
          channel_id: channelId,
          channel_name: "general",
          content: message.content,
          created_at: message.createdAt,
          id: message.id,
          kind: 9,
          pubkey: message.pubkey,
          tags: [
            ["h", channelId],
            ["p", currentPubkey],
          ],
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

test("empty edit confirms deletion of the edited non-selected Inbox row", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  const { detail, rootRow } = await seedEmptyDeleteThread(page);
  const selectedReplyRow = detail.locator(
    `[data-message-id="${EMPTY_DELETE_REPLY_ID}"]`,
  );
  const editsBefore = await editCommandCount(page);

  await submitEmptyEdit(page, detail, EMPTY_DELETE_ROOT_ID);
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText("Delete message?");
  await expect(detail.getByTestId("edit-target")).toBeVisible();
  expect(await editCommandCount(page)).toBe(editsBefore);

  await dialog.getByRole("button", { name: "Delete" }).click();
  await expect(dialog).toBeHidden();
  await expect(detail.getByTestId("edit-target")).toBeHidden();
  await expect(rootRow).toBeHidden();
  await expect(selectedReplyRow).toContainText(EMPTY_DELETE_REPLY_CONTENT);
  expect(await editCommandCount(page)).toBe(editsBefore);

  const deletePayload = await page.evaluate(() => {
    const payloads = (window as MockWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [];
    return payloads.findLast((entry) => entry.command === "delete_message")
      ?.payload;
  });
  expect(deletePayload).toEqual(
    expect.objectContaining({ eventId: EMPTY_DELETE_ROOT_ID }),
  );
});

test("cancelling an empty Inbox edit deletion preserves content and edit mode", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  const { detail, rootRow } = await seedEmptyDeleteThread(page);
  const editsBefore = await editCommandCount(page);

  await submitEmptyEdit(page, detail, EMPTY_DELETE_ROOT_ID);
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText("Delete message?");
  await expect(detail.getByTestId("edit-target")).toBeVisible();
  expect(await editCommandCount(page)).toBe(editsBefore);

  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toBeHidden();
  await expect(detail.getByTestId("edit-target")).toBeVisible();
  await expect(rootRow).toContainText(EMPTY_DELETE_ROOT_CONTENT);
  await expect(
    detail.locator(`[data-message-id="${EMPTY_DELETE_REPLY_ID}"]`),
  ).toContainText(EMPTY_DELETE_REPLY_CONTENT);
  expect(await editCommandCount(page)).toBe(editsBefore);
  expect(
    await page.evaluate(
      () =>
        ((window as MockWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
          (entry) => entry.command === "delete_message",
        ).length,
    ),
  ).toBe(0);
});

// Regression test for the cold Inbox hydration closure. A deletion can target
// an edit event instead of the original message, and `formatTimelineMessages`
// drops an edit only when the edit's own id is in the deletion set. A one-hop
// `#e` fetch over the context message ids returns the edit but not the deletion
// of that edit, so a cold Inbox open re-applies retracted content. The sibling
// reply is the positive control: its live edit is NOT deleted, so it must still
// render as edited after the same cold fetch.
test("cold Inbox open drops an edit that was itself deleted", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  await page.waitForFunction(() => {
    const win = window as MockWindow;
    return (
      typeof win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ === "function"
    );
  });

  await page.evaluate(
    ({
      channelId,
      currentPubkey,
      deletedEditId,
      rootContent,
      rootId,
      selectedOriginalContent,
      selectedReplyId,
      selectedRetractedContent,
      siblingEditId,
      siblingEditedContent,
      siblingOriginalContent,
      siblingReplyId,
    }) => {
      const win = window as MockWindow;
      const emit = win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      const push = win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
      if (!emit || !push) {
        throw new Error("Mock bridge helpers are unavailable.");
      }

      const root = emit({
        channelName: "general",
        content: rootContent,
        id: rootId,
        pubkey: currentPubkey,
      });
      const selectedReply = emit({
        channelName: "general",
        content: selectedOriginalContent,
        id: selectedReplyId,
        mentionPubkeys: [currentPubkey],
        parentEventId: root.id,
        pubkey: currentPubkey,
      });
      const siblingReply = emit({
        channelName: "general",
        content: siblingOriginalContent,
        id: siblingReplyId,
        parentEventId: root.id,
        pubkey: currentPubkey,
      });

      // The retracted edit: published, then deleted by a kind:5 that targets
      // the EDIT event, never the reply itself.
      const retractedEdit = emit({
        channelName: "general",
        content: selectedRetractedContent,
        extraTags: [["e", selectedReply.id]],
        id: deletedEditId,
        kind: 40003,
        pubkey: currentPubkey,
      });
      emit({
        channelName: "general",
        content: "",
        extraTags: [["e", retractedEdit.id]],
        kind: 5,
        pubkey: currentPubkey,
      });

      // Positive control: a live edit with no deletion of its own.
      emit({
        channelName: "general",
        content: siblingEditedContent,
        extraTags: [["e", siblingReply.id]],
        id: siblingEditId,
        kind: 40003,
        pubkey: currentPubkey,
      });

      push({
        category: "mention",
        channel_id: channelId,
        channel_name: "general",
        content: selectedReply.content,
        created_at: selectedReply.created_at,
        id: selectedReply.id,
        kind: selectedReply.kind,
        pubkey: selectedReply.pubkey,
        tags: selectedReply.tags,
      });
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      currentPubkey: CURRENT_PUBKEY,
      deletedEditId: COLD_DELETED_EDIT_ID,
      rootContent: COLD_ROOT_CONTENT,
      rootId: COLD_ROOT_ID,
      selectedOriginalContent: COLD_SELECTED_ORIGINAL_CONTENT,
      selectedReplyId: COLD_SELECTED_REPLY_ID,
      selectedRetractedContent: COLD_SELECTED_RETRACTED_CONTENT,
      siblingEditId: COLD_SIBLING_EDIT_ID,
      siblingEditedContent: COLD_SIBLING_EDITED_CONTENT,
      siblingOriginalContent: COLD_SIBLING_ORIGINAL_CONTENT,
      siblingReplyId: COLD_SIBLING_REPLY_ID,
    },
  );

  // First selection of this item: every edit and deletion must come from the
  // hydration fetch, not from an optimistic local write.
  await page.getByTestId(`home-inbox-item-${COLD_SELECTED_REPLY_ID}`).click();
  const detail = page.getByTestId("home-inbox-detail");
  const siblingRow = detail.locator(
    `[data-message-id="${COLD_SIBLING_REPLY_ID}"]`,
  );
  await expect(siblingRow).toContainText(COLD_SIBLING_EDITED_CONTENT);

  const selectedRow = detail.locator(
    `[data-message-id="${COLD_SELECTED_REPLY_ID}"]`,
  );
  await expect(selectedRow).toContainText(COLD_SELECTED_ORIGINAL_CONTENT);
  await expect(selectedRow).not.toContainText(COLD_SELECTED_RETRACTED_CONTENT);
  await expect(detail).not.toContainText(COLD_SELECTED_RETRACTED_CONTENT);
});
