import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const CHANNEL = "general";
const SELF_PUBKEY = "deadbeef".repeat(8);
const STORAGE_KEY = `buzz-thread-follows.v1:${SELF_PUBKEY}`;
const SCREENSHOTS = "test-results/thread-follow";

type MockMessage = { id: string; created_at: number; pubkey: string };

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
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

async function emitMessage(
  page: import("@playwright/test").Page,
  input: {
    channelName?: string;
    content: string;
    parentEventId?: string;
    createdAt?: number;
    extraTags?: string[][];
    id?: string;
    pending?: boolean;
    pubkey?: string;
  },
): Promise<MockMessage> {
  const message = await page.evaluate(
    (payload) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: payload.channelName,
        content: payload.content,
        parentEventId: payload.parentEventId,
        createdAt: payload.createdAt,
        extraTags: payload.extraTags,
        id: payload.id,
        pending: payload.pending,
        pubkey: payload.pubkey,
      }),
    {
      ...input,
      channelName: input.channelName ?? CHANNEL,
      pubkey: input.pubkey ?? TEST_IDENTITIES.alice.pubkey,
    },
  );
  if (!message) throw new Error("mock message emitter is unavailable");
  return message;
}

async function expectNoThreadFollowControls(
  page: import("@playwright/test").Page,
  messageId: string,
) {
  await openMessageMenu(page, messageId);
  await expect(
    page.getByRole("menuitem", { name: /^(Unfollow|Follow) thread$/ }),
  ).toHaveCount(0);
  await page.keyboard.press("Escape");
}

async function openMessageMenu(
  page: import("@playwright/test").Page,
  messageId: string,
) {
  const row = page.locator(
    `[data-testid="message-row"][data-message-id="${messageId}"]`,
  );
  await row.hover();
  await row.getByTestId(`more-actions-${messageId}`).click();
  await expect(page.getByRole("menu")).toBeVisible();
}

async function storedFollowIds(page: import("@playwright/test").Page) {
  return page.evaluate((key) => {
    const entries = JSON.parse(localStorage.getItem(key) ?? "[]") as Array<{
      rootId: string;
    }>;
    return entries.map((entry) => entry.rootId);
  }, STORAGE_KEY);
}

async function captureMessageMenu(
  page: import("@playwright/test").Page,
  messageId: string,
  path: string,
) {
  await waitForAnimations(page);
  const rowBox = await page
    .locator(`[data-testid="message-row"][data-message-id="${messageId}"]`)
    .boundingBox();
  const menuBox = await page.getByRole("menu").boundingBox();
  const viewport = page.viewportSize();
  if (!rowBox || !menuBox || !viewport) {
    throw new Error("message menu screenshot bounds are unavailable");
  }
  const padding = 16;
  const x = Math.max(0, Math.min(rowBox.x, menuBox.x) - padding);
  const y = Math.max(0, Math.min(rowBox.y, menuBox.y) - padding);
  const right = Math.min(
    viewport.width,
    Math.max(rowBox.x + rowBox.width, menuBox.x + menuBox.width) + padding,
  );
  const bottom = Math.min(
    viewport.height,
    Math.max(rowBox.y + rowBox.height, menuBox.y + menuBox.height) + padding,
  );
  await page.screenshot({
    path,
    clip: { x, y, width: right - x, height: bottom - y },
  });
}

async function notificationBodies(page: import("@playwright/test").Page) {
  return page.evaluate(() =>
    (window.__BUZZ_E2E_NOTIFICATIONS__ ?? []).map((entry) => entry.body),
  );
}

async function waitForMessageProcessing(
  page: import("@playwright/test").Page,
  messageId: string,
) {
  await expect
    .poll(() =>
      page.evaluate((id) => {
        const queryClient = window.__BUZZ_E2E_QUERY_CLIENT__ as unknown as {
          getQueriesData: (filter: unknown) => Array<[unknown, unknown]>;
        };
        return queryClient
          .getQueriesData({ queryKey: [] })
          .some(([, data]) => (JSON.stringify(data) ?? "").includes(id));
      }, messageId),
    )
    .toBe(true);
}

