import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SELF_PUBKEY = "deadbeef".repeat(8);
const CHANNEL_GENERAL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const AGENT_PUBKEY = TEST_IDENTITIES.charlie.pubkey;

type MockMessageEvent = {
  id: string;
  created_at: number;
  pubkey: string;
};

type MockInboxFeedItem = {
  content: string;
  id: string;
  tags: string[][];
};

async function waitForMockLiveSubscription(page: Page, channelName: string) {
  await expect
    .poll(() =>
      page.evaluate(
        (name) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: name,
          }) ?? false,
        channelName,
      ),
    )
    .toBe(true);
}

async function emitMockMessage(
  page: Page,
  content: string,
  options: {
    parentEventId?: string;
    pubkey: string;
    createdAt: number;
    mentionPubkeys?: string[];
  },
): Promise<MockMessageEvent> {
  const event = await page.evaluate(
    ({ body, parentEventId, pubkey, createdAt, mentionPubkeys }) =>
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            parentEventId?: string;
            pubkey: string;
            createdAt: number;
            mentionPubkeys?: string[];
          }) => MockMessageEvent;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: body,
        parentEventId,
        pubkey,
        createdAt,
        mentionPubkeys,
      }),
    {
      body: content,
      parentEventId: options.parentEventId,
      pubkey: options.pubkey,
      createdAt: options.createdAt,
      mentionPubkeys: options.mentionPubkeys,
    },
  );
  if (!event) {
    throw new Error("Mock message emitter is unavailable");
  }
  return event;
}

async function pushMockInboxFeedItems(page: Page, items: MockInboxFeedItem[]) {
  await page.waitForFunction(
    () =>
      typeof (window as Window & { __BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?: unknown })
        .__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ === "function",
  );
  await page.evaluate(
    ({ channelId, feedItems, senderPubkey }) => {
      const pushFeedItem = (
        window as Window & {
          __BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?: (item: {
            category: "mention";
            channel_id: string;
            channel_name: string;
            channel_type: "stream";
            content: string;
            created_at: number;
            id: string;
            kind: number;
            pubkey: string;
            tags: string[][];
          }) => void;
        }
      ).__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
      if (!pushFeedItem) {
        throw new Error("Mock feed injection helper is unavailable");
      }
      const createdAt = Math.floor(Date.now() / 1_000) - 300;
      for (const item of feedItems) {
        pushFeedItem({
          category: "mention",
          channel_id: channelId,
          channel_name: "general",
          channel_type: "stream",
          content: item.content,
          created_at: createdAt,
          id: item.id,
          kind: 9,
          pubkey: senderPubkey,
          tags: item.tags,
        });
      }
    },
    {
      channelId: CHANNEL_GENERAL,
      feedItems: items,
      senderPubkey: TEST_IDENTITIES.alice.pubkey,
    },
  );
}

async function getForcedUnreadSources(page: Page): Promise<string[]> {
  return page.evaluate(
    ({ channelId, pubkey }) => {
      const raw = window.localStorage.getItem(
        `buzz-forced-unread.v1:${pubkey}`,
      );
      if (!raw) return [];
      const entry = JSON.parse(raw)?.[channelId];
      if (entry === undefined) return [];
      return typeof entry === "object" && entry !== null
        ? (entry.sources ?? [])
        : ["manual"];
    },
    { channelId: CHANNEL_GENERAL, pubkey: SELF_PUBKEY },
  );
}

