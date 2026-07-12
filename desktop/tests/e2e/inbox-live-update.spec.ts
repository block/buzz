import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import type { RelayEvent } from "../../src/shared/api/types";

/**
 * Focused regression suite for inbox-stable-conversation (#inbox-regressions):
 *
 * 1. Scroll position is NOT reset when a live reply arrives in the currently-
 *    viewed inbox thread.
 * 2. Composer draft value AND focus survive a live reply arrival.
 * 3. The `?item=` anchor (selectedEventId) remains the old event after the
 *    representative advances to a newer sibling (passive arrival).
 * 4. The correct row stays selected by conversationId (aria-current on the
 *    NEW sibling's row) while `?item=` still points to the old anchor.
 * 5. scrollIntoView fires exactly zero times on a passive representative
 *    advance and exactly once on each deliberate anchor change.
 * 6. Cold `?item=` recovery via getEventById: old anchor absent from feed but
 *    present in mockMessages; newer sibling IS in feed; recovery selects the
 *    sibling row (aria-current), highlights the cold anchor, uses coldRoot.id
 *    as parentEventId. back/forward restore each anchor with full assertions.
 */

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

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
  __BUZZ_E2E_REPLACE_MOCK_FEED_ITEM__?: (
    oldId: string,
    item: {
      category: "mention" | "needs_action" | "activity" | "agent_activity";
      channel_id: string | null;
      channel_name: string;
      content: string;
      created_at: number;
      id: string;
      kind: number;
      pubkey: string;
      tags: string[][];
    },
  ) => unknown;
  __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{
    command: string;
    payload: unknown;
  }>;
  __BUZZ_E2E_SCROLL_INTO_VIEW_COUNT__?: number;
  /** When set to an event ID string, get_event calls for that ID are deferred until __BUZZ_E2E_RELEASE_GET_EVENT__ is called. */
  __BUZZ_E2E_DEFER_GET_EVENT__?: string | null;
  /** Flush all deferred get_event calls; returns the count released. */
  __BUZZ_E2E_RELEASE_GET_EVENT__?: () => number;
  /** Running count of get_event invocations since installMockBridge. */
  __BUZZ_E2E_GET_EVENT_CALL_COUNT__?: number;
};

// ─── helpers ────────────────────────────────────────────────────────────────

async function waitForBridgeReady(page: import("@playwright/test").Page) {
  await page.waitForFunction(() => {
    const win = window as MockWindow;
    return (
      typeof win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ === "function" &&
      typeof win.__BUZZ_E2E_REPLACE_MOCK_FEED_ITEM__ === "function" &&
      typeof win.__BUZZ_E2E_RELEASE_GET_EVENT__ === "function" &&
      Array.isArray(win.__BUZZ_E2E_COMMAND_PAYLOADS__)
    );
  });
}

/** Installs a scrollIntoView spy that counts calls inside the detail pane. */
async function installScrollSpy(page: import("@playwright/test").Page) {
  await page.evaluate(() => {
    (window as MockWindow).__BUZZ_E2E_SCROLL_INTO_VIEW_COUNT__ = 0;
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = function (
      this: Element,
      ...args: Parameters<typeof original>
    ) {
      if (this.closest('[data-testid="home-inbox-detail"]')) {
        (window as MockWindow).__BUZZ_E2E_SCROLL_INTO_VIEW_COUNT__ =
          ((window as MockWindow).__BUZZ_E2E_SCROLL_INTO_VIEW_COUNT__ ?? 0) + 1;
      }
      return original.apply(this, args);
    };
  });
}

async function getScrollIntoViewCount(page: import("@playwright/test").Page) {
  return page.evaluate(
    () => (window as MockWindow).__BUZZ_E2E_SCROLL_INTO_VIEW_COUNT__ ?? 0,
  );
}

async function resetScrollIntoViewCount(page: import("@playwright/test").Page) {
  await page.evaluate(() => {
    (window as MockWindow).__BUZZ_E2E_SCROLL_INTO_VIEW_COUNT__ = 0;
  });
}

/** Clears the captured command-payload log so the next send can be isolated. */
async function clearCommandPayloads(page: import("@playwright/test").Page) {
  await page.evaluate(() => {
    const win = window as MockWindow;
    if (win.__BUZZ_E2E_COMMAND_PAYLOADS__) {
      win.__BUZZ_E2E_COMMAND_PAYLOADS__.length = 0;
    }
  });
}

function getDetailPane(page: import("@playwright/test").Page) {
  return page.getByTestId("home-inbox-detail");
}

function getListPane(page: import("@playwright/test").Page) {
  return page.getByTestId("home-inbox-list");
}

async function getScrollTop(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    const pane = document.querySelector(
      '[data-testid="home-inbox-detail"] [aria-busy]',
    ) as HTMLElement | null;
    return pane?.scrollTop ?? 0;
  });
}

/** Returns the last `send_channel_message` payload captured in the command log. */
async function getLastSendPayload(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    const payloads = (window as MockWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [];
    for (let i = payloads.length - 1; i >= 0; i--) {
      if (payloads[i].command === "send_channel_message") {
        return payloads[i].payload as {
          parentEventId?: string | null;
          content?: string;
          channelId?: string;
        } | null;
      }
    }
    return null;
  });
}

