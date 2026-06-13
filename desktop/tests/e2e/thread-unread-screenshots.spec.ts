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

// Nested replies are collapsed behind a summary row that carries the parent's
// id (data-thread-head-id). Expanding one level renders that reply's direct
// children, so the rendered count MUST grow after the click — asserting that
// ties the test to genuine rendered depth: a no-op expansion fails here rather
// than passing silently. A level can reveal several children at once (a
// branch), so the check is "grew", not "grew by one".
async function expandReply(
  page: import("@playwright/test").Page,
  replyId: string,
) {
  const replies = page
    .getByTestId("message-thread-replies")
    .getByTestId("message-row");
  const before = await replies.count();
  await page.locator(`[data-thread-head-id="${replyId}"]`).click();
  await expect.poll(() => replies.count()).toBeGreaterThan(before);
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

  test("04-thread-deep-nested-unread", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // Build a genuinely nested branch by chaining parentEventId: each reply's
    // id becomes the next reply's parent, so threadPanel increments depth per
    // level and renders progressive indentation. The first three levels are
    // dated in the past — they are the "already read" structure.
    const past = Math.floor(Date.now() / 1000) - 60;
    const r1 = await emitMockMessage(
      page,
      "general",
      "Kicking off the design",
      {
        parentEventId: "mock-general-welcome",
        pubkey: TEST_IDENTITIES.alice.pubkey,
        createdAt: past,
      },
    );
    const r2 = await emitMockMessage(
      page,
      "general",
      "Replying one level down",
      {
        parentEventId: r1!.id,
        pubkey: TEST_IDENTITIES.bob.pubkey,
        createdAt: past + 1,
      },
    );
    // A sibling at r1's level so the tree reads as a branching discussion.
    await emitMockMessage(page, "general", "Separate angle on the same point", {
      parentEventId: r1!.id,
      pubkey: TEST_IDENTITIES.charlie.pubkey,
      createdAt: past + 2,
    });
    const r3 = await emitMockMessage(page, "general", "Going deeper still", {
      parentEventId: r2!.id,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: past + 3,
    });

    // Open the thread on the welcome root, expand the read structure
    // (r1 → r2; r3 is a leaf until r4/r5 arrive), then close. This sets the
    // read frontier over everything that currently exists.
    const summary = page.getByTestId("message-thread-summary").first();
    await expect(summary).toBeVisible();
    await summary.click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await expandReply(page, r1!.id);
    await expandReply(page, r2!.id);
    await page.getByTestId("message-thread-close").click();
    await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();

    // Switch away, then emit the deeper replies past the frontier — these are
    // the unread ones living inside the nested structure.
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    const base = unreadTimestamp();
    const r4 = await emitMockMessage(page, "general", "New nested follow-up", {
      parentEventId: r3!.id,
      pubkey: TEST_IDENTITIES.bob.pubkey,
      createdAt: base,
    });
    await emitMockMessage(page, "general", "Deepest unread reply", {
      parentEventId: r4!.id,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: base + 1,
    });

    // Switch back, open the thread, and expand every level down to the
    // unread tail. Each expandReply asserts a row appeared, so green here
    // means the nesting genuinely rendered — not just that a divider exists.
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await page.getByTestId("message-thread-summary").first().click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await expandReply(page, r1!.id);
    await expandReply(page, r2!.id);
    await expandReply(page, r3!.id);
    await expandReply(page, r4!.id);

    // Fully expanded: r1, r2, sibling, r3, r4, r5 — six rendered replies.
    const replies = page
      .getByTestId("message-thread-replies")
      .getByTestId("message-row");
    await expect(replies).toHaveCount(6);

    const divider = page.getByTestId("message-unread-divider");
    await expect(divider).toBeVisible();
    await divider.scrollIntoViewIfNeeded();
    await page.waitForTimeout(300);

    await page.screenshot({
      path: `${SHOTS}/04-thread-deep-nested-unread.png`,
    });
  });

  test("05-thread-in-panel-subtree-badge", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // A branch p (with a child c) plus a leaf sibling of p, all dated in the
    // past so they form the "already read" structure. p keeps a child, so its
    // in-panel row renders as a collapsible summary that can carry a subtree
    // badge; the leaf sibling proves the panel shows other rows too.
    const past = Math.floor(Date.now() / 1000) - 60;
    const p = await emitMockMessage(page, "general", "Branch parent", {
      parentEventId: "mock-general-welcome",
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: past,
    });
    const c = await emitMockMessage(page, "general", "Child of branch parent", {
      parentEventId: p!.id,
      pubkey: TEST_IDENTITIES.bob.pubkey,
      createdAt: past + 1,
    });
    await emitMockMessage(page, "general", "Sibling branch at top level", {
      parentEventId: "mock-general-welcome",
      pubkey: TEST_IDENTITIES.charlie.pubkey,
      createdAt: past + 2,
    });

    // Open the thread to snapshot the read frontier over the existing
    // structure, then close. p stays collapsed — its summary row must remain a
    // collapsed branch for the subtree badge to render.
    const summary = page.getByTestId("message-thread-summary").first();
    await expect(summary).toBeVisible();
    await summary.click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await page.getByTestId("message-thread-close").click();
    await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();

    // Switch away, then emit two unread replies deep under p (children of c) —
    // p's subtree gains unread descendants while p itself stays collapsed.
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    const base = unreadTimestamp();
    const c2 = await emitMockMessage(
      page,
      "general",
      "Unread under the branch",
      {
        parentEventId: c!.id,
        pubkey: TEST_IDENTITIES.alice.pubkey,
        createdAt: base,
      },
    );
    await emitMockMessage(page, "general", "Another unread under the branch", {
      parentEventId: c2!.id,
      pubkey: TEST_IDENTITIES.bob.pubkey,
      createdAt: base + 1,
    });

    // Switch back and open the panel WITHOUT expanding p. The collapsed p row
    // must show its subtree unread count (the two unread descendants).
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await page.getByTestId("message-thread-summary").first().click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();

    // p renders as a collapsed summary row (it has a child); the sibling is a
    // leaf and renders as a plain row, not a summary. Gate on p's summary row
    // first — green here means the branch genuinely rendered, so the badge
    // assertion below is read off a real collapsed row, not an empty panel.
    const inPanelSummaries = page
      .getByTestId("message-thread-replies")
      .getByTestId("message-thread-summary");
    await expect(inPanelSummaries).toHaveCount(1);

    // Scope to message-thread-replies: this is the in-panel per-branch badge,
    // NOT the depth-0 channel-timeline badge that lives outside the container.
    // Against pre-2.5 code the in-panel badge was hard-0, so this fails there.
    const inPanelBadge = page
      .getByTestId("message-thread-replies")
      .getByTestId("thread-unread-badge");
    await expect(inPanelBadge).toBeVisible();
    await expect(inPanelBadge).toContainText("2");

    await page.screenshot({
      path: `${SHOTS}/05-thread-in-panel-subtree-badge.png`,
    });

    // Expanding p marks its whole subtree read; the descendant-inclusive gate
    // (Phase 2.5) drops the badge from p and every revealed row beneath it.
    await expandReply(page, p!.id);
    await expect(inPanelBadge).toHaveCount(0);

    await page.screenshot({
      path: `${SHOTS}/06-thread-expand-clears-subtree-badge.png`,
    });
  });
});