async function seedChannelActivity(
  page: Page,
  {
    extraThreadCount = 0,
    includeAgent = true,
  }: { extraThreadCount?: number; includeAgent?: boolean } = {},
) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  const ownRoot = await emitMockMessage(
    page,
    "How should the handoff work for longer agent tasks?",
    {
      pubkey: SELF_PUBKEY,
      createdAt: Math.floor(Date.now() / 1000) - 20,
    },
  );
  const extraRoots = await Promise.all(
    Array.from({ length: extraThreadCount }, (_, index) =>
      emitMockMessage(page, `Overflow preview thread ${index + 1}`, {
        pubkey: SELF_PUBKEY,
        createdAt: Math.floor(Date.now() / 1000) - 19 + index,
      }),
    ),
  );

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");

  const unreadAt = Math.floor(Date.now() / 1000) + 60;
  await emitMockMessage(
    page,
    "I tightened the empty state and added the direct thread link.",
    {
      parentEventId: "mock-general-welcome",
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: unreadAt,
      mentionPubkeys: [SELF_PUBKEY],
    },
  );
  await emitMockMessage(
    page,
    "The updated interaction keeps context visible without opening the channel first.",
    {
      parentEventId: ownRoot.id,
      pubkey: TEST_IDENTITIES.bob.pubkey,
      createdAt: unreadAt + 1,
    },
  );
  await Promise.all(
    extraRoots.map((root, index) =>
      emitMockMessage(
        page,
        `A new reply in overflow preview thread ${index + 1}`,
        {
          parentEventId: root.id,
          pubkey: TEST_IDENTITIES.alice.pubkey,
          createdAt: unreadAt + 2 + index,
        },
      ),
    ),
  );

  if (includeAgent) {
    await page.waitForFunction(
      () =>
        typeof (window as Window & { __BUZZ_E2E_SEED_ACTIVE_TURNS__?: unknown })
          .__BUZZ_E2E_SEED_ACTIVE_TURNS__ === "function",
    );
    await page.evaluate(
      ({ agentPubkey, channelId }) => {
        (
          window as Window & {
            __BUZZ_E2E_SEED_ACTIVE_TURNS__?: (input: {
              agentPubkey: string;
              channelId: string;
              turnId: string;
            }) => void;
          }
        ).__BUZZ_E2E_SEED_ACTIVE_TURNS__?.({
          agentPubkey,
          channelId,
          turnId: "channel-hover-preview",
        });
      },
      { agentPubkey: AGENT_PUBKEY, channelId: CHANNEL_GENERAL },
    );
  }

  await expect(page.getByTestId("channel-unread-dot-general")).toBeVisible();
  if (includeAgent) {
    await expect(page.getByTestId("channel-working-general")).toBeVisible();
  }
}

async function openActivityPopover(page: Page) {
  await page.getByTestId("channel-general").hover();
  const popover = page.getByTestId("channel-activity-popover-general");
  await expect(popover).toBeVisible();
  return popover;
}

