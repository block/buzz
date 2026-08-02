import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Regression tests for the thread-pane echo-chamber bug.
 *
 * The thread pane renders the thread-replies query cache, whose only live
 * writer used to be the relay's WebSocket echo. The send mutation painted the
 * channel window optimistically but never the thread cache, so your own
 * thread reply waited a full round-trip plus the relay's post-commit fanout
 * before appearing — the thread pane absorbed 100% of echo lag while the
 * main pane painted instantly.
 *
 * These tests delay the mock relay echo (`sendEchoDelayMs`). The previous
 * mock emitted the echo synchronously inside the send command, which is
 * exactly why no test ever caught this: any UI that incorrectly waited for
 * the echo still painted "immediately" under test.
 */

// The echo lags far beyond the paint deadline; a paint before the deadline
// can only have come from the optimistic cache insert, not the echo.
const ECHO_DELAY_MS = 8_000;
const PAINT_DEADLINE_MS = 2_000;

async function openThreadForMessage(
  page: import("@playwright/test").Page,
  content: string,
) {
  const row = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .filter({ hasText: content })
    .first();
  await row.hover();
  await row.getByRole("button", { name: "Reply" }).click();
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
}

test("own thread reply paints before the relay echo arrives", async ({
  page,
}) => {
  await installMockBridge(page, { sendEchoDelayMs: ECHO_DELAY_MS });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Welcome to #general",
  );

  await openThreadForMessage(page, "Welcome to #general");
  const threadPanel = page.getByTestId("message-thread-panel");
  const threadReplies = threadPanel.getByTestId("message-thread-replies");

  const reply = `Echo-chamber regression ${Date.now()}`;
  await threadPanel.locator('[data-testid="message-input"]').fill(reply);
  await threadPanel.getByTestId("send-message").click();

  // Pre-fix this waited the full ECHO_DELAY_MS; the optimistic insert must
  // paint the reply well before the echo can possibly arrive.
  await expect(threadReplies).toContainText(reply, {
    timeout: PAINT_DEADLINE_MS,
  });

  // The late echo must reconcile with the optimistic entry, not duplicate it.
  await page.waitForTimeout(ECHO_DELAY_MS + 500);
  await expect(
    threadReplies.getByTestId("message-row").filter({ hasText: reply }),
  ).toHaveCount(1);
});

test("reply sent while switching threads lands only in its own thread", async ({
  page,
}) => {
  // Keep the REST send itself slow too, so the thread switch happens while
  // the mutation is still in flight — the optimistic insert must be keyed by
  // the compose-time thread root, not whichever pane is open.
  await installMockBridge(page, {
    sendEchoDelayMs: ECHO_DELAY_MS,
    sendMessageDelayMs: 1_000,
  });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Welcome to #general",
  );

  // Seed a second root so there are two distinct threads to flip between.
  const secondRoot = `Second thread root ${Date.now()}`;
  await expect
    .poll(() =>
      page.evaluate(
        () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
      ),
    )
    .toBe(true);
  await page.evaluate((content) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content,
    });
  }, secondRoot);
  await expect(page.getByTestId("message-timeline")).toContainText(secondRoot);

  await openThreadForMessage(page, "Welcome to #general");
  const threadPanel = page.getByTestId("message-thread-panel");
  const threadReplies = threadPanel.getByTestId("message-thread-replies");

  const reply = `Thread-switch mid-send ${Date.now()}`;
  await threadPanel.locator('[data-testid="message-input"]').fill(reply);
  await threadPanel.getByTestId("send-message").click();

  // Switch to the other thread while the send is still in flight.
  await openThreadForMessage(page, secondRoot);
  await expect(threadPanel.getByTestId("message-thread-head")).toContainText(
    secondRoot,
  );
  // The pending reply belongs to the first thread's cache; it must never
  // bleed into this one — poll across the in-flight window to catch it.
  for (let check = 0; check < 4; check += 1) {
    await page.waitForTimeout(400);
    await expect(threadReplies).not.toContainText(reply);
  }

  // Back on the original thread the reply is there, exactly once.
  await openThreadForMessage(page, "Welcome to #general");
  await expect(threadReplies).toContainText(reply);
  await page.waitForTimeout(ECHO_DELAY_MS);
  await expect(
    threadReplies.getByTestId("message-row").filter({ hasText: reply }),
  ).toHaveCount(1);
});
