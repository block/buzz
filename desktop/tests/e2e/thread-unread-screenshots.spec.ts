import { expect, test } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/thread-unread";

type MockMessageEvent = {
  id: string;
  created_at: number;
  pubkey: string;
};

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

async function emitMockMessage(
  page: import("@playwright/test").Page,
  channelName: string,
  content: string,
  options?: {
    parentEventId?: string;
    pubkey?: string;
    createdAt?: number;
    mentionPubkeys?: string[];
  },
): Promise<MockMessageEvent> {
  const event = await page.evaluate(
    ({ ch, msg, parentEventId, pubkey, ts, mentionPubkeys }) => {
      return (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            parentEventId?: string | null;
            pubkey?: string;
            createdAt?: number;
            mentionPubkeys?: string[];
          }) => { id: string; created_at: number; pubkey: string };
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: ch,
        content: msg,
        parentEventId: parentEventId ?? undefined,
        pubkey: pubkey ?? undefined,
        createdAt: ts,
        mentionPubkeys: mentionPubkeys ?? undefined,
      });
    },
    {
      ch: channelName,
      msg: content,
      parentEventId: options?.parentEventId ?? null,
      pubkey: options?.pubkey ?? TEST_IDENTITIES.alice.pubkey,
      ts: options?.createdAt,
      mentionPubkeys: options?.mentionPubkeys,
    },
  );
  if (!event) {
    throw new Error("Mock message emitter is not installed");
  }
  return event;
}

// Unread thread replies must be dated strictly after the read frontier captured
// when the thread was last open. A minute ahead ensures they land past it.
const UNREAD_OFFSET_SECONDS = 60;

function unreadTimestamp() {
  return Math.floor(Date.now() / 1000) + UNREAD_OFFSET_SECONDS;
}

