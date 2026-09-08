import { expect, test, type Page } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

/**
 * End-to-end guard for the false-empty thread-load bug (PR #6447). A terminal
 * thread-replies fetch failure must paint the error/Retry card and NEVER the
 * "No replies in this branch yet" empty card — the two look similar but a failed
 * load presented as an authoritative empty is the user-visible defect.
 *
 * Unit tests cover `ThreadReplyRegion` in isolation, but they cannot mount the
 * full panel (Tiptap/React-Query), so they never observe the production
 * panel→region handoff. This spec drives the REAL panel wiring through the mock
 * bridge, so any regression at that seam (e.g. a static `isError={false}` at the
 * call site) ships a visible false-empty and turns this case red.
 */

// Fail every get_thread_replies fetch at the IPC boundary while the flag is on,
// then let the real mock handler answer once it is cleared. Wrapping
// __TAURI_INTERNALS__.invoke (installed by the mock bridge) exercises the whole
// thread-load path — query hook, retry, panel, region — exactly as production
// does, with no source seam to bypass. A boolean gate (rather than a failure
// countdown) is deterministic: stray thread-reply prefetches during channel
// open can't drain it, so the panel's own load reliably reaches the terminal
// error state, and clearing the flag makes the Retry fetch reliably succeed.
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

// Open the welcome thread deterministically: prefer its summary row, but fall
// back to hovering the root message and clicking Reply. The summary row depends
// on the channel-window query having materialized the seeded reply, which can
// lag; the root Reply affordance opens the same thread panel without that race,
// so the panel is reliably open before the error-card assertions run.
async function openWelcomeThread(page: Page) {
  const summary = page.locator(
    '[data-testid="message-thread-summary"][data-thread-head-id="mock-general-welcome"]',
  );
  if (await summary.count()) {
    await summary.first().click();
  } else {
    const root = page.locator(
      '[data-testid="message-row"][data-message-id="mock-general-welcome"]',
    );
    await root.hover();
    await root.getByRole("button", { name: "Reply" }).click();
  }
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
}

test.describe("thread load failure", () => {
  test("terminal fetch failure shows error card, never false-empty; Retry recovers", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");

    // Seed a reply into the mock store BEFORE opening general, i.e. before its
    // live subscription exists. The reply lands in the channel window (so the
    // "1 reply" thread summary renders) but is never live-pushed into the
    // thread-replies cache — so the thread opens with an empty reply cache and
    // the failed get_thread_replies has nothing to fall back to. If it were
    // emitted while general was open, the live handler would seed the thread
    // cache and the panel would render the list branch, masking the error card.
    //
    // Wait for the emitter to be installed first: emitting before the app has
    // booted is a silent no-op (the helper is undefined), which would leave the
    // store empty and the recovery assertion with nothing to render.
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
    await page.evaluate((pubkey) => {
      (
        window as typeof window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            parentEventId?: string;
            pubkey?: string;
            createdAt?: number;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: "First reply to welcome",
        parentEventId: "mock-general-welcome",
        pubkey,
        createdAt: Math.floor(Date.now() / 1000) - 10,
      });
    }, TEST_IDENTITIES.alice.pubkey);

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await installThreadFailureSwitch(page);

    // Fail every thread-replies fetch, then open the thread: the panel query and
    // its retry both fail, driving the terminal error state.
    await setThreadRepliesFailing(page, true);
    await openWelcomeThread(page);

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

    // Retry with fetches succeeding again: the reply loads and renders — the
    // error card is gone and no false-empty appears.
    await setThreadRepliesFailing(page, false);
    await page.getByTestId("message-thread-replies-retry").click();
    await expect(page.getByTestId("message-thread-replies-error")).toHaveCount(
      0,
    );
    await expect(page.getByText("First reply to welcome")).toBeVisible();
    await expect(page.getByText("No replies in this branch yet")).toHaveCount(
      0,
    );
  });
});
