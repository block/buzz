/**
 * Regression test: a long inbox detail title must truncate instead of
 * running underneath the header controls (open-in-channel, members,
 * huddle, more menu) when the detail pane is narrow.
 *
 * Run: pnpm build:e2e && pnpm exec playwright test --project=smoke \
 *        tests/e2e/inbox-title-overlap.spec.ts
 * Output: test-results/inbox-title-overlap/
 */
import { expect, test } from "@playwright/test";

import type { RelayEvent } from "../../src/shared/api/types";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/inbox-title-overlap";

const ENGINEERING_CHANNEL_ID = "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9";

type MockWindow = Window & {
  __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
    channelName: string;
    content: string;
    parentEventId?: string | null;
    pubkey?: string;
    mentionPubkeys?: string[];
    id?: string;
  }) => RelayEvent;
  __BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?: (item: {
    category: "mention" | "needs_action" | "activity" | "agent_activity";
    channel_id: string | null;
    channel_name: string;
    content: string;
    created_at: number;
    id: string;
    kind: number;
    pubkey: string;
    tags: string[][];
  }) => unknown;
};

test.describe("inbox detail title overflow", () => {
  // Narrow window matching the report; the list pane is widened to its
  // resizable maximum so the detail pane is squeezed to its 300px minimum
  // and "Message in #engineering" cannot fit next to the header controls.
  test.use({ viewport: { width: 1046, height: 838 } });

  test("long title truncates instead of overlapping header controls", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      window.sessionStorage.setItem(
        "buzz.desktop.home-inbox-list-width",
        "520",
      );
    });
    await installMockBridge(page, { mode: "mock" });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await expect(page.getByTestId("home-inbox-list")).toBeVisible({
      timeout: 10_000,
    });
    await page.waitForFunction(() => {
      const win = window as MockWindow;
      return (
        typeof win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
        typeof win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ === "function"
      );
    });

    const mentionId = "ab".repeat(32);
    await page.evaluate(
      ({ channelId, currentPubkey, id, senderPubkey }) => {
        const win = window as MockWindow;
        const emit = win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        const push = win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
        if (!emit || !push) {
          throw new Error("Mock bridge helpers are not installed.");
        }
        const event = emit({
          channelName: "engineering",
          content:
            "Hey, can you look at the release checklist before tomorrow?",
          id,
          mentionPubkeys: [currentPubkey],
          pubkey: senderPubkey,
        });
        push({
          category: "mention",
          channel_id: channelId,
          channel_name: "engineering",
          content: event.content,
          created_at: event.created_at,
          id: event.id,
          kind: event.kind,
          pubkey: event.pubkey,
          tags: event.tags,
        });
      },
      {
        channelId: ENGINEERING_CHANNEL_ID,
        currentPubkey: TEST_IDENTITIES.tyler.pubkey,
        id: mentionId,
        senderPubkey: TEST_IDENTITIES.alice.pubkey,
      },
    );

    const row = page.getByTestId(`home-inbox-item-${mentionId}`);
    await expect(row).toBeVisible();
    await row.click();

    const detail = page.getByTestId("home-inbox-detail");
    await expect(detail).toBeVisible();
    const title = detail.getByTestId("home-inbox-context-title");
    await expect(title).toHaveText("Message in #engineering");
    await waitForAnimations(page);

    const titleBox = await title.boundingBox();
    const controlsBox = await detail
      .getByTestId("home-inbox-open-context")
      .boundingBox();
    if (!titleBox || !controlsBox) {
      throw new Error("Header title or controls not rendered.");
    }

    // Guard against a vacuous pass: the title must still render with real
    // width, and the squeeze must actually engage — the inner span's
    // ellipsis is active only when its content overflows. If layout
    // constants or fonts ever change so the full title fits, this fails
    // loudly instead of silently no longer testing truncation.
    expect(titleBox.width).toBeGreaterThan(0);
    const isTruncated = await title.evaluate((element) => {
      const span = element.querySelector("span");
      return span ? span.scrollWidth > span.clientWidth : false;
    });
    expect(isTruncated).toBe(true);

    await page.screenshot({
      clip: { height: 96, width: 1046, x: 0, y: 0 },
      path: `${SHOTS}/01-header.png`,
    });

    // The title's right edge must stop short of the first header control.
    expect(titleBox.x + titleBox.width).toBeLessThanOrEqual(controlsBox.x + 1);
  });
});
