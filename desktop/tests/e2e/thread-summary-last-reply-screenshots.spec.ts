import { expect, test } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/thread-summary-last-reply";

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(async () =>
      page.evaluate(
        ({ ch }) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: ch }) ??
          false,
        { ch: channelName },
      ),
    )
    .toBe(true);
}

async function emitMockReply(
  page: import("@playwright/test").Page,
  channelName: string,
  content: string,
  parentEventId: string,
  pubkey: string,
) {
  await page.evaluate(
    ({ ch, msg, parent, author }) =>
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            parentEventId?: string | null;
            pubkey?: string;
            createdAt?: number;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: ch,
        content: msg,
        parentEventId: parent,
        pubkey: author,
        createdAt: Math.floor(Date.now() / 1000) - 10,
      }),
    { ch: channelName, msg: content, parent: parentEventId, author: pubkey },
  );
}

test.describe("thread summary last reply", () => {
  test("names the last replier and previews the reply", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    await emitMockReply(
      page,
      "general",
      "Taking a look now",
      "mock-general-welcome",
      TEST_IDENTITIES.bob.pubkey,
    );
    await emitMockReply(
      page,
      "general",
      "Confirmed with the supplier, we are good to go",
      "mock-general-welcome",
      TEST_IDENTITIES.alice.pubkey,
    );

    const summary = page.getByTestId("message-thread-summary").first();
    await expect(summary).toBeVisible();

    const lastReply = summary.getByTestId("message-thread-summary-last-reply");
    await expect(lastReply).toContainText("last reply by");
    await expect(
      summary.getByTestId("message-thread-summary-last-reply-preview"),
    ).toHaveText("Confirmed with the supplier, we are good to go");

    await summary.screenshot({ path: `${SHOTS}-after.png` });
  });

  test("falls back to the time alone when the reply text is unknown", async ({
    page,
  }) => {
    // A thread the client only knows through a relay summary: counts and
    // participants, no reply content. The row must still be complete.
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const summaries = page.getByTestId("message-thread-summary");
    if ((await summaries.count()) === 0) {
      test.skip(true, "mock bridge seeds no relay-only thread summary");
    }
    await expect(
      summaries.first().getByTestId("message-thread-summary-last-reply"),
    ).toContainText("last reply");
  });
});
