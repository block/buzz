import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const CHANNEL = "general";
const REPLY_COUNT = 2_419;

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
) {
  await expect
    .poll(async () => {
      return page.evaluate(
        ({ channelName }) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName }) ?? false,
        { channelName: CHANNEL },
      );
    })
    .toBe(true);
}

test("a 2,419-reply thread keeps only the visible rows mounted and remains writable", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId(`channel-${CHANNEL}`).click();
  await waitForMockLiveSubscription(page);

  const rootId = await page.evaluate(
    ({ channelName, replyCount }) => {
      const emit = (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            parentEventId?: string;
            createdAt?: number;
          }) => { id: string };
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) {
        return null;
      }

      const base = Math.floor(Date.now() / 1_000) - replyCount - 60;
      const root = emit({
        channelName,
        content: "Long-running project thread",
        createdAt: base,
      });
      for (let index = 0; index < replyCount; index += 1) {
        emit({
          channelName,
          content: `Reply ${index + 1} of ${replyCount}`,
          parentEventId: root.id,
          createdAt: base + index + 1,
        });
      }
      return root.id;
    },
    { channelName: CHANNEL, replyCount: REPLY_COUNT },
  );
  expect(rootId).not.toBeNull();

  const summary = page.getByTestId("message-thread-summary").last();
  await expect(summary).toBeVisible({ timeout: 30_000 });
  await summary.click();

  const threadPanel = page.getByTestId("message-thread-panel");
  const replies = threadPanel.getByTestId("message-thread-replies");
  await expect(threadPanel).toBeVisible();
  await expect(replies).toHaveAttribute("data-virtualized", "true", {
    timeout: 30_000,
  });
  await expect(
    replies.getByText(`Reply ${REPLY_COUNT} of ${REPLY_COUNT}`, {
      exact: true,
    }),
  ).toBeVisible();

  const mountedReplyRows = replies.getByTestId("message-row");
  await expect.poll(() => mountedReplyRows.count()).toBeGreaterThan(0);
  expect(await mountedReplyRows.count()).toBeLessThan(80);

  const editor = threadPanel
    .getByTestId("message-composer")
    .locator("[contenteditable='true']");
  await editor.fill("Typing stays responsive in the long thread");
  await expect(editor).toHaveText("Typing stays responsive in the long thread");

  await threadPanel.getByRole("button", { name: "Close panel" }).click();
  await expect(threadPanel).toHaveCount(0);
});
