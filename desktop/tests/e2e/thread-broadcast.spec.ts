import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// NIP-CW broadcast ("Also send to #channel"): a thread reply opted in via the
// composer checkbox surfaces on the channel timeline as well as in its thread.

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Welcome to #general",
  );
});

async function openThreadOnFirstRootMessage(
  page: import("@playwright/test").Page,
) {
  const rootMessage = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .first();
  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();
  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();
  return threadPanel;
}

test("broadcast reply lands on the channel timeline and the opt-in is one-shot", async ({
  page,
}) => {
  const timestamp = Date.now();
  const broadcastReply = `Broadcast reply ${timestamp}`;
  const threadOnlyReply = `Thread-only reply ${timestamp}`;

  const timeline = page.getByTestId("message-timeline");
  const threadPanel = await openThreadOnFirstRootMessage(page);
  const threadComposer = threadPanel.locator('[data-testid="message-input"]');
  const threadSendButton = threadPanel.getByTestId("send-message");
  const threadReplies = threadPanel.getByTestId("message-thread-replies");
  const broadcastCheckbox = threadPanel.getByTestId(
    "thread-broadcast-checkbox",
  );

  // Direct reply to the thread head — the opt-in is offered and labeled.
  await expect(broadcastCheckbox).toBeVisible();
  await expect(threadPanel.getByTestId("thread-broadcast-label")).toHaveText(
    "Also send to #general",
  );
  await expect(broadcastCheckbox).not.toBeChecked();

  await broadcastCheckbox.click();
  await expect(broadcastCheckbox).toBeChecked();

  await threadComposer.fill(broadcastReply);
  await threadSendButton.click();
  await expect(threadReplies).toContainText(broadcastReply);

  // Unlike a plain thread reply, the broadcast reply is also a timeline row.
  await expect(
    timeline.getByTestId("message-row").filter({ hasText: broadcastReply }),
  ).toHaveCount(1);

  // The opt-in is consumed by the send — the next reply is thread-only.
  await expect(broadcastCheckbox).not.toBeChecked();

  await threadComposer.fill(threadOnlyReply);
  await threadSendButton.click();
  await expect(threadReplies).toContainText(threadOnlyReply);
  await expect(
    timeline.getByTestId("message-row").filter({ hasText: threadOnlyReply }),
  ).toHaveCount(0);
});

test("broadcast opt-in is hidden when targeting a nested reply", async ({
  page,
}) => {
  const timestamp = Date.now();
  const firstReply = `Nesting seed reply ${timestamp}`;

  const threadPanel = await openThreadOnFirstRootMessage(page);
  const threadComposer = threadPanel.locator('[data-testid="message-input"]');
  const threadSendButton = threadPanel.getByTestId("send-message");
  const threadReplies = threadPanel.getByTestId("message-thread-replies");
  const broadcastCheckbox = threadPanel.getByTestId(
    "thread-broadcast-checkbox",
  );

  await expect(broadcastCheckbox).toBeVisible();
  await threadComposer.fill(firstReply);
  await threadSendButton.click();

  const replyRow = threadReplies
    .getByTestId("message-row")
    .filter({ hasText: firstReply });
  await expect(replyRow).toBeVisible();

  // Target the depth-1 reply — the next send would be depth 2, which NIP-CW
  // never surfaces on the channel timeline, so the opt-in disappears.
  await replyRow.hover();
  await replyRow.getByRole("button", { name: "Reply" }).click();
  await expect(broadcastCheckbox).toHaveCount(0);
});