test("a delivered message can be followed before its first reply and unfollowed", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId(`channel-${CHANNEL}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(CHANNEL);
  await waitForMockLiveSubscription(page, CHANNEL);

  const rootId = "mock-general-alice";
  await expect(
    page.locator(`[data-testid="message-row"][data-message-id="${rootId}"]`),
  ).toContainText("Hey team — checking in.");

  await openMessageMenu(page, rootId);
  const followItem = page.getByRole("menuitem", { name: "Follow thread" });
  await expect(followItem).toBeVisible();
  await captureMessageMenu(page, rootId, `${SCREENSHOTS}/follow-thread.png`);
  await followItem.focus();
  await page.keyboard.press("Enter");
  await expect.poll(() => storedFollowIds(page)).toEqual([rootId]);

  await page.reload();
  await page.getByTestId(`channel-${CHANNEL}`).click();
  await waitForMockLiveSubscription(page, CHANNEL);
  await expect.poll(() => storedFollowIds(page)).toEqual([rootId]);
  await openMessageMenu(page, rootId);
  const unfollowItem = page.getByRole("menuitem", {
    name: "Unfollow thread",
  });
  await expect(unfollowItem).toBeVisible();
  await captureMessageMenu(page, rootId, `${SCREENSHOTS}/unfollow-thread.png`);
  await page.keyboard.press("Escape");

  await page.getByTestId("channel-random").click();
  await emitMessage(page, {
    content: "First reply after follow",
    parentEventId: rootId,
    createdAt: Math.floor(Date.now() / 1000) + 60,
  });
  await expect
    .poll(() => notificationBodies(page))
    .toContain("First reply after follow");
  await expect(page.getByTestId(`channel-unread-dot-${CHANNEL}`)).toBeVisible();

  await page.getByTestId(`channel-${CHANNEL}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(CHANNEL);
  await openMessageMenu(page, rootId);
  await unfollowItem.focus();
  await page.keyboard.press("Enter");
  await expect.poll(() => storedFollowIds(page)).toEqual([]);

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId(`channel-unread-dot-${CHANNEL}`)).toHaveCount(
    0,
  );
  const notificationCount = (await notificationBodies(page)).length;
  const mutedReply = await emitMessage(page, {
    content: "Later ordinary reply after unfollow",
    parentEventId: rootId,
    createdAt: Math.floor(Date.now() / 1000) + 120,
  });
  await waitForMessageProcessing(page, mutedReply.id);
  await expect(page.getByTestId(`channel-unread-dot-${CHANNEL}`)).toHaveCount(
    0,
  );

  await emitMessage(page, {
    content: "Control reply after muted reply",
    parentEventId: "mock-general-welcome",
    createdAt: Math.floor(Date.now() / 1000) + 180,
  });
  await expect
    .poll(() => notificationBodies(page))
    .toContain("Control reply after muted reply");
  const finalNotificationBodies = await notificationBodies(page);
  expect(finalNotificationBodies).toHaveLength(notificationCount + 1);
  expect(finalNotificationBodies).not.toContain(
    "Later ordinary reply after unfollow",
  );
});

test("following a broadcast reply persists its thread root", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId(`channel-${CHANNEL}`).click();
  await waitForMockLiveSubscription(page, CHANNEL);

  const pending = await emitMessage(page, {
    content: "Pending message has no durable thread id",
    pending: true,
  });
  await openMessageMenu(page, pending.id);
  await expect(
    page.getByRole("menuitem", { name: /^(Unfollow|Follow) thread$/ }),
  ).toHaveCount(0);
  await page.keyboard.press("Escape");

  const root = await emitMessage(page, { content: "Broadcast thread root" });
  const broadcastReply = await emitMessage(page, {
    content: "Broadcast reply row",
    parentEventId: root.id,
    extraTags: [["broadcast", "1"]],
  });

  await openMessageMenu(page, broadcastReply.id);
  const followItem = page.getByRole("menuitem", { name: "Follow thread" });
  await followItem.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect.poll(() => storedFollowIds(page)).toEqual([root.id]);
  expect(await storedFollowIds(page)).not.toContain(broadcastReply.id);

  await emitMessage(page, {
    content: "Child of broadcast reply",
    parentEventId: broadcastReply.id,
  });
  await expect(
    page.locator(`[data-thread-head-id="${broadcastReply.id}"]`),
  ).toBeVisible();
  await openMessageMenu(page, broadcastReply.id);
  const unfollowItem = page.getByRole("menuitem", {
    name: "Unfollow thread",
  });
  await expect(unfollowItem).toBeVisible();
  await unfollowItem.focus();
  await page.keyboard.press("Enter");
  await expect.poll(() => storedFollowIds(page)).toEqual([]);
});

