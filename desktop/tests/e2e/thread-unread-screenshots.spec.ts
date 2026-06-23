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
    await expect(
      page
        .getByTestId("message-thread-panel")
        .getByTestId("thread-collapse-rail"),
    ).toHaveCount(0);
    await expect(
      page
        .getByTestId("message-thread-panel")
        .getByTestId("thread-collapse-guide"),
    ).toHaveCount(0);
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

  test("03-thread-badge-casual-browse", async ({ page }) => {
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

    // The thread summary still shows local unread reply state for the visible
    // thread, even though this casual thread should not create a channel-nav
    // unread dot or notification interest.
    const badges = page
      .locator(`[data-thread-head-id="${rootEvent.id}"]`)
      .getByTestId("thread-unread-badge");
    await expect(badges).toHaveCount(1);
    await expect(badges).toContainText("2");

    await page.screenshot({
      path: `${SHOTS}/03-thread-badge-casual-browse.png`,
    });

    // Opening a casual, unmuted thread should clear its local badge too. The
    // badge render gate and read-on-open gate must stay aligned.
    await page.locator(`[data-thread-head-id="${rootEvent.id}"]`).click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await page.getByTestId("message-thread-close").click();
    await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();
    await expect(badges).toHaveCount(0);
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

    await page.getByTestId("message-thread-head").scrollIntoViewIfNeeded();
    await page
      .locator(
        `[data-testid="thread-collapse-rail"][data-thread-head-id="mock-general-welcome"]`,
      )
      .click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await expect(replies).toHaveCount(0);
    const rootStack = page
      .getByTestId("message-thread-replies")
      .locator(
        `[data-testid="message-thread-summary"][data-thread-head-id="mock-general-welcome"]`,
      );
    await expect(rootStack).toBeVisible();
    await expect(rootStack).toContainText("6 replies");
    await rootStack.click();
    await expect(replies).toHaveCount(6);

    await page
      .locator(
        `[data-testid="thread-collapse-guide"][data-thread-head-id="${r1.id}"]`,
      )
      .first()
      .click();
    await expect(replies).toHaveCount(1);
    await expect(
      page
        .getByTestId("message-thread-replies")
        .locator(
          `[data-testid="message-thread-summary"][data-thread-head-id="${r1.id}"]`,
        ),
    ).toBeVisible();
    await expect(
      page
        .getByTestId("message-thread-replies")
        .locator(
          `[data-testid="thread-collapse-rail"][data-thread-head-id="${r1.id}"]`,
        ),
    ).toHaveCount(0);
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

    // v3 contract: expanding a branch marks only its REVEALED direct children
    // read, never the whole subtree. The unread replies sit two levels under p
    // (p -> c -> c2 -> c2-child), so a single expand of p only reveals c — the
    // deeper unread stays collapsed and the badge survives. The badge clears
    // only as each level is individually revealed: expand p (reveals c, badge
    // still counts c2 + c2-child), expand c (reveals c2, read), expand c2
    // (reveals c2-child, read) -> badge clears to 0.
    await expandReply(page, p.id);
    await expect(inPanelBadge).toBeVisible();

    await page.screenshot({
      path: `${SHOTS}/06-thread-expand-clears-subtree-badge.png`,
    });

    await expandReply(page, c.id);
    await expandReply(page, c2.id);
    await expect(inPanelBadge).toHaveCount(0);
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

    // Each branch gains its own unread reply, nested one level under the
    // branch's child (branchNew -> newChild -> unread; branchOld -> oldChild ->
    // unread). Under the v3 per-message contract, expanding a branch marks only
    // its REVEALED direct children read — so revealing newChild does NOT reach
    // the unread reply beneath it. Clearing a branch's badge requires expanding
    // down to the level the unread actually sits at; the sibling branch is
    // never touched, so its badge survives independently.
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

    // Expand the LATER branch down to where its unread sits: revealing
    // branchNew shows newChild (still collapsed over the unread reply, so the
    // badge survives), then revealing newChild marks the unread reply read and
    // clears branchNew's badge. The older sibling is never expanded, so its
    // badge survives — per-message markers isolate each branch.
    await expandReply(page, branchNew.id);
    await expect(inPanelBadges).toHaveCount(2);
    await expandReply(page, newChild.id);
    await expect(inPanelBadges).toHaveCount(1);

    await page.screenshot({
      path: `${SHOTS}/09-expand-clears-own-branch-sibling-survives.png`,
    });

    // Expanding the older branch to its unread depth clears the last badge.
    await expandReply(page, branchOld.id);
    await expandReply(page, oldChild.id);
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

  // Thread-only replies now route through Inbox instead of lighting the
  // channel's sidebar dot. Viewing the channel should still leave the channel
  // dot clear when the only new item is an unopened thread reply.
  test("11-thread-reply-does-not-light-sidebar-dot-after-channel-view", async ({
    page,
  }) => {
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

    // The crux: leave general. Its sidebar dot must stay clear because
    // thread-only reply activity belongs in Inbox, not the channel nav.
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");
    await expect(page.getByTestId("channel-unread-general")).toHaveCount(0);

    await page.screenshot({
      path: `${SHOTS}/11-thread-reply-no-sidebar-dot.png`,
    });
  });

  // Regression guard for the all-replies window: when the loaded window holds
  // ONLY thread replies (the top-level root has scrolled past the history
  // limit), thread-only activity should still stay out of channel unread dots.
  //
  // The `all-replies` fixture carries a far-future `lastMessageAt` (standing in
  // for the backend's reply-inclusive MAX) with no top-level message in its
  // window.
  test("12-thread-reply-does-not-light-all-replies-sidebar-dot", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");

    // Emit ONE reply whose parent root is NOT in the window (orphan parent id),
    // so the loaded window is all-replies: no top-level message exists for
    // `latestActiveMessage` to find. The reply mentions the current user so it
    // clears the notify gate and creates Inbox activity without lighting the
    // channel sidebar dot.
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "all-replies");
    await emitMockMessage(page, "all-replies", "Orphan reply mentioning you", {
      parentEventId: "mock-root-scrolled-past-window",
      pubkey: TEST_IDENTITIES.alice.pubkey,
      mentionPubkeys: [SELF_PUBKEY],
      createdAt: unreadTimestamp(),
    });
    await expect(page.getByTestId("channel-unread-all-replies")).toHaveCount(0);

    // View all-replies while the reply is unread.
    await page.getByTestId("channel-all-replies").click();
    await expect(page.getByTestId("chat-title")).toHaveText("all-replies");

    // The crux: leave the channel. Its sidebar dot should remain clear because
    // thread-only reply activity belongs in Inbox.
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await expect(page.getByTestId("channel-unread-all-replies")).toHaveCount(0);

    await page.screenshot({
      path: `${SHOTS}/12-thread-reply-no-all-replies-sidebar-dot.png`,
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

    // v3 contract: opening a thread marks only its REVEALED direct children
    // read, never the whole subtree. Opening Alice's thread reveals direct
    // child A (read), but nested mention B stays collapsed under A — so the
    // root badge drops to 1, not 0. Expanding A reveals B, marks it read, and
    // clears the badge. The badge predicate reads the live per-message marker,
    // not a subtree-max open ceiling.
    await aliceSummary.click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    await expect(badge).toContainText("1");

    await expandReply(page, replyA?.id ?? "");
    await expect(badge).toHaveCount(0);

    await page.getByTestId("message-thread-close").click();
    await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();

    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await expect(badge).toHaveCount(0);
  });

  // The mark-read/unread menu is a SINGLE item whose label toggles by the
  // clicked message's own read state — driven by the same predicate the unread
  // badge uses (computeThreadUnreadMarker over the message + its forced-unread
  // overlay), so the label and badge can never disagree. Pre-fix the menu
  // rendered TWO simultaneous items ("Mark unread" AND "Mark read") gated only
  // on prop presence. This pins the single-toggle contract in both states and
  // through a full round trip.
  //
  // A top-level message in the OPEN channel is read-on-open by construction:
  // ChannelScreen advances the channel frontier to the newest top-level message
  // and the channel→message fold clears it (this is the badge's own behaviour —
  // a message in the channel you are looking at is never unread). So the only
  // route to an unread top-level message here is the mark-unread action itself,
  // which is exactly what the toggle's forced-unread overlay exists to drive.
  test("15-mark-read-unread-menu-single-toggle", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // Emit an Alice-authored (non-self) top-level message, read-on-open.
    const message = await emitMockMessage(page, "general", "Toggle me", {
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: Math.floor(Date.now() / 1000) - 10,
    });
    const messageId = message?.id ?? "";

    const toggle = page.getByTestId(`mark-read-toggle-${messageId}`);
    const moreActions = page.getByTestId(`more-actions-${messageId}`);

    // Selecting a DropdownMenuItem closes the Radix menu and returns focus to
    // the trigger. Re-clicking the trigger before that close settles is eaten
    // by Radix's closing transition (the menu never reopens). Gate each reopen
    // on the previous menu being fully unmounted — the toggle testid only
    // exists while the dropdown content is mounted, so count 0 is a reliable
    // "closed" signal — then re-hover from a clean state before re-clicking.
    const openMenu = async () => {
      await expect(toggle).toHaveCount(0);
      await page.mouse.move(0, 0);
      await page.getByText("Toggle me").hover();
      await moreActions.click();
      await expect(toggle).toHaveCount(1);
    };

    // Read → the single item reads "Mark unread", and there is exactly one
    // (never both items at once). Clicking it forces the message unread.
    await openMenu();
    await expect(toggle).toHaveText("Mark unread");
    await toggle.click();

    // Now unread → the same single item shows the inverse label. Clicking it
    // marks the message read again.
    await openMenu();
    await expect(toggle).toHaveText("Mark read");
    await toggle.click();

    // Back to read → the label has toggled back, still a single item. The
    // round trip proves the label tracks the live predicate, not prop presence.
    await openMenu();
    await expect(toggle).toHaveText("Mark unread");
  });
});
