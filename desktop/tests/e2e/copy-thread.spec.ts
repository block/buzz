import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/pr-screenshots";

// Mirrors thread-unread.spec.ts: messages are silently dropped without a live
// subscription, so wait for the channel's subscription before emitting.
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

async function emitMockReply(
  page: import("@playwright/test").Page,
  content: string,
  pubkey: string,
  createdAt: number,
) {
  const event = await page.evaluate(
    ({ msg, pk, ts }) => {
      return (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            parentEventId?: string | null;
            pubkey?: string;
            createdAt?: number;
          }) => { id: string };
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: msg,
        parentEventId: "mock-general-welcome",
        pubkey: pk,
        createdAt: ts,
      });
    },
    { msg: content, pk: pubkey, ts: createdAt },
  );
  if (!event) {
    throw new Error("Mock message emitter is not installed");
  }
}

test("copy thread puts a plain-text transcript of the loaded thread on the clipboard", async ({
  page,
}) => {
  // The mock bridge routes copy_text_to_clipboard through navigator.clipboard,
  // which rejects without an explicit permission grant in headless Chromium.
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: "http://127.0.0.1:4173",
  });
  await installMockBridge(page);
  await page.goto("/");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  // Two replies to the seeded welcome root, from two different authors,
  // backdated so ordering is deterministic (root is seeded at -120s).
  const now = Math.floor(Date.now() / 1000);
  await emitMockReply(
    page,
    "First reply — agents make this easy",
    TEST_IDENTITIES.alice.pubkey,
    now - 50,
  );
  await emitMockReply(
    page,
    "Second reply with two lines\nand a wrapped continuation",
    TEST_IDENTITIES.bob.pubkey,
    now - 40,
  );

  // Open the thread panel from the root's summary row.
  const threadSummary = page.getByTestId("message-thread-summary").first();
  await expect(threadSummary).toBeVisible();
  await threadSummary.click();
  const panel = page.getByTestId("message-thread-panel");
  await expect(panel).toBeVisible();

  const copyButton = page.getByTestId("copy-thread");
  await expect(copyButton).toBeVisible();
  await expect(copyButton).toBeEnabled();

  // Both replies must be rendered before copying so the transcript is full.
  const replies = panel
    .getByTestId("message-thread-replies")
    .getByTestId("message-row");
  await expect(replies).toHaveCount(2);

  await waitForAnimations(page);
  await panel.screenshot({ path: `${SHOTS}/01-copy-thread-action.png` });

  await copyButton.click();

  const toast = page
    .locator("[data-sonner-toast]")
    .filter({ hasText: "Thread copied to clipboard" });
  await expect(toast).toBeVisible();

  const payload = await page.evaluate(() => {
    const log = (
      window as Window & {
        __BUZZ_E2E_COMMAND_LOG__?: Array<{
          command: string;
          payload: Record<string, unknown> | null;
        }>;
      }
    ).__BUZZ_E2E_COMMAND_LOG__;
    return log?.findLast(({ command }) => command === "copy_text_to_clipboard")
      ?.payload;
  });

  const text = (payload as { text?: string } | undefined)?.text;
  expect(typeof text).toBe("string");
  const transcript = text as string;

  // Root first, then the replies in thread order, one block per message.
  const rootIndex = transcript.indexOf("Welcome to #general");
  const firstReplyIndex = transcript.indexOf(
    "First reply — agents make this easy",
  );
  const secondReplyIndex = transcript.indexOf(
    "Second reply with two lines\nand a wrapped continuation",
  );
  expect(rootIndex).toBeGreaterThanOrEqual(0);
  expect(firstReplyIndex).toBeGreaterThan(rootIndex);
  expect(secondReplyIndex).toBeGreaterThan(firstReplyIndex);

  // Each block leads with "author — timestamp": one header per message.
  const headerLines = transcript
    .split("\n")
    .filter((line) => / — .+\d{4} at /.test(line));
  expect(headerLines).toHaveLength(3);

  // Evidence of the confirmation toast for the PR screenshot set.
  await waitForAnimations(page);
  await toast.screenshot({ path: `${SHOTS}/02-copy-thread-toast.png` });
});