test.describe("channel activity hover preview", () => {
  test.beforeEach(async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: "Charlie",
          status: "running",
          channelNames: ["general"],
        },
      ],
    });
  });

  test("shows unread channel activity and working agents, then opens the selected thread", async ({
    page,
  }) => {
    await seedChannelActivity(page, { extraThreadCount: 5 });
    const popover = await openActivityPopover(page);

    await waitForAnimations(page);
    const enterScale = await popover.evaluate((element) =>
      getComputedStyle(element).getPropertyValue("--tw-enter-scale"),
    );
    expect(enterScale).toBe("1");

    const heading = popover.getByRole("heading", {
      name: "Channel activity",
    });
    await expect(heading).toBeVisible();
    await expect(heading).toHaveCSS("backdrop-filter", /blur/);
    const activityScroll = popover.getByTestId("channel-activity-scroll");
    await expect(activityScroll).toHaveCSS("overflow-y", "auto");
    await expect(activityScroll.getByRole("heading")).toHaveCount(0);
    await expect
      .poll(() =>
        activityScroll.evaluate(
          (element) => element.scrollHeight > element.clientHeight,
        ),
      )
      .toBe(true);
    await waitForAnimations(page);
    const headingY = (await heading.boundingBox())?.y;
    const scrollY = (await activityScroll.boundingBox())?.y;
    const headingBottom = await heading.evaluate(
      (element) => element.getBoundingClientRect().bottom,
    );
    expect(scrollY).toBeGreaterThanOrEqual(headingBottom - 1);
    await activityScroll.evaluate((element) => {
      element.scrollTop = element.scrollHeight;
    });
    await expect
      .poll(() => activityScroll.evaluate((element) => element.scrollTop))
      .toBeGreaterThan(0);
    const scrolledHeadingY = (await heading.boundingBox())?.y;
    expect(headingY).toBeDefined();
    expect(scrolledHeadingY).toBeDefined();
    expect(Math.abs((scrolledHeadingY ?? 0) - (headingY ?? 0))).toBeLessThan(
      0.5,
    );
    await activityScroll.evaluate((element) => {
      element.scrollTop = 0;
    });
    await expect
      .poll(() =>
        activityScroll.evaluate(
          (element) =>
            getComputedStyle(element, "::-webkit-scrollbar-thumb")
              .backgroundColor,
        ),
      )
      .toMatch(/0\.15\)/);
    await expect(popover.getByTestId(/^channel-activity-item-/)).toHaveCount(7);
    await expect(popover).toContainText(
      "I tightened the empty state and added the direct thread link.",
    );
    await expect(popover.getByRole("heading")).toHaveCount(1);
    await expect(
      popover.getByTestId(`channel-activity-agent-${AGENT_PUBKEY}`),
    ).toContainText("Charlie");
    await expect(
      popover.getByTestId(`channel-activity-agent-${AGENT_PUBKEY}`),
    ).toContainText("Working");
    const orderedRows = popover.locator(
      '[data-testid^="channel-activity-agent-"], [data-testid^="channel-activity-item-"]',
    );
    await expect(orderedRows.first()).toHaveAttribute(
      "data-testid",
      `channel-activity-agent-${AGENT_PUBKEY}`,
    );

    await waitForAnimations(page);
    await page.screenshot({
      clip: { x: 0, y: 80, width: 720, height: 640 },
      path: "test-results/channel-activity-hover/channel-activity-popover.png",
    });

    const targetRow = popover
      .getByTestId(/^channel-activity-item-/)
      .filter({ hasText: "direct thread link" });
    await targetRow.getByRole("button", { name: /Open thread/ }).click();

    await expect(page.getByTestId("chat-title")).toHaveText("general");
    const threadPanel = page.getByTestId("message-thread-panel");
    await expect(threadPanel).toBeVisible();
    await expect(threadPanel).toContainText("direct thread link");
  });

  test("removes the dot and preview after the final activity is read", async ({
    page,
  }) => {
    await seedChannelActivity(page, { includeAgent: false });
    const popover = await openActivityPopover(page);
    await expect(popover.getByTestId(/^channel-activity-item-/)).toHaveCount(2);

    for (let remaining = 1; remaining >= 0; remaining -= 1) {
      const row = popover.getByTestId(/^channel-activity-item-/).first();
      await row.hover();
      await row.getByRole("button", { name: "Mark as read" }).click();
      await expect(popover.getByTestId(/^channel-activity-item-/)).toHaveCount(
        remaining,
      );
    }

    await expect(page.getByTestId("channel-unread-dot-general")).toHaveCount(0);
    await expect(
      page.getByTestId("channel-activity-popover-general"),
    ).toHaveCount(0);
  });

  test("groups multiple unread replies against the previewed channel", async ({
    page,
  }) => {
    await seedChannelActivity(page, { includeAgent: false });
    await emitMockMessage(
      page,
      "A second unread reply should stay grouped with this thread.",
      {
        parentEventId: "mock-general-welcome",
        pubkey: TEST_IDENTITIES.bob.pubkey,
        createdAt: Math.floor(Date.now() / 1_000) + 120,
        mentionPubkeys: [SELF_PUBKEY],
      },
    );

    const popover = await openActivityPopover(page);
    const groupedRow = popover
      .getByTestId(/^channel-activity-item-/)
      .filter({ hasText: "direct thread link" });
    await expect(groupedRow).toContainText("2 unread");
  });

  test("marks projected thread activity read from the active channel menu", async ({
    page,
  }) => {
    await seedChannelActivity(page, { includeAgent: false });
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await expect(page.getByTestId("channel-unread-dot-general")).toBeVisible();

    await page.getByTestId("channel-general").click({ button: "right" });
    await expect(
      page.getByRole("menuitem", { name: "Mark as read" }),
    ).toBeVisible();
    await expect(
      page.getByRole("menuitem", { name: "Mark unread" }),
    ).toHaveCount(0);
    await page.getByRole("menuitem", { name: "Mark as read" }).click();

    await expect(page.getByTestId("channel-unread-dot-general")).toHaveCount(0);
  });

  test("marks projected thread activity read in the active channel with Shift+Escape", async ({
    page,
  }) => {
    await seedChannelActivity(page, { includeAgent: false });
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await expect(page.getByTestId("channel-unread-dot-general")).toBeVisible();

    const shortcutHandled = await page.evaluate(() => {
      const event = new KeyboardEvent("keydown", {
        bubbles: true,
        cancelable: true,
        key: "Escape",
        shiftKey: true,
      });
      window.dispatchEvent(event);
      return event.defaultPrevented;
    });
    expect(shortcutHandled).toBe(true);

    await expect(page.getByTestId("channel-unread-dot-general")).toHaveCount(0);
    await page.getByTestId("channel-general").hover();
    await expect(
      page.getByTestId("channel-activity-popover-general"),
    ).toHaveCount(0);
  });

  test("supports row actions and opens an agent's scoped activity", async ({
    page,
  }) => {
    await seedChannelActivity(page);
    let popover = await openActivityPopover(page);
    const initialRows = popover.getByTestId(/^channel-activity-item-/);
    await expect(initialRows).toHaveCount(2);

    const firstRow = initialRows.first();
    await firstRow.hover();
    await firstRow.getByRole("button", { name: "Mark as read" }).click();
    await expect(popover.getByTestId(/^channel-activity-item-/)).toHaveCount(1);

    const remainingRow = popover.getByTestId(/^channel-activity-item-/).first();
    await remainingRow.hover();
    await remainingRow.getByRole("button", { name: "Remind me later" }).click();
    await expect(
      page.getByRole("dialog").getByText("Remind me later"),
    ).toBeVisible();
    await page.getByRole("button", { name: "Cancel" }).click();

    popover = await openActivityPopover(page);
    await popover.getByTestId(`channel-activity-agent-${AGENT_PUBKEY}`).click();

    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await expect(page.getByTestId("agent-session-agent-name")).toContainText(
      "Charlie",
    );
    await expect(page.getByTestId("agent-session-scope-label")).toContainText(
      "#general",
    );
  });

  test("keeps multiple Inbox threads visible after opening their channel", async ({
    page,
  }) => {
    const inboxItemIds = [
      "older-inbox-thread-for-hover",
      "second-inbox-thread-for-hover",
    ];
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await page.getByRole("button", { name: "Inbox", exact: true }).click();
    await expect(page.getByTestId("home-inbox-list")).toBeVisible();
    await pushMockInboxFeedItems(
      page,
      inboxItemIds.map((id, index) => ({
        content:
          index === 0
            ? "Older Inbox thread reopened for hover testing."
            : "A second Inbox thread reopened for hover testing.",
        id,
        tags: [
          ["h", CHANNEL_GENERAL],
          [
            "e",
            index === 0
              ? "older-inbox-thread-root"
              : "second-inbox-thread-root",
            "",
            "root",
          ],
          [
            "e",
            index === 0
              ? "older-inbox-thread-parent"
              : "second-inbox-thread-parent",
            "",
            "reply",
          ],
          ["p", TEST_IDENTITIES.tyler.pubkey],
        ],
      })),
    );

    for (const itemId of inboxItemIds) {
      const inboxRow = page.getByTestId(`home-inbox-item-${itemId}`);
      await expect(inboxRow).toBeVisible();
      await inboxRow.hover();
      await inboxRow.getByRole("button", { name: "Mark as read" }).click();
      await inboxRow.hover();
      await inboxRow.getByRole("button", { name: "Mark unread" }).click();
    }

    await expect(page.getByTestId("channel-general")).toHaveCSS(
      "font-weight",
      "700",
    );
    for (const [index, itemId] of inboxItemIds.entries()) {
      const inboxRow = page.getByTestId(`home-inbox-item-${itemId}`);
      await inboxRow.hover();
      await inboxRow.getByRole("button", { name: "Mark as read" }).click();
      if (index === 0) {
        await expect(page.getByTestId("channel-general")).toHaveCSS(
          "font-weight",
          "700",
        );
      }
    }
    await expect(page.getByTestId("channel-general")).not.toHaveCSS(
      "font-weight",
      "700",
    );
    for (const itemId of inboxItemIds) {
      const inboxRow = page.getByTestId(`home-inbox-item-${itemId}`);
      await inboxRow.hover();
      await inboxRow.getByRole("button", { name: "Mark unread" }).click();
    }

    await expect(page.getByTestId("channel-general")).toHaveCSS(
      "font-weight",
      "700",
    );
    await expect(page.getByTestId("channel-unread-dot-general")).toBeVisible();
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await expect(page.getByTestId("channel-unread-dot-general")).toBeVisible();
    await page.mouse.move(900, 680);
    let popover = await openActivityPopover(page);
    await expect(popover.getByTestId(/^channel-activity-item-/)).toHaveCount(2);
    await expect(popover).toContainText("Older Inbox thread reopened");
    await expect(popover).toContainText("A second Inbox thread reopened");
    await expect(popover.getByText("Thread", { exact: true })).toHaveCount(2);

    await popover
      .getByTestId(/^channel-activity-item-/)
      .filter({ hasText: "Older Inbox thread reopened" })
      .getByRole("button", { name: /Open thread/ })
      .click();
    await expect(popover).toBeHidden();
    await page.mouse.move(900, 680);
    popover = await openActivityPopover(page);
    await expect(popover.getByTestId(/^channel-activity-item-/)).toHaveCount(1);
    await expect(popover).not.toContainText("Older Inbox thread reopened");
    await expect(page.getByTestId("channel-unread-dot-general")).toBeVisible();
    await expect(page.getByTestId("channel-general")).toHaveCSS(
      "font-weight",
      "400",
    );

    await page.mouse.move(900, 680);
    await expect(popover).toBeHidden();
    await page.getByRole("button", { name: "Inbox", exact: true }).click();
    const topLevelItemId = "top-level-inbox-item-for-channel-read";
    await pushMockInboxFeedItems(page, [
      {
        content: "Top-level Inbox item reopened for channel read testing.",
        id: topLevelItemId,
        tags: [
          ["h", CHANNEL_GENERAL],
          ["p", TEST_IDENTITIES.tyler.pubkey],
        ],
      },
    ]);
    const topLevelInboxRow = page.getByTestId(
      `home-inbox-item-${topLevelItemId}`,
    );
    await expect(topLevelInboxRow).toBeVisible();
    await topLevelInboxRow.hover();
    await topLevelInboxRow.getByRole("button", { name: "Mark unread" }).click();
    await page.getByTestId("channel-general").click({ button: "right" });
    await page.getByRole("menuitem", { name: "Mark as read" }).click();
    await topLevelInboxRow.hover();
    await expect(
      topLevelInboxRow.getByRole("button", { name: "Mark unread" }),
    ).toBeVisible();
    await expect(page.getByTestId("channel-unread-dot-general")).toHaveCount(0);
    await page.getByTestId("channel-general").hover();
    await expect(
      page.getByTestId("channel-activity-popover-general"),
    ).toHaveCount(0);
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");
    await expect(page.getByTestId("channel-general")).not.toHaveCSS(
      "font-weight",
      "700",
    );
  });

  test("reading a grouped Inbox thread preserves an unrelated manual unread", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    const manualUnreadMessage = await emitMockMessage(
      page,
      "Keep this separate timeline message unread.",
      {
        pubkey: TEST_IDENTITIES.bob.pubkey,
        createdAt: Math.floor(Date.now() / 1_000),
      },
    );
    const manualUnreadRow = page
      .getByTestId("message-row")
      .filter({ hasText: "Keep this separate timeline message unread." });
    await manualUnreadRow.hover();
    await page.getByTestId(`more-actions-${manualUnreadMessage.id}`).click();
    await page
      .getByTestId(`mark-read-toggle-${manualUnreadMessage.id}`)
      .click();

    await page.getByRole("button", { name: "Inbox", exact: true }).click();
    const groupedRootId = "grouped-inbox-root-preserve-manual";
    const groupedReplyId = "grouped-inbox-reply-preserve-manual";
    await pushMockInboxFeedItems(page, [
      {
        content: "Grouped Inbox root for manual unread preservation.",
        id: groupedRootId,
        tags: [
          ["h", CHANNEL_GENERAL],
          ["p", TEST_IDENTITIES.tyler.pubkey],
        ],
      },
      {
        content: "Grouped Inbox reply marked read independently.",
        id: groupedReplyId,
        tags: [
          ["h", CHANNEL_GENERAL],
          ["e", groupedRootId, "", "root"],
          ["e", groupedRootId, "", "reply"],
          ["p", TEST_IDENTITIES.tyler.pubkey],
        ],
      },
    ]);
    const groupedInboxRow = page.getByTestId(
      `home-inbox-item-${groupedReplyId}`,
    );
    await expect(groupedInboxRow).toBeVisible();
    await groupedInboxRow.hover();
    await groupedInboxRow.getByRole("button", { name: "Mark as read" }).click();

    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("channel-general")).toHaveCSS(
      "font-weight",
      "700",
    );
  });

  test("preserves Inbox ownership while another top-level row remains unread", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await page.getByRole("button", { name: "Inbox", exact: true }).click();
    const topLevelItemIds = [
      "first-top-level-inbox-owner",
      "second-top-level-inbox-owner",
    ];
    await pushMockInboxFeedItems(
      page,
      topLevelItemIds.map((id, index) => ({
        content: `Top-level Inbox owner ${index + 1}.`,
        id,
        tags: [
          ["h", CHANNEL_GENERAL],
          ["p", TEST_IDENTITIES.tyler.pubkey],
        ],
      })),
    );
    for (const itemId of topLevelItemIds) {
      const inboxRow = page.getByTestId(`home-inbox-item-${itemId}`);
      await expect(inboxRow).toBeVisible();
      await inboxRow.hover();
      await inboxRow.getByRole("button", { name: "Mark unread" }).click();
    }
    await expect(page.getByTestId("channel-general")).toHaveCSS(
      "font-weight",
      "700",
    );
    await expect.poll(() => getForcedUnreadSources(page)).toEqual(["inbox"]);

    await page.getByRole("button", { name: "Inbox", exact: true }).click();
    for (const [index, itemId] of topLevelItemIds.entries()) {
      const inboxRow = page.getByTestId(`home-inbox-item-${itemId}`);
      await inboxRow.hover();
      await inboxRow.getByRole("button", { name: "Mark as read" }).click();
      if (index === 0) {
        await expect(page.getByTestId("channel-general")).toHaveCSS(
          "font-weight",
          "700",
        );
        await expect
          .poll(() => getForcedUnreadSources(page))
          .toEqual(["inbox"]);
      }
    }
    await expect.poll(() => getForcedUnreadSources(page)).toEqual([]);
  });

  test("surfaces future replies after the user reacts to a thread root", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    const root = await emitMockMessage(
      page,
      "Reacting here means I care about future replies.",
      {
        pubkey: TEST_IDENTITIES.alice.pubkey,
        createdAt: Math.floor(Date.now() / 1_000) - 20,
      },
    );
    const rootRow = page
      .getByTestId("message-row")
      .filter({ hasText: "Reacting here means I care" });
    await expect(rootRow).toBeVisible();
    await rootRow.hover();
    const actionBar = page.getByTestId(`message-action-bar-${root.id}`);
    await expect(actionBar).toBeVisible();
    await actionBar
      .getByRole("button", { name: /^React with / })
      .first()
      .click();
    await expect(
      rootRow.getByRole("button", { name: /^Toggle .* reaction$/ }),
    ).toBeVisible();

    await page.getByTestId("channel-random").click();
    await emitMockMessage(
      page,
      "This follow-up appears because the root was reacted to.",
      {
        parentEventId: root.id,
        pubkey: TEST_IDENTITIES.bob.pubkey,
        createdAt: Math.floor(Date.now() / 1_000) + 60,
      },
    );

    await expect(page.getByTestId("channel-unread-dot-general")).toBeVisible();
    const popover = await openActivityPopover(page);
    await expect(popover).toContainText(
      "This follow-up appears because the root was reacted to.",
    );
  });
});
