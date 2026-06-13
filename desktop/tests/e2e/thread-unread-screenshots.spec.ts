import { expect, test } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/thread-unread";

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(async () => {
      return page.evaluate(
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
      );
    })
    .toBe(true);
}

function emitMockMessage(
  page: import("@playwright/test").Page,
  channelName: string,
  content: string,
  options?: {
    parentEventId?: string;
    pubkey?: string;
    createdAt?: number;
  },
) {
  return page.evaluate(
    ({ ch, msg, parentEventId, pubkey, ts }) => {
      return (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            parentEventId?: string | null;
            pubkey?: string;
            createdAt?: number;
          }) => { id: string; created_at: number; pubkey: string };
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: ch,
        content: msg,
        parentEventId: parentEventId ?? undefined,
        pubkey: pubkey ?? undefined,
        createdAt: ts,
      });
    },
    {
      ch: channelName,
      msg: content,
      parentEventId: options?.parentEventId ?? null,
      pubkey: options?.pubkey ?? TEST_IDENTITIES.alice.pubkey,
      ts: options?.createdAt,
    },
  );
}

// Unread thread replies must be dated strictly after the read frontier captured
// when the thread was last open. A minute ahead ensures they land past it.
const UNREAD_OFFSET_SECONDS = 60;

function unreadTimestamp() {
  return Math.floor(Date.now() / 1000) + UNREAD_OFFSET_SECONDS;
}

test.describe("thread unread indicator screenshots", () => {
  test("01-thread-unread-badge", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    // Open general — catch-up adds mock-general-welcome to authoredRootIds
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // Emit an initial reply so the thread summary row appears
    await emitMockMessage(page, "general", "First reply to welcome", {
      parentEventId: "mock-general-welcome",
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: Math.floor(Date.now() / 1000) - 10,
    });

    // Open the thread to establish a read frontier, then close it
    const threadSummary = page.getByTestId("message-thread-summary").first();
    await expect(threadSummary).toBeVisible();
    await threadSummary.click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await page.getByTestId("message-thread-close").click();
    await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();

    // Switch away so general becomes inactive
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    // Emit new thread replies (these will be unread)
    const base = unreadTimestamp();
    for (let i = 0; i < 3; i++) {
      await emitMockMessage(page, "general", `Unread reply ${i + 1}`, {
        parentEventId: "mock-general-welcome",
        pubkey: TEST_IDENTITIES.alice.pubkey,
        createdAt: base + i,
      });
    }

    // Switch back — thread summary should show unread badge
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const badge = page.getByTestId("thread-unread-badge");
    await expect(badge).toBeVisible();
    await expect(badge).toContainText("3");

    await page.screenshot({
      path: `${SHOTS}/01-thread-unread-badge.png`,
    });
  });

  test("02-thread-new-divider", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // Emit an initial reply so the thread summary appears
    await emitMockMessage(page, "general", "Earlier reply", {
      parentEventId: "mock-general-welcome",
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: Math.floor(Date.now() / 1000) - 10,
    });

    // Open thread to establish frontier, then close
    const threadSummary = page.getByTestId("message-thread-summary").first();
    await expect(threadSummary).toBeVisible();
    await threadSummary.click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await page.getByTestId("message-thread-close").click();
    await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();

    // Switch away
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    // Emit new unread replies
    const base = unreadTimestamp();
    for (let i = 0; i < 2; i++) {
      await emitMockMessage(page, "general", `New reply ${i + 1}`, {
        parentEventId: "mock-general-welcome",
        pubkey: TEST_IDENTITIES.alice.pubkey,
        createdAt: base + i,
      });
    }

    // Switch back and open the thread panel
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await page.getByTestId("message-thread-summary").first().click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();

    // The unread divider should appear above the first unread reply
    // (not at index 0 since there's a read reply before the unread ones)
    const divider = page.getByTestId("message-unread-divider");
    await expect(divider).toBeVisible();
    await divider.scrollIntoViewIfNeeded();
    await page.waitForTimeout(300);

    await page.screenshot({
      path: `${SHOTS}/02-thread-new-divider.png`,
    });
  });

  test("03-thread-no-badge-casual-browse", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // Emit a root message from alice (tyler has NO stake in this thread)
    const rootEvent = await emitMockMessage(
      page,
      "general",
      "Alice starts a discussion",
      {
        pubkey: TEST_IDENTITIES.alice.pubkey,
        createdAt: Math.floor(Date.now() / 1000) - 30,
      },
    );

    // Emit replies from bob to alice's thread (tyler still has no stake)
    const base = unreadTimestamp();
    for (let i = 0; i < 2; i++) {
      await emitMockMessage(page, "general", `Bob chimes in ${i + 1}`, {
        parentEventId: rootEvent!.id,
        pubkey: TEST_IDENTITIES.bob.pubkey,
        createdAt: base + i,
      });
    }

    // Wait for thread summary to render
    await page.waitForTimeout(500);

    // The thread summary should NOT show an unread badge — tyler has no
    // notification interest in alice's thread (not participated/authored/followed)
    const badges = page.getByTestId("thread-unread-badge");
    await expect(badges).toHaveCount(0);

    await page.screenshot({
      path: `${SHOTS}/03-thread-no-badge-casual-browse.png`,
    });
  });
});