/** Returns the current `?item=` URL param value (or null if absent).
 * The app uses hash-based routing, so `?item=` lives inside the hash:
 * `http://host/#/?item=VALUE` — parse from the hash fragment. */
async function getItemParam(page: import("@playwright/test").Page) {
  const url = new URL(page.url());
  // hash is e.g. "#/?item=abc" — strip leading "#" then parse as URL path+search
  const hashSearch = new URLSearchParams(
    url.hash.slice(url.hash.indexOf("?") + 1),
  );
  return hashSearch.get("item");
}

// ─── seed helpers ─────────────────────────────────────────────────────────

/**
 * Seeds a thread (root → anchor) and pushes anchor as the feed representative.
 * The anchor is a depth-1 reply to root, so parentId = root.id.
 */
async function seedNestedAnchor(page: import("@playwright/test").Page) {
  return page.evaluate(
    ({ channelId, currentPubkey, senderPubkey }) => {
      const win = window as MockWindow;
      const emit = win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      const push = win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
      if (!emit || !push) throw new Error("Bridge helpers not ready");

      const root = emit({
        channelName: "general",
        content: "Thread root message.",
        pubkey: senderPubkey,
        id: "a0".repeat(32),
      });

      const anchor = emit({
        channelName: "general",
        content: "Nested anchor — the reply that becomes the inbox item.",
        parentEventId: root.id,
        pubkey: senderPubkey,
        mentionPubkeys: [currentPubkey],
        id: "b1".repeat(32),
      });

      push({
        id: anchor.id,
        kind: anchor.kind,
        pubkey: anchor.pubkey,
        content: anchor.content,
        created_at: anchor.created_at,
        channel_id: channelId,
        channel_name: "general",
        tags: anchor.tags,
        category: "mention",
      });

      return { root, anchor };
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      currentPubkey: TEST_IDENTITIES.tyler.pubkey,
      senderPubkey: TEST_IDENTITIES.alice.pubkey,
    },
  );
}

/**
 * Injects a newer same-thread sibling via REPLACE, evicting the old anchor
 * from the feed snapshot. Returns the new sibling event.
 */
async function injectNewerSibling(
  page: import("@playwright/test").Page,
  rootId: string,
  anchorId: string,
) {
  return page.evaluate(
    ({ channelId, currentPubkey, senderPubkey, rootEventId, oldAnchorId }) => {
      const win = window as MockWindow;
      const emit = win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      const replace = win.__BUZZ_E2E_REPLACE_MOCK_FEED_ITEM__;
      if (!emit || !replace) throw new Error("Bridge helpers not ready");

      const sibling = emit({
        channelName: "general",
        content:
          "Newer sibling reply — displaces old anchor as representative.",
        parentEventId: rootEventId,
        pubkey: senderPubkey,
        mentionPubkeys: [currentPubkey],
        id: "c2".repeat(32),
      });

      replace(oldAnchorId, {
        id: sibling.id,
        kind: sibling.kind,
        pubkey: sibling.pubkey,
        content: sibling.content,
        created_at: sibling.created_at + 1,
        channel_id: channelId,
        channel_name: "general",
        tags: sibling.tags,
        category: "mention",
      });

      return { sibling, oldAnchorId };
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      currentPubkey: TEST_IDENTITIES.tyler.pubkey,
      senderPubkey: TEST_IDENTITIES.alice.pubkey,
      rootEventId: rootId,
      oldAnchorId: anchorId,
    },
  );
}

// ─── tests ────────────────────────────────────────────────────────────────

