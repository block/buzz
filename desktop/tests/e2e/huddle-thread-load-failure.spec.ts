import { expect, test, type Page } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

/**
 * Consumer-level guard for the false-empty thread-load bug on the Huddle
 * transcript (PR #6447, Carl r3). The transcript flattens summarized reply
 * subtrees into the chat timeline via `useThreadRepliesForRoots`, whose combine
 * now reports an aggregate `isError`/`refetch`. `useHuddleChannelMessages` used
 * to read only `.events` and drop that state, so one failed subtree left the
 * partial transcript presenting as complete with no warning or recovery.
 *
 * The combine unit test proves the hook REPORTS failure; it cannot catch a
 * consumer discarding it. This drives the REAL Huddle wiring
 * (useHuddleChannelMessages -> ChannelScreen -> ChannelPane) through the mock
 * bridge: two summarized roots, fail only ONE subtree's fetch at the IPC
 * boundary, assert the surviving root's reply still renders AND the retry alert
 * appears, then Retry recovers the failed subtree. Dropping the propagation
 * turns this red.
 */

const HUDDLE_CHANNEL_ID = "11111111-1111-4111-8111-111111111111";
const HUDDLE_PARENT_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

const ROOT_A_CONTENT = "Huddle root A";
const REPLY_A_CONTENT = "Huddle reply A survives";
const ROOT_B_CONTENT = "Huddle root B";
const REPLY_B_CONTENT = "Huddle reply B recovered";

async function waitForMockLiveSubscription(page: Page, channelName: string) {
  await expect
    .poll(() =>
      page.evaluate(
        (name) =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: name,
          }) ?? false,
        channelName,
      ),
    )
    .toBe(true);
}

// Fail get_thread_replies for exactly one root while the flag names it, letting
// the other subtree and the eventual retry succeed. Wrapping the real
// __TAURI_INTERNALS__.invoke exercises the whole per-root query path with no
// source seam to bypass — the aggregate error and the failed-only refetch are
// production behavior, not a test stub.
async function installPerRootThreadFailureSwitch(page: Page) {
  await page.evaluate(() => {
    const w = window as typeof window & {
      __FAIL_THREAD_ROOT__?: string | null;
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          payload: unknown,
          options: unknown,
        ) => Promise<unknown>;
      };
    };
    w.__FAIL_THREAD_ROOT__ = null;
    const original = w.__TAURI_INTERNALS__.invoke.bind(w.__TAURI_INTERNALS__);
    w.__TAURI_INTERNALS__.invoke = async (command, payload, options) => {
      if (
        command === "get_thread_replies" &&
        w.__FAIL_THREAD_ROOT__ != null &&
        (payload as { rootEventId?: string })?.rootEventId ===
          w.__FAIL_THREAD_ROOT__
      ) {
        throw new Error("relay unreachable: request timed out");
      }
      return original(command, payload, options);
    };
  });
}

async function setFailingThreadRoot(page: Page, rootId: string | null) {
  await page.evaluate((rootId) => {
    (
      window as typeof window & { __FAIL_THREAD_ROOT__?: string | null }
    ).__FAIL_THREAD_ROOT__ = rootId;
  }, rootId);
}

test.describe("huddle thread load failure", () => {
  test("a failed reply subtree shows the retry alert beside surviving rows; Retry recovers", async ({
    page,
  }) => {
    await installMockBridge(page, {
      windowLabel: `huddle-${HUDDLE_CHANNEL_ID}`,
      huddle: {
        parentChannelId: HUDDLE_PARENT_ID,
        ephemeralChannelId: HUDDLE_CHANNEL_ID,
        members: [
          { pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" },
          { pubkey: TEST_IDENTITIES.alice.pubkey, role: "bot" },
        ],
        transcriptionEnabled: true,
      },
    });
    await page.goto("/");

    await expect(page.getByTestId("huddle-transcript-intro")).toBeVisible();
    await installPerRootThreadFailureSwitch(page);
    await waitForMockLiveSubscription(page, "huddle");

    // Seed two summarized roots. Each threaded reply emits a live thread
    // summary (descendant_count > 0), so both roots enter the transcript's
    // useThreadRepliesForRoots fan-out and each gets its own subtree fetch. Fail
    // root B's fetch BEFORE seeding so its first fan-out fetch reaches the
    // terminal error while root A resolves — the aggregate reports failure with
    // A's reply already merged, exactly the partial-transcript case.
    const rootB = "b".repeat(64);
    await setFailingThreadRoot(page, rootB);
    const seeded = await page.evaluate(
      ({
        agentPubkey,
        rootAContent,
        replyAContent,
        rootBContent,
        replyBContent,
        rootBId,
      }) => {
        const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        if (!emit) throw new Error("Mock message emitter is not installed.");
        const rootA = emit({ channelName: "huddle", content: rootAContent });
        emit({
          channelName: "huddle",
          content: replyAContent,
          parentEventId: rootA.id,
          pubkey: agentPubkey,
        });
        emit({ channelName: "huddle", content: rootBContent, id: rootBId });
        emit({
          channelName: "huddle",
          content: replyBContent,
          parentEventId: rootBId,
          pubkey: agentPubkey,
        });
        return { rootA: rootA.id };
      },
      {
        agentPubkey: TEST_IDENTITIES.alice.pubkey,
        rootAContent: ROOT_A_CONTENT,
        replyAContent: REPLY_A_CONTENT,
        rootBContent: ROOT_B_CONTENT,
        replyBContent: REPLY_B_CONTENT,
        rootBId: rootB,
      },
    );
    expect(seeded.rootA).toBeTruthy();

    // The load-bearing assertion: root A's reply still renders (non-destructive
    // — a failed subtree does not blank the surviving rows) AND root B's failed
    // fan-out surfaces the retry alert. That alert is rendered ONLY by the
    // Huddle transcript's `huddleThreadRepliesError` propagation; without it the
    // partial transcript would present as complete. (Reply rows themselves flow
    // through the live channel window, so their presence is not the signal — the
    // alert is.)
    await expect(
      page.getByTestId("message-row").filter({ hasText: REPLY_A_CONTENT }),
    ).toBeVisible({ timeout: 15_000 });
    await expect(
      page.getByTestId("message-thread-replies-error"),
    ).toContainText("Couldn't load replies", { timeout: 15_000 });

    // Retry with fetches succeeding: the failed-only refetch recovers root B's
    // subtree, the aggregate error clears, and the alert is dismissed while both
    // replies stay visible.
    await setFailingThreadRoot(page, null);
    await page.getByTestId("message-thread-replies-retry").click();
    await expect(page.getByTestId("message-thread-replies-error")).toHaveCount(
      0,
    );
    await expect(
      page.getByTestId("message-row").filter({ hasText: REPLY_B_CONTENT }),
    ).toBeVisible();
    await expect(
      page.getByTestId("message-row").filter({ hasText: REPLY_A_CONTENT }),
    ).toBeVisible();
  });
});
