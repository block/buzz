import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const AGENTS_CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301";

type MockInboxWindow = Window & {
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
  }) => unknown;
  __BUZZ_E2E_SIGNED_EVENTS__?: Array<{
    content: string;
    kind: number;
    tags: string[][];
  }>;
};

async function seedUnreadInboxItems(page: import("@playwright/test").Page) {
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  await page.waitForFunction(
    () =>
      typeof (window as MockInboxWindow).__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ ===
      "function",
  );

  const ids = {
    first: "inbox-explicit-read-first",
    second: "inbox-explicit-read-second",
  };
  await page.evaluate(
    ({ agentsChannelId, currentPubkey, generalChannelId, itemIds, sender }) => {
      const win = window as MockInboxWindow;
      const push = win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
      if (!push) throw new Error("Mock feed helper is not installed.");

      const createdAt = Math.floor(Date.now() / 1_000) + 10_000;
      push({
        category: "mention",
        channel_id: generalChannelId,
        channel_name: "general",
        channel_type: "stream",
        content: "First unread item for explicit completion",
        created_at: createdAt + 1,
        id: itemIds.first,
        kind: 9,
        pubkey: sender,
        tags: [
          ["h", generalChannelId],
          ["p", currentPubkey],
        ],
      });
      push({
        category: "mention",
        channel_id: agentsChannelId,
        channel_name: "agents",
        channel_type: "stream",
        content: "Second unread item for explicit completion",
        created_at: createdAt,
        id: itemIds.second,
        kind: 9,
        pubkey: sender,
        tags: [
          ["h", agentsChannelId],
          ["p", currentPubkey],
        ],
      });
      if (win.__BUZZ_E2E_SIGNED_EVENTS__) {
        win.__BUZZ_E2E_SIGNED_EVENTS__.length = 0;
      }
    },
    {
      agentsChannelId: AGENTS_CHANNEL_ID,
      currentPubkey: TEST_IDENTITIES.tyler.pubkey,
      generalChannelId: GENERAL_CHANNEL_ID,
      itemIds: ids,
      sender: TEST_IDENTITIES.alice.pubkey,
    },
  );

  await expect(page.getByTestId(`home-inbox-item-${ids.first}`)).toBeVisible();
  await expect(page.getByTestId(`home-inbox-item-${ids.second}`)).toBeVisible();
  return ids;
}

async function setUnreadOnly(
  page: import("@playwright/test").Page,
  checked: boolean,
) {
  await page.getByTestId("inbox-options-trigger").click();
  const toggle = page.getByRole("switch", { name: "Show unread only" });
  if ((await toggle.isChecked()) !== checked) {
    await toggle.click();
  }
  await page.keyboard.press("Escape");
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("Inbox preview stays unread until the distinct explicit action completes it", async ({
  page,
}) => {
  const ids = await seedUnreadInboxItems(page);
  const first = page.getByTestId(`home-inbox-item-${ids.first}`);
  const second = page.getByTestId(`home-inbox-item-${ids.second}`);

  const firstPreview = first.getByRole("button", {
    name: "Open inbox item from alice",
  });
  await firstPreview.focus();
  await page.keyboard.press("Enter");

  await expect(first).toHaveAttribute("aria-current", "true");
  await expect(
    first.getByRole("button", { name: "Mark as read" }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as MockInboxWindow).__BUZZ_E2E_SIGNED_EVENTS__?.filter(
            (event) => event.kind === 30_078,
          ).length ?? 0,
      ),
    )
    .toBe(0);

  await setUnreadOnly(page, true);
  await expect(first).toBeVisible();
  await expect(second).toBeVisible();

  const secondPreview = second.getByRole("button", {
    name: "Open inbox item from alice",
  });
  await secondPreview.focus();
  await page.keyboard.press("Enter");
  await expect(second).toHaveAttribute("aria-current", "true");
  await expect(first).toBeVisible();
  await expect(second).toBeVisible();
  await expect(
    second.getByRole("button", { name: "Mark as read" }),
  ).toBeVisible();

  await firstPreview.focus();
  await page.keyboard.press("Enter");
  const markRead = first.getByRole("button", { name: "Mark as read" });
  await expect(markRead).toBeVisible();
  await markRead.click();

  await expect(first).toHaveCount(0);
  await expect(second).toHaveAttribute("aria-current", "true");
  await expect
    .poll(
      () =>
        page.evaluate(
          ({ channelId }) => {
            const events =
              (window as MockInboxWindow).__BUZZ_E2E_SIGNED_EVENTS__ ?? [];
            return events.some((event) => {
              if (event.kind !== 30_078) return false;
              const payload = JSON.parse(event.content) as {
                contexts?: Record<string, number>;
              };
              return (payload.contexts?.[channelId] ?? 0) > 0;
            });
          },
          { channelId: GENERAL_CHANNEL_ID },
        ),
      { timeout: 12_000 },
    )
    .toBe(true);

  await setUnreadOnly(page, false);
  await expect(first).toBeVisible();
  await first.hover();
  const markUnread = first.getByRole("button", { name: "Mark unread" });
  await expect(markUnread).toBeVisible();
  await markUnread.click();
  await expect(
    first.getByRole("button", { name: "Mark as read" }),
  ).toBeVisible();

  await setUnreadOnly(page, true);
  await expect(first).toBeVisible();
});
