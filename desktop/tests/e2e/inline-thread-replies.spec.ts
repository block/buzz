import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";
const MOCK_IDENTITY_PUBKEY = "deadbeef".repeat(8);

async function waitForMockLiveSubscription(page: Page, channelName: string) {
  await expect
    .poll(() =>
      page.evaluate(
        ({ name }) =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: name,
          }) ?? false,
        { name: channelName },
      ),
    )
    .toBe(true);
}
async function seedThread(page: Page, channelName: string, label: string) {
  return page.evaluate(
    ({ channel, surface, alicePubkey, bobPubkey, currentPubkey }) => {
      const root = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: channel,
        content: `${surface} planning thread`,
        createdAt: 1_708_000_000,
        pubkey: alicePubkey,
      });
      if (!root) throw new Error("Failed to seed inline thread root");

      for (let index = 0; index < 30; index += 1) {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: channel,
          content: `${surface} later message ${index}`,
          createdAt: 1_708_000_100 + index,
          pubkey: bobPubkey,
        });
      }

      const reply = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: channel,
        content: `${surface} direct reply`,
        createdAt: 1_708_000_200,
        parentEventId: root.id,
        pubkey: bobPubkey,
      });
      if (!reply) throw new Error("Failed to seed inline thread reply");

      const nestedReply = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: channel,
        content: `${surface} nested reply`,
        createdAt: 1_708_000_201,
        parentEventId: reply.id,
        pubkey: currentPubkey,
      });
      if (!nestedReply) throw new Error("Failed to seed nested inline reply");

      return {
        nestedReplyContent: nestedReply.content,
        nestedReplyId: nestedReply.id,
        replyContent: reply.content,
        rootContent: root.content,
        rootId: root.id,
      };
    },
    {
      alicePubkey: TEST_IDENTITIES.alice.pubkey,
      bobPubkey: TEST_IDENTITIES.bob.pubkey,
      currentPubkey: MOCK_IDENTITY_PUBKEY,
      channel: channelName,
      surface: label,
    },
  );
}

for (const surface of [
  {
    channelName: "general",
    label: "Channel",
    screenshot: "test-results/inline-thread-replies/channel.png",
    expectUnreadReply: false,
  },
  {
    channelName: "alice-tyler",
    label: "DM",
    screenshot: "test-results/inline-thread-replies/dm.png",
    expectUnreadReply: true,
  },
]) {
  test(`${surface.label} thread replies expand in the main conversation`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 720 });
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId(`channel-${surface.channelName}`).click();
    await expect(page.getByTestId("chat-title")).toHaveText(
      surface.channelName,
    );
    await waitForMockLiveSubscription(page, surface.channelName);

    const thread = await seedThread(page, surface.channelName, surface.label);
    const summary = page.locator(
      `[data-testid="message-thread-summary"][data-thread-head-id="${thread.rootId}"]`,
    );
    const toggle = page.locator(
      `[data-testid="message-thread-inline-toggle"][data-thread-head-id="${thread.rootId}"]`,
    );
    const timeline = page.getByTestId("message-timeline");
    await summary.scrollIntoViewIfNeeded();
    await expect
      .poll(() =>
        timeline.evaluate(
          (element) =>
            element.scrollTop + element.clientHeight < element.scrollHeight - 8,
        ),
      )
      .toBe(true);
    await expect(summary).toBeVisible();
    await expect(toggle).toHaveAttribute("aria-pressed", "false");
    await expect(
      page.getByText(thread.replyContent, { exact: true }),
    ).toHaveCount(0);
    if (surface.expectUnreadReply) {
      await expect(page.getByTestId("thread-unread-badge")).toBeVisible();
    }

    await toggle.focus();
    await toggle.press("Enter");
    await expect(toggle).toHaveAttribute("aria-pressed", "true");
    await expect(toggle).toHaveText("Hide replies");
    const inlineReplies = page.getByTestId("message-thread-inline-replies");
    await expect(inlineReplies).toBeVisible();
    await expect(
      inlineReplies.getByText(thread.replyContent, { exact: true }),
    ).toBeVisible();
    await expect(
      inlineReplies.getByText(thread.nestedReplyContent, { exact: true }),
    ).toBeVisible();
    if (surface.expectUnreadReply) {
      await expect(page.getByTestId("thread-unread-badge")).toHaveCount(0);
    }
    await expect(page.getByTestId("message-thread-panel")).toHaveCount(0);

    await waitForAnimations(page);
    await inlineReplies.locator("..").screenshot({ path: surface.screenshot });

    const directReplyRow = inlineReplies
      .getByTestId("message-row")
      .filter({ hasText: thread.replyContent });
    await directReplyRow.hover();
    const replyButton = directReplyRow.getByRole("button", { name: "Reply" });
    await replyButton.focus();
    await replyButton.press("Enter");
    let threadPanel = page.getByTestId("message-thread-panel");
    await expect(threadPanel).toBeVisible();
    await expect(threadPanel.getByTestId("message-thread-head")).toContainText(
      thread.rootContent,
    );
    await expect(threadPanel.getByTestId("reply-target")).toContainText(
      thread.replyContent,
    );
    await page.getByTestId("auxiliary-panel-close").click();
    await expect(threadPanel).toHaveCount(0);

    const nestedReplyRow = inlineReplies
      .getByTestId("message-row")
      .filter({ hasText: thread.nestedReplyContent });
    await nestedReplyRow.hover();
    const moreActionsButton = nestedReplyRow.getByLabel("More actions");
    await moreActionsButton.focus();
    await moreActionsButton.press("Enter");
    const editMenuItem = page.getByTestId(
      `edit-message-${thread.nestedReplyId}`,
    );
    await expect(editMenuItem).toBeVisible();
    await editMenuItem.click();
    threadPanel = page.getByTestId("message-thread-panel");
    await expect(threadPanel).toBeVisible();
    await expect(threadPanel.getByTestId("message-thread-head")).toContainText(
      thread.rootContent,
    );
    await expect(threadPanel.getByTestId("edit-target")).toBeVisible();
    await threadPanel.getByRole("button", { name: "Cancel edit" }).click();
    await page.getByTestId("auxiliary-panel-close").click();
    await expect(threadPanel).toHaveCount(0);

    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-pressed", "false");
    await expect(inlineReplies).toHaveCount(0);
    await summary.click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await page.getByTestId("auxiliary-panel-close").click();
    await expect(page.getByTestId("message-thread-panel")).toHaveCount(0);
  });
}
