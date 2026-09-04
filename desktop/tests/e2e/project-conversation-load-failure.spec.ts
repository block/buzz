import { expect, test, type Page } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

/**
 * End-to-end guard for the false-empty thread-load bug on the SECOND producer
 * of the shared thread panel: the Projects "channel conversation" panel
 * (`ProjectConversationPanel`). PR #6447 wired the query failure state through
 * `ChannelScreen`, but the Projects surface calls the same `useThreadReplies`
 * and used to hard-code `threadRepliesPending={false}` with no error/retry — so
 * a terminal `/query` failure there painted "No replies in this branch yet"
 * with no recovery, the exact defect the PR fixed one surface over.
 *
 * This drives the REAL Projects panel wiring through the mock bridge: open the
 * project's Channels tab, open a conversation, fail every `get_thread_replies`
 * fetch at the IPC boundary, and assert the error/Retry card renders (never the
 * false-empty). Any regression at the Projects call site (e.g. dropping
 * `threadRepliesError` again, or a static `isError={false}`) ships a visible
 * false-empty and turns this case red.
 */

const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const ROOT_CONTENT = `Projects conversation root ${DEFAULT_MOCK_PUBKEY} buzz`;
const REPLY_CONTENT = "Projects conversation reply body";

// The projects surface is a preview feature — opt in before the app mounts.
async function enableProjectsFeature(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "buzz-feature-overrides-v1",
      JSON.stringify({ projects: true }),
    );
  });
}

// Fail every get_thread_replies fetch at the IPC boundary while the flag is on,
// then let the real mock handler answer once it is cleared. Wrapping
// __TAURI_INTERNALS__.invoke exercises the whole Projects thread-load path —
// the panel's useThreadReplies query, its retry, and the shared region — with
// no source seam to bypass.
async function installThreadFailureSwitch(page: Page) {
  await page.evaluate(() => {
    const w = window as typeof window & {
      __FAIL_THREAD_REPLIES__?: boolean;
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          payload: unknown,
          options: unknown,
        ) => Promise<unknown>;
      };
    };
    w.__FAIL_THREAD_REPLIES__ = false;
    const original = w.__TAURI_INTERNALS__.invoke.bind(w.__TAURI_INTERNALS__);
    w.__TAURI_INTERNALS__.invoke = async (command, payload, options) => {
      if (command === "get_thread_replies" && w.__FAIL_THREAD_REPLIES__) {
        throw new Error("relay unreachable: request timed out");
      }
      return original(command, payload, options);
    };
  });
}

async function setThreadRepliesFailing(page: Page, failing: boolean) {
  await page.evaluate((failing) => {
    (
      window as typeof window & { __FAIL_THREAD_REPLIES__?: boolean }
    ).__FAIL_THREAD_REPLIES__ = failing;
  }, failing);
}

test.describe("project conversation load failure", () => {
  test("terminal fetch failure shows error card, never false-empty; Retry recovers", async ({
    page,
  }) => {
    await enableProjectsFeature(page);
    await installMockBridge(page);
    await page.goto("/", { waitUntil: "domcontentloaded" });

    // Seed the conversation BEFORE opening general — i.e. before its live
    // subscription exists — so the reply lands in the mock store (searchable,
    // and returnable by a successful get_thread_replies) but is never live
    // pushed into the thread-replies cache. If general were open first, the
    // live handler would seed that cache and the panel would render the list
    // branch, masking the error card. The root carries the repository discovery
    // token so it surfaces as the channel's latest discussion hit (what the
    // Channels-tab row opens); its reply omits the token so it never competes
    // to be the opened hit.
    //
    // Wait for the emitter first: emitting before the app boots is a silent
    // no-op (the helper is undefined), which would leave the store empty.
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            typeof (
              window as typeof window & {
                __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: unknown;
              }
            ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
        ),
      )
      .toBe(true);
    const rootId = await page.evaluate(
      ({ author, rootContent, replyContent }) => {
        const now = Math.floor(Date.now() / 1000);
        const emit = (
          window as typeof window & {
            __BUZZ_E2E_EMIT_MOCK_MESSAGE__: (input: {
              channelName: string;
              content: string;
              parentEventId?: string;
              pubkey?: string;
              createdAt?: number;
            }) => { id: string };
          }
        ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        const root = emit({
          channelName: "general",
          content: rootContent,
          pubkey: author,
          createdAt: now,
        });
        emit({
          channelName: "general",
          content: replyContent,
          parentEventId: root.id,
          pubkey: author,
          createdAt: now + 1,
        });
        return root.id;
      },
      {
        author: TEST_IDENTITIES.alice.pubkey,
        rootContent: ROOT_CONTENT,
        replyContent: REPLY_CONTENT,
      },
    );
    expect(rootId).toBeTruthy();

    // Navigate to the project's Channels tab and open the general conversation.
    await page.getByTestId("open-projects-view").click();
    await page.getByTestId("projects-section-projects").click();
    const projectEntry = page
      .locator(
        '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
      )
      .first();
    await expect(projectEntry).toBeVisible({ timeout: 10_000 });
    await projectEntry.click();
    await page.getByTestId("project-home-context-repo-buzz").click();
    await page.getByRole("tab", { name: "Channels", exact: true }).click();
    const channelRow = page
      .getByTestId("project-channel-row")
      .filter({ hasText: "#general" })
      .first();
    await expect(channelRow).toBeVisible({ timeout: 10_000 });

    // Fail every thread-replies fetch, THEN open the conversation: the panel's
    // query and its retry both fail, driving the terminal error state with an
    // empty reply cache and nothing to fall back to.
    await installThreadFailureSwitch(page);
    await setThreadRepliesFailing(page, true);
    await channelRow.click();

    const panel = page.getByTestId("project-conversation-panel");
    await expect(panel).toBeVisible();

    // The load-bearing assertion: a terminal failure paints the error/Retry
    // card and NEVER the false-empty "No replies in this branch yet" state.
    await expect(
      page.getByTestId("message-thread-replies-error"),
    ).toContainText("Couldn't load replies", { timeout: 15_000 });
    await expect(page.getByTestId("message-thread-replies-retry")).toHaveText(
      "Retry",
    );
    await expect(page.getByText("No replies in this branch yet")).toHaveCount(
      0,
    );

    // Retry with fetches succeeding: the reply loads and renders — the error
    // card is gone and no false-empty appears.
    await setThreadRepliesFailing(page, false);
    await page.getByTestId("message-thread-replies-retry").click();
    await expect(page.getByTestId("message-thread-replies-error")).toHaveCount(
      0,
    );
    await expect(page.getByText(REPLY_CONTENT)).toBeVisible();
    await expect(page.getByText("No replies in this branch yet")).toHaveCount(
      0,
    );
  });
});
