import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

// Regression coverage for the reported bug: the top-level timeline shows the
// EDITED message body, but opening its thread panel shows the STALE, un-edited
// original. The thread head is taken from the channel window (which carries the
// edit) but its overlay was previously dropped, relying solely on the async
// thread-reply aux backfill — so before that fetch lands the head renders stale.
//
// We reproduce the divergence deterministically by *gating* the thread-replies
// fetch (`deferThreadReplies`): the channel window already holds the edit, but
// the thread-aux response is held open — it provably cannot land until the test
// releases it. This is stronger than a timed delay: a 4s timer self-heals
// inside Playwright's auto-retry window, so on the buggy code the delayed aux
// backfill could arrive mid-assertion and false-green. A held gate cannot.

const CHANNEL = "general";
const SHOT = "test-results/thread-head-stale-edit";

type MockMessageWindow = Window & {
  __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
    channelName: string;
    content: string;
    parentEventId?: string | null;
    pubkey?: string;
    kind?: number;
    extraTags?: string[][];
  }) => { id: string; created_at: number; pubkey: string } | undefined;
};

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(() =>
      page.evaluate(
        (ch) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: ch }) ??
          false,
        channelName,
      ),
    )
    .toBe(true);
}

test("thread head reflects the channel-window edit even before thread aux loads", async ({
  page,
}) => {
  // Hold the thread-replies (and its aux edit backfill) response open so it
  // provably cannot land until we release it after asserting the head.
  await installMockBridge(page, { deferThreadReplies: true });
  await page.goto("/");
  await page.getByTestId(`channel-${CHANNEL}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(CHANNEL);
  await waitForMockLiveSubscription(page, CHANNEL);

  const ORIGINAL = "can i get a review on these two PRs? (pr 6706, pr 6701)";
  const EDITED = "can i get a review on these PRs? (pr 6706, pr 6701, pr 6503)";

  // 1. Post a top-level message, then edit it — both land in the channel window.
  //    The message MUST be authored by the active identity (tyler): an edit is
  //    only overlaid onto a message when the edit's signer matches the target's
  //    author (formatTimelineMessages authorization), and the mock `edit_message`
  //    command signs with the active identity.
  const root = await page.evaluate(
    ({ channelName, content, pubkey }) =>
      (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName,
        content,
        pubkey,
      }) ?? null,
    {
      channelName: CHANNEL,
      content: ORIGINAL,
      pubkey: TEST_IDENTITIES.tyler.pubkey,
    },
  );
  expect(root?.id).toBeTruthy();
  const rootId = root?.id;
  if (!rootId) throw new Error("mock message emit did not return an id");

  await page.evaluate(
    ({ channelName, rootId, content, pubkey, editKind }) =>
      (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName,
        content,
        pubkey,
        kind: editKind,
        // kind:40003 edit event targeting the root — the same shape the relay
        // delivers into the channel-window subscription.
        extraTags: [["e", rootId]],
      }),
    {
      channelName: CHANNEL,
      rootId,
      content: EDITED,
      pubkey: TEST_IDENTITIES.tyler.pubkey,
      editKind: 40003,
    },
  );

  // 2. The main timeline row shows the edited body (baseline: edit is applied
  //    in the channel window).
  const timelineRow = page.locator(
    `[data-testid="message-row"][data-message-id="${rootId}"]`,
  );
  await expect(timelineRow.getByTestId("message-body")).toContainText(
    "these PRs?",
  );
  await expect(timelineRow.getByTestId("message-body")).not.toContainText(
    "these two PRs?",
  );

  // 3. Open the thread via the reply action (the flow in the bug report).
  const replyButton = page.getByTestId(`reply-message-${rootId}`);
  await replyButton.click({ force: true });
  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();

  // 4. The thread head must show the EDITED body immediately — while the
  //    thread-aux backfill is still gated (provably not yet delivered). Pre-fix
  //    this rendered the stale original ("these two PRs?"), and because the gate
  //    is held (not merely delayed) no backfill can arrive to heal it.
  const headBody = threadPanel
    .locator(`[data-testid="message-row"][data-message-id="${rootId}"]`)
    .getByTestId("message-body");
  await expect(headBody).toContainText("these PRs?");
  await expect(headBody).not.toContainText("these two PRs?");

  // The thread-aux fetch must actually be held: this proves the edited head
  // above came purely from the channel-window overlay, not from a backfill that
  // slipped in. (Guards against the fetch never having been dispatched.)
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as Window & {
              __BUZZ_E2E_THREAD_REPLIES_PENDING__?: () => number;
            }
          ).__BUZZ_E2E_THREAD_REPLIES_PENDING__?.() ?? 0,
      ),
    )
    .toBeGreaterThan(0);

  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT}/thread-head-edited.png` });

  // 5. Release the gate; the held thread-aux backfill now lands and the head
  //    stays edited (dedup against relay-provided aux, no regression).
  await page.evaluate(() =>
    (
      window as Window & {
        __BUZZ_E2E_RELEASE_THREAD_REPLIES__?: () => number;
      }
    ).__BUZZ_E2E_RELEASE_THREAD_REPLIES__?.(),
  );
  await expect(headBody).toContainText("these PRs?");
  await expect(headBody).not.toContainText("these two PRs?");
});