// The pubkey the mock bridge logs in as (mirrors `e2eBridge`'s self identity).
// Mentioning it clears the notify gate so an external reply lights the sidebar
// dot without the user having to participate in the thread first.
const SELF_PUBKEY = "deadbeef".repeat(8);

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
        parentEventId: rootEvent.id,
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
        parentEventId: r1.id,
        pubkey: TEST_IDENTITIES.bob.pubkey,
        createdAt: past + 1,
      },
    );
    // A sibling at r1's level so the tree reads as a branching discussion.
    await emitMockMessage(page, "general", "Separate angle on the same point", {
      parentEventId: r1.id,
      pubkey: TEST_IDENTITIES.charlie.pubkey,
      createdAt: past + 2,
    });
    const r3 = await emitMockMessage(page, "general", "Going deeper still", {
      parentEventId: r2.id,
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
    await expandReply(page, r1.id);
    await expandReply(page, r2.id);
    await page.getByTestId("message-thread-close").click();
    await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();

    // Switch away, then emit the deeper replies past the frontier — these are
    // the unread ones living inside the nested structure.
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    const base = unreadTimestamp();
    const r4 = await emitMockMessage(page, "general", "New nested follow-up", {
      parentEventId: r3.id,
      pubkey: TEST_IDENTITIES.bob.pubkey,
      createdAt: base,
    });
    await emitMockMessage(page, "general", "Deepest unread reply", {
      parentEventId: r4.id,
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
    await expandReply(page, r1.id);
    await expandReply(page, r2.id);
    await expandReply(page, r3.id);
    await expandReply(page, r4.id);

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
      parentEventId: p.id,
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
        parentEventId: c.id,
        pubkey: TEST_IDENTITIES.alice.pubkey,
        createdAt: base,
      },
    );
    await emitMockMessage(page, "general", "Another unread under the branch", {
      parentEventId: c2.id,
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
    await expandReply(page, p.id);
    await expect(inPanelBadge).toHaveCount(0);

    await page.screenshot({
      path: `${SHOTS}/06-thread-expand-clears-subtree-badge.png`,
    });
  });

  test("06-in-panel-badge-bumps-on-live-reply", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // Collapsed branch p with one read child, plus an unread descendant so the
    // in-panel subtree badge starts at a known count.
    const past = Math.floor(Date.now() / 1000) - 60;
    const p = await emitMockMessage(page, "general", "Branch parent", {
      parentEventId: "mock-general-welcome",
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: past,
    });
    const c = await emitMockMessage(page, "general", "Child of branch parent", {
      parentEventId: p.id,
      pubkey: TEST_IDENTITIES.bob.pubkey,
      createdAt: past + 1,
    });

    const summary = page.getByTestId("message-thread-summary").first();
    await expect(summary).toBeVisible();
    await summary.click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await page.getByTestId("message-thread-close").click();
    await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();

    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    const base = unreadTimestamp();
    await emitMockMessage(page, "general", "First unread under branch", {
      parentEventId: c.id,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: base,
    });

    // Reopen WITHOUT expanding p: badge shows the single unread descendant.
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await page.getByTestId("message-thread-summary").first().click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();

    const inPanelBadge = page
      .getByTestId("message-thread-replies")
      .getByTestId("thread-unread-badge");
    await expect(inPanelBadge).toBeVisible();
    await expect(inPanelBadge).toContainText("1");

    // A live reply from another author lands under the open, collapsed branch.
    // The live root marker did NOT advance (panel open ≠ branch expanded), so
    // the badge must bump to 2 on the same tick — readStateVersion-driven
    // recompute is what makes this fire live rather than on a later re-render.
    await emitMockMessage(page, "general", "Second unread under branch", {
      parentEventId: c.id,
      pubkey: TEST_IDENTITIES.bob.pubkey,
      createdAt: base + 1,
    });
    await expect(inPanelBadge).toContainText("2");

    await page.screenshot({
      path: `${SHOTS}/07-in-panel-badge-bumps-on-live-reply.png`,
    });
  });

  test("07-expand-clears-own-branch-badge-sibling-survives", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // Two collapsed sibling branches, each with one read child. branchOld will
    // gain a chronologically EARLIER unread reply; branchNew a LATER one.
    const past = Math.floor(Date.now() / 1000) - 120;
    const branchOld = await emitMockMessage(page, "general", "Older branch", {
      parentEventId: "mock-general-welcome",
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: past,
    });
    const oldChild = await emitMockMessage(page, "general", "Old child", {
      parentEventId: branchOld.id,
      pubkey: TEST_IDENTITIES.bob.pubkey,
      createdAt: past + 1,
    });
    const branchNew = await emitMockMessage(page, "general", "Newer branch", {
      parentEventId: "mock-general-welcome",
      pubkey: TEST_IDENTITIES.charlie.pubkey,
      createdAt: past + 2,
    });
    const newChild = await emitMockMessage(page, "general", "New child", {
      parentEventId: branchNew.id,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: past + 3,
    });

    const summary = page.getByTestId("message-thread-summary").first();
    await expect(summary).toBeVisible();
    await summary.click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await page.getByTestId("message-thread-close").click();
    await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();

    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    // Each branch gains its own unread reply. Badges are computed against the
    // open-time frozen frontier snapshot, and expand-clear is driven by the
    // per-branch `expandedSubtreeReplyIds` gate — NOT a cross-branch live
    // marker sweep. So expanding one branch clears only its OWN badge; the
    // sibling's badge survives until that branch is expanded too.
    const base = unreadTimestamp();
    await emitMockMessage(page, "general", "Unread in older branch", {
      parentEventId: oldChild.id,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: base,
    });
    await emitMockMessage(page, "general", "Unread in newer branch", {
      parentEventId: newChild.id,
      pubkey: TEST_IDENTITIES.bob.pubkey,
      createdAt: base + 30,
    });

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await page.getByTestId("message-thread-summary").first().click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();

    // Both collapsed branches carry an unread badge before any expand.
    const inPanelBadges = page
      .getByTestId("message-thread-replies")
      .getByTestId("thread-unread-badge");
    await expect(inPanelBadges).toHaveCount(2);

    await page.screenshot({
      path: `${SHOTS}/08-two-sibling-badges-before-expand.png`,
    });

    // Expand the LATER branch. Only its OWN badge clears, via the
    // `expandedSubtreeReplyIds` gate against the frozen open-time frontier.
    // The older sibling's badge SURVIVES — the design does not sweep across
    // branches off a live marker.
    await expandReply(page, branchNew.id);
    await expect(inPanelBadges).toHaveCount(1);

    await page.screenshot({
      path: `${SHOTS}/09-expand-clears-own-branch-sibling-survives.png`,
    });

    // Expanding the older branch clears the last remaining badge.
    await expandReply(page, branchOld.id);
    await expect(inPanelBadges).toHaveCount(0);

    await page.screenshot({
      path: `${SHOTS}/10-both-branches-expanded-all-cleared.png`,
    });
  });

  // Regression guard for the Option-1 channel-marker fix: viewing a channel
  // marks ONLY its top-level timeline read, never its thread replies. Before
  // the fix, the channel marker advanced past the newest reply on every view,
  // so the hierarchical effective(thread)=max(thread,channel) cleared the
  // badge the instant the channel was re-entered. This walks open -> badge
  // present -> leave -> RE-ENTER -> badge STILL present. Without the top-level
  // filter on activeReadAt this fails on the second entry.
  test("10-thread-badge-survives-channel-reentry", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // Read frontier over an initial reply, then close the thread.
    await emitMockMessage(page, "general", "First reply to welcome", {
      parentEventId: "mock-general-welcome",
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: Math.floor(Date.now() / 1000) - 10,
    });
    const threadSummary = page.getByTestId("message-thread-summary").first();
    await expect(threadSummary).toBeVisible();
    await threadSummary.click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await page.getByTestId("message-thread-close").click();
    await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();

    // Leave, emit unread replies, return — badge appears (same as test 01).
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");
    const base = unreadTimestamp();
    for (let i = 0; i < 3; i++) {
      await emitMockMessage(page, "general", `Unread reply ${i + 1}`, {
        parentEventId: "mock-general-welcome",
        pubkey: TEST_IDENTITIES.alice.pubkey,
        createdAt: base + i,
      });
    }
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    const badge = page.getByTestId("thread-unread-badge");
    await expect(badge).toBeVisible();
    await expect(badge).toContainText("3");

    // The crux: the first entry above marked the channel read WHILE the unread
    // replies were present. Leave and re-enter WITHOUT opening the thread. If
    // the channel marker had absorbed the replies, the badge would be gone now.
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await expect(badge).toBeVisible();
    await expect(badge).toContainText("3");

    await page.screenshot({
      path: `${SHOTS}/10-thread-badge-survives-channel-reentry.png`,
    });
  });

  // Pins the Fix-A sidebar consequence Will approved on the record: a channel
  // whose ONLY unread is an unopened thread reply KEEPS its sidebar dot after
  // the channel is viewed. Channel-open advances the marker over top-level
  // messages only and does NOT clear observed-latest, so the reply still counts
  // as unread for the sidebar. A future change that re-folds replies into the
  // channel-view marker would drop the dot on view and fail here.
  test("11-sidebar-dot-persists-after-channel-view", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    // Open general and read its thread frontier, so the only thing that can be
    // unread afterward is a NEW reply — not the channel timeline.
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");
    await emitMockMessage(page, "general", "First reply to welcome", {
      parentEventId: "mock-general-welcome",
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: Math.floor(Date.now() / 1000) - 10,
    });
    const threadSummary = page.getByTestId("message-thread-summary").first();
    await expect(threadSummary).toBeVisible();
    await threadSummary.click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await page.getByTestId("message-thread-close").click();

    // Leave, emit an unread reply (thread-reply-only unread), then RE-ENTER
    // general so the channel-open marker fires while the reply is unread.
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");
    await emitMockMessage(page, "general", "Unread reply", {
      parentEventId: "mock-general-welcome",
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: unreadTimestamp(),
    });
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // The crux: leave general. Its sidebar dot must remain — viewing the
    // channel did NOT absorb the unopened thread reply (Fix A).
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");
    await expect(page.getByTestId("channel-unread-general")).toBeVisible();

    await page.screenshot({
      path: `${SHOTS}/11-sidebar-dot-persists.png`,
    });
  });

  // Regression guard for the all-replies window: when the loaded window holds
  // ONLY thread replies (the top-level root has scrolled past the history
  // limit), `latestActiveMessage` is null and `activeReadAt` must NOT fall back
  // to the channel's `lastMessageAt` — that value is reply-inclusive (a reply's
  // own timestamp), so advancing the channel marker to it silently absorbs the
  // unread reply and clears the dot, defeating Fix A. The fix nulls the
  // fallback so the marker advance is suppressed until a real top-level
  // position is known; this pins the dot's survival in that window.
  //
  // The `all-replies` fixture carries a far-future `lastMessageAt` (standing in
  // for the backend's reply-inclusive MAX) with no top-level message in its
  // window — so the buggy fallback would advance the marker past the reply.
  test("12-sidebar-dot-survives-all-replies-window", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    // Emit ONE reply whose parent root is NOT in the window (orphan parent id),
    // so the loaded window is all-replies: no top-level message exists for
    // `latestActiveMessage` to find. The reply mentions the current user so it
    // clears the notify gate and lights the sidebar dot — the observable this
    // test asserts on. (Any notify trigger works; a mention is the simplest.
    // The bug is independent of why the reply is notified.)
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "all-replies");
    await emitMockMessage(page, "all-replies", "Orphan reply mentioning you", {
      parentEventId: "mock-root-scrolled-past-window",
      pubkey: TEST_IDENTITIES.alice.pubkey,
      mentionPubkeys: [SELF_PUBKEY],
      createdAt: unreadTimestamp(),
    });
    await expect(page.getByTestId("channel-unread-all-replies")).toBeVisible();

    // View all-replies while the reply is unread. The all-replies window forces
    // the `activeReadAt` fallback; the bug would advance the channel marker to
    // the far-future `lastMessageAt` and clear the dot.
    await page.getByTestId("channel-all-replies").click();
    await expect(page.getByTestId("chat-title")).toHaveText("all-replies");

    // The crux: leave the channel. Its sidebar dot must remain — the reply is
    // still unread, and viewing the all-replies window must not absorb it.
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await expect(page.getByTestId("channel-unread-all-replies")).toBeVisible();

    await page.screenshot({
      path: `${SHOTS}/12-sidebar-dot-all-replies.png`,
    });
  });

  // Regression guard for BUG-2 (clear-on-read): opening an unread thread marks
  // its visible direct replies read, and the depth-0 badge must clear to zero
  // IN PLACE — without leaving and re-entering the channel. Every other test
  // re-enters the channel to refresh the badge; none asserts that reading the
  // thread alone clears it. The mechanism: the mark-read effect advances the
  // thread frontier over the head + direct replies on open, bumping
  // readStateVersion, which recomputes computeThreadBadgeCounts against the
  // now-advanced snapshot. Before the fix the badge read a frozen open-time
  // snapshot that mark-read never invalidated, so it persisted until channel
  // re-entry. This walks badge=3 -> open thread -> close -> badge gone, all
  // while staying in general.
  test("13-thread-badge-clears-on-read-without-reentry", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // Read frontier over an initial reply, then close the thread (same setup as
    // test 01) so the subsequent replies land strictly past the frontier.
    await emitMockMessage(page, "general", "First reply to welcome", {
      parentEventId: "mock-general-welcome",
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: Math.floor(Date.now() / 1000) - 10,
    });
    const threadSummary = page.getByTestId("message-thread-summary").first();
    await expect(threadSummary).toBeVisible();
    await threadSummary.click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await page.getByTestId("message-thread-close").click();
    await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();

    // Leave, emit unread replies, return — badge appears (same as test 01).
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");
    const base = unreadTimestamp();
    for (let i = 0; i < 3; i++) {
      await emitMockMessage(page, "general", `Unread reply ${i + 1}`, {
        parentEventId: "mock-general-welcome",
        pubkey: TEST_IDENTITIES.alice.pubkey,
        createdAt: base + i,
      });
    }
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    const badge = page.getByTestId("thread-unread-badge");
    await expect(badge).toBeVisible();
    await expect(badge).toContainText("3");

    await page.screenshot({
      path: `${SHOTS}/13-thread-badge-before-read.png`,
    });

    // The crux: open the thread (mark-read advances the frontier past all three
    // direct replies), then close it. Stay in general the entire time — no
    // channel switch. The badge must clear to zero off the readStateVersion
    // recompute alone. Before the BUG-2 fix it would persist at 3 here.
    await page.getByTestId("message-thread-summary").first().click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await page.getByTestId("message-thread-close").click();
    await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();

    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await expect(badge).toHaveCount(0);

    await page.screenshot({
      path: `${SHOTS}/13-thread-badge-clears-on-read.png`,
    });
  });

  // Regression guard for the mention-gate + subtree-count fixes. The viewer is
  // a pure MENTION RECIPIENT of a nested reply in a thread they never authored,
  // participated in, or followed: root `mock-general-alice` (Alice-authored) ->
  // reply A (Alice) -> reply B (Alice, @-mentions self). This fails pre-fix on
  // TWO independent defects:
  //   1. The badge gate `isNotifiedForThread` had no mention term, so a
  //      recipient who never participated/authored/followed gated false and the
  //      badge never appeared at all.
  //   2. `computeThreadBadgeCounts` counted only the root's DIRECT children, so
  //      the nested mention reply B (under A) was never tallied toward the root.
  // After the gate fix the badge appears but undercounts (1, missing B); only
  // after the subtree-count fix does it reach 2. Asserting `2` gates both.
  test("14-mention-only-nested-thread-badge", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // The viewer never replies, authors, or follows Alice's thread — they are a
    // pure mention recipient. Leave general so it goes inactive, then emit the
    // unread chain on a thread that was never read: reply A (direct child of
    // the root) and reply B (nested under A) that @-mentions the viewer. With
    // no prior read the root's frontier seeds to null, so the whole subtree
    // counts unread — A + B = 2. The emitter derives B's true rootId from A's
    // stored thread reference, so B lands in Alice's thread at depth 2.
    const aliceSummary = page.locator(
      '[data-thread-head-id="mock-general-alice"]',
    );
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");
    const base = unreadTimestamp();
    const replyA = await emitMockMessage(page, "general", "Reply A (depth 1)", {
      parentEventId: "mock-general-alice",
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: base,
    });
    await emitMockMessage(page, "general", "Reply B mentioning you (depth 2)", {
      parentEventId: replyA?.id,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      mentionPubkeys: [SELF_PUBKEY],
      createdAt: base + 1,
    });

    // Return to general. The mention on the nested reply must surface a badge on
    // Alice's root — proving the gate now honors mentions — and the count must
    // span the subtree (A + B = 2), proving the count walks past direct
    // children. Pre-fix: no badge; post-gate-only: badge "1"; both fixes: "2".
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const badge = aliceSummary.getByTestId("thread-unread-badge");
    await expect(badge).toBeVisible();
    await expect(badge).toContainText("2");

    // Opening a notified thread advances the frontier to the full subtree max,
    // so it consumes the direct reply A AND the nested mention B at once — the
    // badge clears to 0 in place without drilling into A's collapsed branch.
    await aliceSummary.click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await expect(badge).toHaveCount(0);

    await page.getByTestId("message-thread-close").click();
    await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();

    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await expect(badge).toHaveCount(0);
  });
});
