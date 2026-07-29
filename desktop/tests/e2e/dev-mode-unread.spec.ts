import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

// Contextual unread in developer mode: unread threads and tabs bubble up to
// the parent's navigator row, and opening an unread parent routes directly
// to the tab — and thread side chat — that needs attention, instead of
// landing on the parent's main view.

// The pubkey the mock bridge logs in as (mirrors `e2eBridge`'s self identity).
const SELF_PUBKEY = "deadbeef".repeat(8);

// Unread replies must land strictly after any read frontier captured while
// the channel was open. A minute ahead ensures they do.
function unreadTimestamp() {
  return Math.floor(Date.now() / 1000) + 60;
}

async function openDevMode(page: import("@playwright/test").Page) {
  await installMockBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("dev-mode-composer").waitFor();
}

// ArrowUp steps through channel previews newest-first; walk until the
// target channel is previewed, then Enter opens it (with unread routing).
async function openChannelFromNavigator(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await page.getByTestId("dev-mode-composer").focus();
  const topBar = page.getByTestId("dev-mode-topbar-channel");
  for (let step = 0; step < 20; step += 1) {
    await page.keyboard.press("ArrowUp");
    const previewed = (await topBar.innerText()).replace(/^#\s*/, "").trim();
    if (previewed === channelName) break;
  }
  await expect(topBar).toContainText(channelName);
  await page.keyboard.press("Enter");
}

async function createChannel(
  page: import("@playwright/test").Page,
  name: string,
) {
  await page.evaluate(async (channelName) => {
    const w = window as Window & {
      __TAURI_INTERNALS__?: {
        invoke: (command: string, payload: unknown) => Promise<unknown>;
      };
      __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
    };
    await w.__TAURI_INTERNALS__?.invoke("create_channel", {
      name: channelName,
      channelType: "stream",
      visibility: "open",
    });
    await w.__BUZZ_E2E_INVALIDATE_CHANNELS__?.();
  }, name);
}

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(async () =>
      page.evaluate(
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
      ),
    )
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
): Promise<{ id: string }> {
  const event = await page.evaluate(
    ({ ch, msg, parentEventId, pubkey, ts, mentionPubkeys }) =>
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            parentEventId?: string;
            pubkey?: string;
            createdAt?: number;
            mentionPubkeys?: string[];
          }) => { id: string };
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: ch,
        content: msg,
        parentEventId: parentEventId ?? undefined,
        pubkey: pubkey ?? undefined,
        createdAt: ts,
        mentionPubkeys: mentionPubkeys ?? undefined,
      }),
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

// The navigator row for a main channel, for asserting its unread dot.
function navigatorRow(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  return page
    .getByTestId("dev-mode-channel-navigator")
    .locator("button", { hasText: `# ${channelName}` })
    .first();
}

test("unread thread reply in a tab routes the parent open to its side chat", async ({
  page,
}) => {
  await openDevMode(page);
  await openChannelFromNavigator(page, "general");
  await page.getByTestId("dev-mode-transcript").waitFor();
  await createChannel(page, "general--flaky-ci");

  // Visit the tab once: establishes the live subscription and a read
  // frontier strictly before the unread reply.
  const tabs = page.getByTestId("dev-mode-channel-tab");
  await expect(tabs).toHaveCount(2);
  await tabs.nth(1).click();
  const topBar = page.getByTestId("dev-mode-topbar-channel");
  await expect(topBar).toContainText("general--flaky-ci");
  await waitForMockLiveSubscription(page, "general--flaky-ci");

  const root = await emitMockMessage(
    page,
    "general--flaky-ci",
    "prompt: chase the flaky test",
    { pubkey: SELF_PUBKEY, createdAt: Math.floor(Date.now() / 1000) - 40 },
  );
  await expect(
    page.getByTestId("dev-mode-prompt-card").filter({
      hasText: "prompt: chase the flaky test",
    }),
  ).toBeVisible();

  // Back out to the fresh composer so nothing is being viewed.
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");

  // An external agent reply lands in the tab's thread while we are away.
  // The self-mention clears the notify gate exactly like an agent replying
  // to the user's prompt.
  await emitMockMessage(page, "general--flaky-ci", "agent: found the cause", {
    parentEventId: root.id,
    createdAt: unreadTimestamp(),
    mentionPubkeys: [SELF_PUBKEY],
  });

  // The thread unread bubbles to the parent's navigator row.
  await expect(
    navigatorRow(page, "general").getByTestId("dev-mode-unread-dot"),
  ).toBeVisible();

  // Opening the parent routes to the unread tab and opens the side chat on
  // the unread thread.
  await openChannelFromNavigator(page, "general");
  await expect(topBar).toContainText("general--flaky-ci");
  await expect(tabs.filter({ hasText: "flaky-ci" }).first()).toHaveAttribute(
    "data-active",
    "true",
  );
  const threadPanel = page.getByTestId("dev-mode-thread-panel");
  await expect(threadPanel).toBeVisible();
  await expect(threadPanel).toContainText("agent: found the cause");

  // Reading the thread clears the contextual indicators.
  await expect(
    navigatorRow(page, "general").getByTestId("dev-mode-unread-dot"),
  ).toHaveCount(0);
});

test("unread top-level post routes to its tab without opening a side chat", async ({
  page,
}) => {
  await openDevMode(page);
  await openChannelFromNavigator(page, "general");
  await page.getByTestId("dev-mode-transcript").waitFor();
  await createChannel(page, "general--rollback");

  const tabs = page.getByTestId("dev-mode-channel-tab");
  await expect(tabs).toHaveCount(2);
  await tabs.nth(1).click();
  const topBar = page.getByTestId("dev-mode-topbar-channel");
  await expect(topBar).toContainText("general--rollback");
  await waitForMockLiveSubscription(page, "general--rollback");

  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");

  await emitMockMessage(page, "general--rollback", "status: rollback done", {
    createdAt: unreadTimestamp(),
  });

  await expect(
    navigatorRow(page, "general").getByTestId("dev-mode-unread-dot"),
  ).toBeVisible();

  await openChannelFromNavigator(page, "general");
  await expect(topBar).toContainText("general--rollback");
  await expect(tabs.filter({ hasText: "rollback" }).first()).toHaveAttribute(
    "data-active",
    "true",
  );
  await expect(page.getByTestId("dev-mode-thread-panel")).toHaveCount(0);

  // Viewing the tab clears the channel-level unread — no threads need to
  // be opened.
  await expect(
    navigatorRow(page, "general").getByTestId("dev-mode-unread-dot"),
  ).toHaveCount(0);
});

test("a read channel opens exactly where asked, with no routing", async ({
  page,
}) => {
  await openDevMode(page);
  await openChannelFromNavigator(page, "general");
  await page.getByTestId("dev-mode-transcript").waitFor();
  await createChannel(page, "general--quiet");

  const tabs = page.getByTestId("dev-mode-channel-tab");
  await expect(tabs).toHaveCount(2);

  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");

  await openChannelFromNavigator(page, "general");
  const topBar = page.getByTestId("dev-mode-topbar-channel");
  await expect(topBar).toContainText("general");
  await expect(tabs.nth(0)).toHaveAttribute("data-active", "true");
  await expect(page.getByTestId("dev-mode-thread-panel")).toHaveCount(0);
});