test.describe("inbox stable-conversation regressions", () => {
  test("scroll and focused draft preserved; new representative row selected when live sibling displaces anchor", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await expect(getListPane(page)).toBeVisible();
    await waitForBridgeReady(page);
    await installScrollSpy(page);

    const { root, anchor } = await seedNestedAnchor(page);

    // Select the inbox item — this is the deliberate selection that sets the anchor.
    const itemLocator = page.getByTestId(`home-inbox-item-${anchor.id}`);
    await itemLocator.click();
    const detail = getDetailPane(page);
    await expect(detail).toContainText("Nested anchor");
    expect(await getItemParam(page)).toBe(anchor.id);

    // Add filler replies to make the detail pane scrollable.
    await page.evaluate(
      ({ senderPubkey, rootId }) => {
        const emit = (window as MockWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        if (!emit) return;
        for (let i = 0; i < 30; i++) {
          emit({
            channelName: "general",
            content: `Filler reply ${i} to make the list long enough to scroll.`,
            parentEventId: rootId,
            pubkey: senderPubkey,
          });
        }
      },
      { senderPubkey: TEST_IDENTITIES.alice.pubkey, rootId: root.id },
    );

    await expect(detail).toContainText("Filler reply 0");

    // Scroll to a deterministic mid-position: assign scrollTop directly so the
    // test does not depend on wheel-event routing or animation timing.
    // First scroll to bottom (ensures scrollable content is laid out), then
    // pull back up by half to create a non-zero, non-bottom scrollTop.
    await page.evaluate(() => {
      const pane = document.querySelector(
        '[data-testid="home-inbox-detail"] [aria-busy]',
      ) as HTMLElement | null;
      if (!pane) return;
      pane.scrollTop = pane.scrollHeight; // go to bottom
      const mid = Math.max(1, Math.floor(pane.scrollHeight / 2));
      pane.scrollTop = mid; // settle at mid
    });
    // State-based wait: scroll must be non-zero before proceeding.
    await page.waitForFunction(() => {
      const pane = document.querySelector(
        '[data-testid="home-inbox-detail"] [aria-busy]',
      ) as HTMLElement | null;
      return (pane?.scrollTop ?? 0) > 0;
    });
    const scrollTopBefore = await getScrollTop(page);
    expect(scrollTopBefore).toBeGreaterThan(0);

    // Type a draft and confirm focus before the live update.
    const composer = detail.getByTestId("message-input");
    await composer.click();
    await composer.fill("My draft text — must survive the live update");
    await expect(composer).toHaveText(
      "My draft text — must survive the live update",
    );
    await expect(composer).toBeFocused();

    // Reset scroll spy so only calls from the live update are counted.
    await resetScrollIntoViewCount(page);

    // Inject newer sibling — passive representative advance.
    await injectNewerSibling(page, root.id, anchor.id);
    // Wait for the sibling row to appear (proves the feed update landed).
    await expect(
      page.getByTestId(`home-inbox-item-${"c2".repeat(32)}`),
    ).toBeVisible();

    // ── 1: scroll position preserved ──────────────────────────────────
    const scrollTopAfter = await getScrollTop(page);
    expect(Math.abs(scrollTopAfter - scrollTopBefore)).toBeLessThanOrEqual(2);

    // ── 2a: draft text preserved ───────────────────────────────────────
    await expect(composer).toHaveText(
      "My draft text — must survive the live update",
    );

    // ── 2b: composer remains focused ──────────────────────────────────
    await expect(composer).toBeFocused();

    // ── 3: ?item= anchor unchanged (passive arrival keeps old anchor) ──
    expect(await getItemParam(page)).toBe(anchor.id);

    // ── 4a: zero scrollIntoView on passive representative advance ──────
    expect(await getScrollIntoViewCount(page)).toBe(0);

    // ── 4b: new sibling row has aria-current (conversation is selected) ─
    const siblingRow = page.getByTestId(`home-inbox-item-${"c2".repeat(32)}`);
    await expect(siblingRow).toHaveAttribute("aria-current", "true");

    // ── 5: old anchor content still visible in detail ──────────────────
    await expect(detail).toContainText("Nested anchor");

    // ── 6: selected-message highlight on old anchor ────────────────────
    const selectedMsg = detail.getByTestId("home-inbox-selected-message");
    await expect(selectedMsg).toContainText("Nested anchor");

    // ── 7: send → parentEventId = anchor's parentId (root.id) ─────────
    await composer.fill("Test reply to verify parentEventId");
    await detail.getByRole("button", { name: /send/i }).click();
    await page.waitForFunction(() =>
      ((window as MockWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).some(
        (p) => p.command === "send_channel_message",
      ),
    );
    const lastSend = await getLastSendPayload(page);
    expect(lastSend).not.toBeNull();
    expect(lastSend?.parentEventId).toBe(root.id);
  });

  test("deliberate selection recenters exactly once; passive live update does not recenter", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await expect(getListPane(page)).toBeVisible();
    await waitForBridgeReady(page);
    await installScrollSpy(page);

    const { root: rootA, anchor: anchorA } = await seedNestedAnchor(page);

    // Seed a second independent thread.
    const anchorB = await page.evaluate(
      ({ channelId, currentPubkey, senderPubkey }) => {
        const win = window as MockWindow;
        const emit = win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        const push = win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
        if (!emit || !push) throw new Error("Bridge helpers not ready");

        const rootB = emit({
          channelName: "general",
          content: "Second thread root.",
          pubkey: senderPubkey,
          id: "d3".repeat(32),
        });
        const replyB = emit({
          channelName: "general",
          content: "Second thread reply — a different conversation.",
          parentEventId: rootB.id,
          pubkey: senderPubkey,
          mentionPubkeys: [currentPubkey],
          id: "e4".repeat(32),
        });
        push({
          id: replyB.id,
          kind: replyB.kind,
          pubkey: replyB.pubkey,
          content: replyB.content,
          created_at: replyB.created_at,
          channel_id: channelId,
          channel_name: "general",
          tags: replyB.tags,
          category: "mention",
        });
        return replyB;
      },
      {
        channelId: GENERAL_CHANNEL_ID,
        currentPubkey: TEST_IDENTITIES.tyler.pubkey,
        senderPubkey: TEST_IDENTITIES.alice.pubkey,
      },
    );

    // Deliberate select A — wait for detail pane to show content and URL to update.
    await page.getByTestId(`home-inbox-item-${anchorA.id}`).click();
    await expect(getDetailPane(page)).toContainText("Nested anchor");
    await page.waitForFunction(
      ({ id }) => {
        const h = window.location.hash;
        const qs = h.slice(h.indexOf("?") + 1);
        return new URLSearchParams(qs).get("item") === id;
      },
      { id: anchorA.id },
    );

    // Reset spy — measure only calls after this point.
    await resetScrollIntoViewCount(page);

    // Passive live update for A — must NOT recenter.
    const { sibling: siblingA } = await injectNewerSibling(
      page,
      rootA.id,
      anchorA.id,
    );
    // Wait for sibling row to appear (proves the representative advanced).
    await expect(
      page.getByTestId(`home-inbox-item-${siblingA.id}`),
    ).toBeVisible();
    expect(await getScrollIntoViewCount(page)).toBe(0);
    // Sibling row is now aria-current; ?item= unchanged.
    await expect(
      page.getByTestId(`home-inbox-item-${siblingA.id}`),
    ).toHaveAttribute("aria-current", "true");
    expect(await getItemParam(page)).toBe(anchorA.id);

    // Reset spy before deliberate select B.
    await resetScrollIntoViewCount(page);

    // Deliberate select B — must recenter exactly once.
    await page.getByTestId(`home-inbox-item-${anchorB.id}`).click();
    await expect(getDetailPane(page)).toContainText("Second thread reply");
    await page.waitForFunction(
      ({ id }) => {
        const h = window.location.hash;
        const qs = h.slice(h.indexOf("?") + 1);
        return new URLSearchParams(qs).get("item") === id;
      },
      { id: anchorB.id },
    );
    expect(await getItemParam(page)).toBe(anchorB.id);
    expect(await getScrollIntoViewCount(page)).toBe(1);

    // Reset spy before deliberate select back to A (via sibling row).
    await resetScrollIntoViewCount(page);

    // Deliberate select A again (clicking the sibling row) — recenters once,
    // sets ?item= to the sibling (that is the clicked representative).
    await page.getByTestId(`home-inbox-item-${siblingA.id}`).click();
    await expect(getDetailPane(page)).toContainText("Nested anchor");
    await page.waitForFunction(
      ({ id }) => {
        const h = window.location.hash;
        const qs = h.slice(h.indexOf("?") + 1);
        return new URLSearchParams(qs).get("item") === id;
      },
      { id: siblingA.id },
    );
    // A deliberate click on the sibling representative sets ?item= to sibling.
    expect(await getItemParam(page)).toBe(siblingA.id);
    expect(await getScrollIntoViewCount(page)).toBe(1);
  });

  test("cold ?item= recovery: sibling in feed, old anchor in mockMessages only; back/forward restore", async ({
    page,
  }) => {
    // Fixture: coldRoot → coldAnchor + coldSibling (same depth as coldAnchor).
    // - All three are emitted into mockMessages (resolvable via get_event).
    // - Only coldSibling is pushed to the feed.
    // - Cold navigate to ?item=<coldAnchor.id>.
    //
    // Required (no conditionals):
    //   URL = coldAnchor.id
    //   coldSibling row aria-current=true
    //   home-inbox-selected-message contains coldAnchor content
    //   send parentEventId = coldRoot.id
    //
    // Back/forward between two fully resolvable anchors assert URL, current
    // representative row, selected-message highlight, and parentEventId.

    await installMockBridge(page);
    await page.goto("/");
    await expect(getListPane(page)).toBeVisible();
    await waitForBridgeReady(page);

    // ── Seed cold conversation ────────────────────────────────────────
    // All events go into mockMessages via emit; only coldSibling enters feed.
    const { coldRoot, coldAnchor, coldSibling } = await page.evaluate(
      ({ channelId, currentPubkey, senderPubkey }) => {
        const win = window as MockWindow;
        const emit = win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        const push = win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
        if (!emit || !push) throw new Error("Bridge helpers not ready");

        const coldRoot = emit({
          channelName: "general",
          content: "Cold root.",
          pubkey: senderPubkey,
          id: "f5".repeat(32),
        });
        // coldAnchor: depth-1 reply — the cold ?item= target. NOT pushed to feed.
        const coldAnchor = emit({
          channelName: "general",
          content: "Cold anchor — in mockMessages only, not in feed.",
          parentEventId: coldRoot.id,
          pubkey: senderPubkey,
          mentionPubkeys: [currentPubkey],
          id: "f6".repeat(32),
        });
        // coldSibling: same depth, newer — this IS pushed to feed as the representative.
        const coldSibling = emit({
          channelName: "general",
          content: "Cold sibling — newer representative, present in feed.",
          parentEventId: coldRoot.id,
          pubkey: senderPubkey,
          mentionPubkeys: [currentPubkey],
          id: "f7".repeat(32),
        });
        push({
          id: coldSibling.id,
          kind: coldSibling.kind,
          pubkey: coldSibling.pubkey,
          content: coldSibling.content,
          created_at: coldSibling.created_at + 1,
          channel_id: channelId,
          channel_name: "general",
          tags: coldSibling.tags,
          category: "mention",
        });
        return { coldRoot, coldAnchor, coldSibling };
      },
      {
        channelId: GENERAL_CHANNEL_ID,
        currentPubkey: TEST_IDENTITIES.tyler.pubkey,
        senderPubkey: TEST_IDENTITIES.alice.pubkey,
      },
    );

    // ── Seed a second live anchor for the back/forward partner ────────
    const { root: liveRoot, anchor: liveAnchor } = await seedNestedAnchor(page);

    // Select the live anchor first — creates a history entry at ?item=<liveAnchor.id>.
    await page.getByTestId(`home-inbox-item-${liveAnchor.id}`).click();
    await expect(getDetailPane(page)).toContainText("Nested anchor");
    expect(await getItemParam(page)).toBe(liveAnchor.id);

    // ── Navigate to cold anchor (pushes a second history entry) ──────
    // Use real hash assignment so the browser creates a genuine history entry
    // (required for page.goBack() / page.goForward() to work) and emits a real
    // hashchange event (required for hash-router to pick up the new ?item=).
    await page.evaluate(
      ({ anchorId }) => {
        const url = new URL(window.location.href);
        const hashPath = url.hash.slice(1) || "/";
        const hashUrl = new URL(hashPath, "http://x");
        hashUrl.searchParams.set("item", anchorId);
        // Assign to location.hash — creates a real history entry + hashchange.
        window.location.hash = hashUrl.pathname + hashUrl.search;
      },
      { anchorId: coldAnchor.id },
    );

    // Allow cold recovery (async getEventById) to complete.
    // State-based: wait for the sibling row to become aria-current.
    await expect(
      page.getByTestId(`home-inbox-item-${coldSibling.id}`),
    ).toHaveAttribute("aria-current", "true");

    // ── Cold recovery assertions (unconditional) ──────────────────────
    // URL preserved as cold anchor.
    expect(await getItemParam(page)).toBe(coldAnchor.id);

    // coldSibling row (the feed representative) is aria-current.
    await expect(
      page.getByTestId(`home-inbox-item-${coldSibling.id}`),
    ).toHaveAttribute("aria-current", "true");

    // Detail pane shows the cold anchor as the highlighted selected message.
    const selectedMsgCold = getDetailPane(page).getByTestId(
      "home-inbox-selected-message",
    );
    await expect(selectedMsgCold).toContainText("Cold anchor");

    // Send a reply and assert parentEventId = coldRoot.id.
    await clearCommandPayloads(page);
    const composerCold = getDetailPane(page).getByTestId("message-input");
    await composerCold.fill("Reply from cold-recovered anchor");
    await getDetailPane(page).getByRole("button", { name: /send/i }).click();
    await page.waitForFunction(() =>
      ((window as MockWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).some(
        (p) => p.command === "send_channel_message",
      ),
    );
    const coldSend = await getLastSendPayload(page);
    expect(coldSend).not.toBeNull();
    expect(coldSend?.parentEventId).toBe(coldRoot.id);

    // ── Go back to live anchor ────────────────────────────────────────
    await page.goBack();
    await page.waitForFunction(
      ({ id }) => {
        const h = window.location.hash;
        const qs = h.slice(h.indexOf("?") + 1);
        return new URLSearchParams(qs).get("item") === id;
      },
      { id: liveAnchor.id },
    );

    expect(await getItemParam(page)).toBe(liveAnchor.id);
    // liveAnchor's representative row is aria-current.
    await expect(
      page.getByTestId(`home-inbox-item-${liveAnchor.id}`),
    ).toHaveAttribute("aria-current", "true");
    // coldSibling row must NOT be aria-current.
    await expect(
      page.getByTestId(`home-inbox-item-${coldSibling.id}`),
    ).not.toHaveAttribute("aria-current", "true");
    // Detail shows the live anchor as the selected message.
    await expect(
      getDetailPane(page).getByTestId("home-inbox-selected-message"),
    ).toContainText("Nested anchor");
    // Send and assert parentEventId = liveRoot.id.
    await clearCommandPayloads(page);
    const composerBack = getDetailPane(page).getByTestId("message-input");
    await composerBack.fill("Reply after back navigation");
    await getDetailPane(page).getByRole("button", { name: /send/i }).click();
    await page.waitForFunction(() =>
      ((window as MockWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).some(
        (p) => p.command === "send_channel_message",
      ),
    );
    const backSend = await getLastSendPayload(page);
    expect(backSend).not.toBeNull();
    expect(backSend?.parentEventId).toBe(liveRoot.id);

    // ── Go forward back to cold anchor ───────────────────────────────
    await page.goForward();
    await page.waitForFunction(
      ({ id }) => {
        const h = window.location.hash;
        const qs = h.slice(h.indexOf("?") + 1);
        return new URLSearchParams(qs).get("item") === id;
      },
      { id: coldAnchor.id },
    );

    expect(await getItemParam(page)).toBe(coldAnchor.id);
    // coldSibling row is aria-current again.
    await expect(
      page.getByTestId(`home-inbox-item-${coldSibling.id}`),
    ).toHaveAttribute("aria-current", "true");
    // liveAnchor row must NOT be aria-current.
    await expect(
      page.getByTestId(`home-inbox-item-${liveAnchor.id}`),
    ).not.toHaveAttribute("aria-current", "true");
    // Detail shows cold anchor as selected message.
    await expect(
      getDetailPane(page).getByTestId("home-inbox-selected-message"),
    ).toContainText("Cold anchor");
    // Send and assert parentEventId = coldRoot.id again.
    await clearCommandPayloads(page);
    const composerFwd = getDetailPane(page).getByTestId("message-input");
    await composerFwd.fill("Reply after forward navigation");
    await getDetailPane(page).getByRole("button", { name: /send/i }).click();
    await page.waitForFunction(() =>
      ((window as MockWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).some(
        (p) => p.command === "send_channel_message",
      ),
    );
    const fwdSend = await getLastSendPayload(page);
    expect(fwdSend).not.toBeNull();
    expect(fwdSend?.parentEventId).toBe(coldRoot.id);
  });

  test("cold recovery survives mid-flight feedItems update without being cancelled", async ({
    page,
  }) => {
    // Verifies that the in-flight getEventById promise for a cold anchor is NOT
    // cancelled when a concurrent feedItems update (a new live message arriving)
    // causes the cold-recovery effect to re-run.  The old promise must still
    // resolve and populate the recovered item.
    //
    // Sequence (deterministic via defer/release seam):
    //   1. Enable __BUZZ_E2E_DEFER_GET_EVENT__ so get_event is held in-flight.
    //   2. Navigate to ?item=<coldAnchor> (absent from feed) — triggers one
    //      get_event call, which is now queued (provably in-flight).
    //   3. Wait for exactly one deferred call to be queued (state-based).
    //   4. While get_event is provably still in-flight, push an unrelated feed
    //      item — re-runs the cold-recovery effect.
    //   5. Release the deferred get_event.  Recovery must complete.
    //   6. Assert all cold-recovery invariants: URL, aria-current, selected
    //      message highlight, parentEventId — same shape as the primary cold test.

    await installMockBridge(page);
    await page.goto("/");
    await expect(getListPane(page)).toBeVisible();
    await waitForBridgeReady(page);

    // Seed the cold conversation: root → coldAnchor + coldSibling.
    // All three are in mockMessages; only coldSibling is pushed to the feed.
    const { coldRoot, coldAnchor, coldSibling } = await page.evaluate(
      ({ channelId, currentPubkey, senderPubkey }) => {
        const win = window as MockWindow;
        const emit = win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        const push = win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
        if (!emit || !push) throw new Error("Bridge helpers not ready");

        const coldRoot = emit({
          channelName: "general",
          content: "Race-test cold root.",
          pubkey: senderPubkey,
          id: "e8".repeat(32),
        });
        const coldAnchor = emit({
          channelName: "general",
          content: "Race-test cold anchor — in mockMessages only.",
          parentEventId: coldRoot.id,
          pubkey: senderPubkey,
          mentionPubkeys: [currentPubkey],
          id: "e9".repeat(32),
        });
        const coldSibling = emit({
          channelName: "general",
          content: "Race-test cold sibling — in feed as representative.",
          parentEventId: coldRoot.id,
          pubkey: senderPubkey,
          mentionPubkeys: [currentPubkey],
          id: "ea".repeat(32),
        });
        push({
          id: coldSibling.id,
          kind: coldSibling.kind,
          pubkey: coldSibling.pubkey,
          content: coldSibling.content,
          created_at: coldSibling.created_at + 1,
          channel_id: channelId,
          channel_name: "general",
          tags: coldSibling.tags,
          category: "mention",
        });
        return { coldRoot, coldAnchor, coldSibling };
      },
      {
        channelId: GENERAL_CHANNEL_ID,
        currentPubkey: TEST_IDENTITIES.tyler.pubkey,
        senderPubkey: TEST_IDENTITIES.alice.pubkey,
      },
    );

    // ── Step 1: enable the defer seam for coldAnchor.id only ──────────
    // Only get_event calls for coldAnchor.id are held; all other lookups
    // (ancestor context, thread roots) continue to resolve immediately.
    await page.evaluate(
      ({ targetId }) => {
        (window as MockWindow).__BUZZ_E2E_DEFER_GET_EVENT__ = targetId;
        (window as MockWindow).__BUZZ_E2E_GET_EVENT_CALL_COUNT__ = 0;
      },
      { targetId: coldAnchor.id },
    );

    // ── Step 2: navigate to cold anchor — triggers one get_event call ──
    // Use real hash assignment for genuine history entry + real hashchange.
    await page.evaluate(
      ({ anchorId }) => {
        const url = new URL(window.location.href);
        const hashPath = url.hash.slice(1) || "/";
        const hashUrl = new URL(hashPath, "http://x");
        hashUrl.searchParams.set("item", anchorId);
        // Assign to location.hash — creates a real history entry + hashchange.
        window.location.hash = hashUrl.pathname + hashUrl.search;
      },
      { anchorId: coldAnchor.id },
    );

    // ── Step 3: wait until exactly one get_event call is queued ────────
    // State-based: poll the call counter rather than sleeping.
    await page.waitForFunction(
      () => (window as MockWindow).__BUZZ_E2E_GET_EVENT_CALL_COUNT__ === 1,
    );

    // ── Step 4: push an unrelated feed item (re-runs cold-recovery effect)
    // This is provably mid-flight because we haven't called release yet.
    await page.evaluate(
      ({ channelId, currentPubkey, senderPubkey }) => {
        const win = window as MockWindow;
        const emit = win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        const push = win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
        if (!emit || !push) return;
        const unrelated = emit({
          channelName: "general",
          content:
            "Unrelated live item — triggers feedItems update mid-flight.",
          pubkey: senderPubkey,
          mentionPubkeys: [currentPubkey],
          id: "eb".repeat(32),
        });
        push({
          id: unrelated.id,
          kind: unrelated.kind,
          pubkey: unrelated.pubkey,
          content: unrelated.content,
          created_at: unrelated.created_at + 2,
          channel_id: channelId,
          channel_name: "general",
          tags: unrelated.tags,
          category: "mention",
        });
      },
      {
        channelId: GENERAL_CHANNEL_ID,
        currentPubkey: TEST_IDENTITIES.tyler.pubkey,
        senderPubkey: TEST_IDENTITIES.alice.pubkey,
      },
    );

    // Confirm the unrelated item appeared (proves feedItems update fired).
    await expect(
      page.getByTestId(`home-inbox-item-${"eb".repeat(32)}`),
    ).toBeVisible();

    // ── Step 5: release the deferred get_event ─────────────────────────
    const released = await page.evaluate(
      () => (window as MockWindow).__BUZZ_E2E_RELEASE_GET_EVENT__?.() ?? 0,
    );
    expect(released).toBe(1);

    // Confirm no duplicate requests snuck in during the mid-flight update.
    // Count reflects only deferred coldAnchor.id calls (release resets to 0,
    // so this is always 0 after release — the pre-release count was 1).
    expect(
      await page.evaluate(
        () => (window as MockWindow).__BUZZ_E2E_GET_EVENT_CALL_COUNT__ ?? 0,
      ),
    ).toBe(0); // reset to 0 by release

    // ── Step 6: assert cold recovery completed ─────────────────────────
    // URL preserved as cold anchor.
    await page.waitForFunction(
      ({ anchorId }) => {
        const h = window.location.hash;
        const qs = h.slice(h.indexOf("?") + 1);
        return new URLSearchParams(qs).get("item") === anchorId;
      },
      { anchorId: coldAnchor.id },
    );

    // coldSibling row (feed representative) is aria-current.
    await expect(
      page.getByTestId(`home-inbox-item-${coldSibling.id}`),
    ).toHaveAttribute("aria-current", "true");

    // Cold anchor is the highlighted selected message.
    await expect(
      getDetailPane(page).getByTestId("home-inbox-selected-message"),
    ).toContainText("Race-test cold anchor");

    // Send and assert parentEventId = coldRoot.id.
    await clearCommandPayloads(page);
    const composer = getDetailPane(page).getByTestId("message-input");
    await composer.fill("Reply after race-test cold recovery");
    await getDetailPane(page).getByRole("button", { name: /send/i }).click();
    await page.waitForFunction(() =>
      ((window as MockWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).some(
        (p) => p.command === "send_channel_message",
      ),
    );
    const raceSend = await getLastSendPayload(page);
    expect(raceSend).not.toBeNull();
    expect(raceSend?.parentEventId).toBe(coldRoot.id);
  });

  test("clicking newest inbox item after fetch-path load snaps to that message, not mid-thread", async ({
    page,
  }) => {
    // Regression: the selected message was visible in `displayMessages` at
    // click time (it was the feed representative), but useInboxThreadContext
    // fetched older ancestors and prepended them above the viewport, shifting
    // scrollTop to mid-thread while the newest message slid below the fold.
    //
    // Fix: the deliberate-selection center is deferred until
    // isThreadContextLoading transitions true → false (fetch settled).
    //
    // Fixture:
    //   - fetchRoot + 10 older replies are in mockMessages only (fetch path).
    //   - fetchNewest (reply to fetchRoot) is pushed to the feed as the
    //     representative.
    //   - The thread is long enough that the detail pane requires scrolling.
    //
    // This test must FAIL at 5536cede5 (center fires before fetch, lands
    // mid-thread) and PASS with the fix (center fires after fetch, lands on
    // the newest message).

    await installMockBridge(page);
    await page.goto("/");
    await expect(getListPane(page)).toBeVisible();
    await waitForBridgeReady(page);
    await installScrollSpy(page);

    // ── Seed the fetch-path conversation ─────────────────────────────
    // All events go into mockMessages via emit; only fetchNewest enters the feed.
    const { fetchRoot, fetchNewest } = await page.evaluate(
      ({ channelId, currentPubkey, senderPubkey }) => {
        const win = window as MockWindow;
        const emit = win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        const push = win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
        if (!emit || !push) throw new Error("Bridge helpers not ready");

        const fetchRoot = emit({
          channelName: "general",
          content: "Fetch-path thread root.",
          pubkey: senderPubkey,
          id: "d0".repeat(32),
        });

        // 10 older replies — in mockMessages, fetched via the context load.
        // Each is tall enough to push the newest message below the fold.
        for (let i = 1; i <= 10; i++) {
          emit({
            channelName: "general",
            content: `Fetch-path older reply ${i} — long enough to occupy vertical space in the pane so that the total thread height requires scrolling to see the newest message.`,
            parentEventId: fetchRoot.id,
            pubkey: senderPubkey,
            id: `d${i.toString(16).padStart(1, "0")}`.repeat(32),
          });
        }

        // fetchNewest: the feed representative — the message the user clicks.
        const fetchNewest = emit({
          channelName: "general",
          content:
            "Fetch-path newest reply — the one the user clicks in inbox.",
          parentEventId: fetchRoot.id,
          pubkey: senderPubkey,
          mentionPubkeys: [currentPubkey],
          id: "df".repeat(32),
        });

        push({
          id: fetchNewest.id,
          kind: fetchNewest.kind,
          pubkey: fetchNewest.pubkey,
          content: fetchNewest.content,
          created_at: fetchNewest.created_at + 11,
          channel_id: channelId,
          channel_name: "general",
          tags: fetchNewest.tags,
          category: "mention",
        });

        return { fetchRoot, fetchNewest };
      },
      {
        channelId: GENERAL_CHANNEL_ID,
        currentPubkey: TEST_IDENTITIES.tyler.pubkey,
        senderPubkey: TEST_IDENTITIES.alice.pubkey,
      },
    );

    // Confirm the feed representative row appeared in the inbox list.
    const newestRow = page.getByTestId(`home-inbox-item-${fetchNewest.id}`);
    await expect(newestRow).toBeVisible();

    // Reset the spy so only the click's center is counted.
    await resetScrollIntoViewCount(page);

    // ── Click the newest reply row ────────────────────────────────────
    await newestRow.click();
    const detail = getDetailPane(page);
    await expect(detail).toContainText("Fetch-path newest reply");

    // Wait for the thread context fetch to complete: the detail pane transitions
    // from aria-busy=true → aria-busy=false once isThreadContextLoading settles.
    // State-based: poll until aria-busy is no longer "true".
    await page.waitForFunction(() => {
      const scrollable = document.querySelector(
        '[data-testid="home-inbox-detail"] [aria-busy]',
      );
      return scrollable?.getAttribute("aria-busy") !== "true";
    });

    // After settle, confirm the older replies are now rendered (fetch landed).
    await expect(detail).toContainText("Fetch-path older reply 1");

    // ── Assert: selected message is within the pane viewport ─────────
    // The selected message (fetchNewest) must be visible inside the scroll
    // container — i.e. its bounding rect overlaps with the pane's visible area.
    const isSelectedMessageInViewport = await page.evaluate(() => {
      const selectedMsg = document.querySelector<HTMLElement>(
        '[data-testid="home-inbox-selected-message"]',
      );
      const pane = document.querySelector<HTMLElement>(
        '[data-testid="home-inbox-detail"] [aria-busy]',
      );
      if (!selectedMsg || !pane) return false;
      const paneRect = pane.getBoundingClientRect();
      const msgRect = selectedMsg.getBoundingClientRect();
      // At least the top half of the element must overlap the pane's visible area.
      const msgCenter = msgRect.top + msgRect.height / 2;
      return msgCenter >= paneRect.top && msgCenter <= paneRect.bottom;
    });
    expect(isSelectedMessageInViewport).toBe(true);

    // ── Assert: exactly one programmatic scroll fired ─────────────────
    expect(await getScrollIntoViewCount(page)).toBe(1);

    // ── Bonus: passive live arrivals after settle still trigger zero ──
    // Inject a sibling to confirm the scroll-stability invariant is intact.
    await page.evaluate(
      ({ channelId, currentPubkey, senderPubkey, rootId, oldId }) => {
        const win = window as MockWindow;
        const emit = win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        const replace = win.__BUZZ_E2E_REPLACE_MOCK_FEED_ITEM__;
        if (!emit || !replace) throw new Error("Bridge helpers not ready");
        const passive = emit({
          channelName: "general",
          content: "Passive live sibling after fetch settled.",
          parentEventId: rootId,
          pubkey: senderPubkey,
          mentionPubkeys: [currentPubkey],
          id: "de".repeat(32),
        });
        replace(oldId, {
          id: passive.id,
          kind: passive.kind,
          pubkey: passive.pubkey,
          content: passive.content,
          created_at: passive.created_at + 12,
          channel_id: channelId,
          channel_name: "general",
          tags: passive.tags,
          category: "mention",
        });
      },
      {
        channelId: GENERAL_CHANNEL_ID,
        currentPubkey: TEST_IDENTITIES.tyler.pubkey,
        senderPubkey: TEST_IDENTITIES.alice.pubkey,
        rootId: fetchRoot.id,
        oldId: fetchNewest.id,
      },
    );
    await expect(
      page.getByTestId(`home-inbox-item-${"de".repeat(32)}`),
    ).toBeVisible();
    // Count must remain 1 — passive arrival must not trigger another center.
    expect(await getScrollIntoViewCount(page)).toBe(1);
  });
});
