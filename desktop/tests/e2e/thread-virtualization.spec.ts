import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const CHANNEL = "general";
const REPLY_COUNT = 2_419;
const OFFSCREEN_REPLY_NUMBER = 201;

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

test("a 2,419-reply thread opens an off-screen deep link and stays windowed", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId(`channel-${CHANNEL}`).click();
  await waitForMockLiveSubscription(page);

  const seeded = await page.evaluate(
    ({ channelName, offscreenReplyNumber, replyCount }) => {
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
      let targetId = "";
      for (let index = 0; index < replyCount; index += 1) {
        const reply = emit({
          channelName,
          content: `Reply ${index + 1} of ${replyCount}`,
          parentEventId: root.id,
          createdAt: base + index + 1,
        });
        if (index + 1 === offscreenReplyNumber) {
          targetId = reply.id;
        }
      }
      return { rootId: root.id, targetId };
    },
    {
      channelName: CHANNEL,
      offscreenReplyNumber: OFFSCREEN_REPLY_NUMBER,
      replyCount: REPLY_COUNT,
    },
  );
  expect(seeded?.rootId).toBeTruthy();
  expect(seeded?.targetId).toBeTruthy();

  await page.evaluate(
    ({ messageId, threadRootId }) => {
      const hash = window.location.hash.replace(/^#/, "") || "/";
      const [path, query = ""] = hash.split("?");
      const params = new URLSearchParams(query);
      params.set("messageId", messageId);
      params.set("thread", threadRootId);
      params.set("threadRootId", threadRootId);
      const nextHash = `#${path}?${params.toString()}`;
      window.history.pushState(
        {},
        "",
        `${window.location.pathname}${nextHash}`,
      );
      window.dispatchEvent(new HashChangeEvent("hashchange"));
      window.dispatchEvent(new PopStateEvent("popstate"));
    },
    {
      messageId: seeded?.targetId ?? "",
      threadRootId: seeded?.rootId ?? "",
    },
  );

  const threadPanel = page.getByTestId("message-thread-panel");
  const replies = threadPanel.getByTestId("message-thread-replies");
  const targetReply = replies.getByText(
    `Reply ${OFFSCREEN_REPLY_NUMBER} of ${REPLY_COUNT}`,
    { exact: true },
  );
  const latestReply = replies.getByText(
    `Reply ${REPLY_COUNT} of ${REPLY_COUNT}`,
    { exact: true },
  );
  await expect(threadPanel).toBeVisible();
  await expect(replies).toHaveAttribute("data-virtualized", "true", {
    timeout: 30_000,
  });
  await expect(targetReply).toBeVisible({ timeout: 30_000 });
  await expect(latestReply).toHaveCount(0);

  const mountedReplyRows = replies.getByTestId("message-row");
  await expect.poll(() => mountedReplyRows.count()).toBeGreaterThan(0);
  expect(await mountedReplyRows.count()).toBeLessThan(80);

  await threadPanel.getByTestId("thread-scroll-to-latest").click();
  await expect(latestReply).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(async () =>
      threadPanel.getByTestId("message-thread-body").evaluate((element) => {
        const scroller = element as HTMLElement;
        return (
          scroller.scrollHeight - scroller.clientHeight - scroller.scrollTop <=
          32
        );
      }),
    )
    .toBe(true);
  expect(await mountedReplyRows.count()).toBeLessThan(80);

  const editor = threadPanel
    .getByTestId("message-composer")
    .locator("[contenteditable='true']");
  await editor.fill("Typing stays responsive in the long thread");
  await expect(editor).toHaveText("Typing stays responsive in the long thread");

  await threadPanel.getByRole("button", { name: "Close panel" }).click();
  await expect(threadPanel).toHaveCount(0);
});