test("a delivered self-authored root immediately offers unfollow", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId(`channel-${CHANNEL}`).click();
  await waitForMockLiveSubscription(page, CHANNEL);

  const root = await emitMessage(page, {
    content: "Root written by the current user",
    pubkey: SELF_PUBKEY,
  });
  await waitForMessageProcessing(page, root.id);

  await openMessageMenu(page, root.id);
  await expect(
    page.getByRole("menuitem", { name: "Unfollow thread" }),
  ).toBeVisible();
  await expect(
    page.getByRole("menuitem", { name: "Follow thread", exact: true }),
  ).toHaveCount(0);
  await expect.poll(() => storedFollowIds(page)).toEqual([]);

  const unfollowItem = page.getByRole("menuitem", {
    name: "Unfollow thread",
  });
  await unfollowItem.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await openMessageMenu(page, root.id);
  await expect(
    page.getByRole("menuitem", { name: "Follow thread", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("menuitem", { name: "Unfollow thread" }),
  ).toHaveCount(0);
  await expect.poll(() => storedFollowIds(page)).toEqual([]);
});

test("a mention-only root offers follow and explicit follow enables ordinary reply notifications", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId(`channel-${CHANNEL}`).click();
  await waitForMockLiveSubscription(page, CHANNEL);

  const root = await emitMessage(page, {
    content: "Thread root before mention",
  });
  const mention = await emitMessage(page, {
    content: "Mention-only thread reply",
    parentEventId: root.id,
    extraTags: [["p", SELF_PUBKEY]],
  });
  await waitForMessageProcessing(page, mention.id);

  await openMessageMenu(page, root.id);
  const followItem = page.getByRole("menuitem", {
    name: "Follow thread",
    exact: true,
  });
  await expect(followItem).toBeVisible();
  await expect(
    page.getByRole("menuitem", { name: "Unfollow thread" }),
  ).toHaveCount(0);
  await followItem.click();
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect.poll(() => storedFollowIds(page)).toEqual([root.id]);

  await page.getByTestId("channel-random").click();
  await emitMessage(page, {
    content: "Ordinary reply after explicit follow",
    parentEventId: root.id,
    createdAt: Math.floor(Date.now() / 1000) + 60,
  });
  await expect
    .poll(() => notificationBodies(page))
    .toContain("Ordinary reply after explicit follow");
});

test("a plain DM message omits thread follow controls", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");
  await waitForMockLiveSubscription(page, "alice-tyler");

  const root = await emitMessage(page, {
    channelName: "alice-tyler",
    content: "Plain DM root",
    pubkey: SELF_PUBKEY,
  });
  await waitForMessageProcessing(page, root.id);
  await expectNoThreadFollowControls(page, root.id);
});

test("a DM thread summary omits thread follow controls", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");
  await waitForMockLiveSubscription(page, "alice-tyler");

  const root = await emitMessage(page, {
    channelName: "alice-tyler",
    content: "DM thread root",
  });
  const reply = await emitMessage(page, {
    channelName: "alice-tyler",
    content: "DM thread reply",
    parentEventId: root.id,
  });
  await waitForMessageProcessing(page, reply.id);
  await expect(
    page.locator(`[data-thread-head-id="${root.id}"]`),
  ).toBeVisible();
  await expectNoThreadFollowControls(page, root.id);
});
