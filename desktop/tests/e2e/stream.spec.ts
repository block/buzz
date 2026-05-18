import { expect, test, type Browser, type Page } from "@playwright/test";

import { installRelayBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { assertRelaySeeded } from "../helpers/seed";

const isCi = Boolean(process.env.CI);
const relayDeliveryTimeoutMs = isCi ? 15_000 : 5_000;
const relaySeedHookTimeoutMs = isCi ? 90_000 : 30_000;

async function expectTimelineToContain(page: Page, text: string) {
  await expect(page.getByTestId("message-timeline")).toContainText(text, {
    timeout: relayDeliveryTimeoutMs,
  });
}

async function getTimelineMetrics(page: Page) {
  return page.getByTestId("message-timeline").evaluate((element) => {
    const timeline = element as HTMLDivElement;

    return {
      clientHeight: timeline.clientHeight,
      scrollHeight: timeline.scrollHeight,
      scrollTop: timeline.scrollTop,
      distanceFromBottom:
        timeline.scrollHeight - timeline.clientHeight - timeline.scrollTop,
    };
  });
}

async function ensureTimelineScrollable(
  senderPage: Page,
  receiverPage: Page,
  prefix: string,
) {
  const input = senderPage.getByTestId("message-input");
  const sendButton = senderPage.getByTestId("send-message");

  for (let index = 0; index < 24; index += 1) {
    const metrics = await getTimelineMetrics(receiverPage);
    if (metrics.scrollHeight > metrics.clientHeight + 160) {
      return;
    }

    const message = `${prefix} seed ${index}`;

    await expect(input).toBeEnabled();
    await input.fill(message);
    await sendButton.click();
    await expectTimelineToContain(receiverPage, message);
  }

  const metrics = await getTimelineMetrics(receiverPage);
  expect(metrics.scrollHeight).toBeGreaterThan(metrics.clientHeight + 160);
}

async function createAndJoinSharedStream(
  ownerPage: Page,
  memberPage: Page,
  channelName: string,
) {
  await ownerPage.getByRole("button", { name: "Create a channel" }).click();
  await ownerPage.getByTestId("create-channel-name").fill(channelName);
  await ownerPage.getByTestId("create-channel-submit").click();
  await expect(ownerPage.getByTestId("stream-list")).toContainText(channelName);
  await expect(ownerPage.getByTestId("chat-title")).toHaveText(channelName);

  await memberPage.getByTestId("browse-channels").click();
  await expect(memberPage.getByTestId("channel-browser-dialog")).toBeVisible();
  await memberPage
    .getByTestId(`browse-channel-${channelName}`)
    .getByRole("button", { name: "Join" })
    .click();
  await expect(memberPage.getByTestId("chat-title")).toHaveText(channelName);
  await expect(memberPage.getByTestId("stream-list")).toContainText(
    channelName,
  );
}

async function sendChannelMessage(
  page: Page,
  {
    channelName,
    content,
    mentionPubkeys,
  }: {
    channelName: string;
    content: string;
    mentionPubkeys?: string[];
  },
) {
  await page.evaluate(
    async ({ channelName: targetChannelName, content, mentionPubkeys }) => {
      const tauriWindow = window as Window & {
        __TAURI_INTERNALS__?: {
          invoke: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<unknown>;
        };
      };

      const invoke = tauriWindow.__TAURI_INTERNALS__?.invoke;
      if (!invoke) {
        throw new Error("Tauri invoke bridge is unavailable.");
      }

      const channels = (await invoke("get_channels")) as Array<{
        id: string;
        name: string;
      }>;
      const channel = channels.find(({ name }) => name === targetChannelName);
      if (!channel) {
        throw new Error(`Channel not found: ${targetChannelName}`);
      }

      await invoke("send_channel_message", {
        channelId: channel.id,
        content,
        kind: null,
        mediaTags: null,
        mentionPubkeys: mentionPubkeys ?? null,
        parentEventId: null,
      });
    },
    { channelName, content, mentionPubkeys },
  );
}

async function scrollTimelineAwayFromBottom(page: Page, minDistance = 160) {
  const timeline = page.getByTestId("message-timeline");
  await timeline.hover();

  for (let attempt = 0; attempt < 8; attempt += 1) {
    await page.mouse.wheel(0, -800);
    const metrics = await getTimelineMetrics(page);
    if (metrics.distanceFromBottom > minDistance) {
      return;
    }
  }

  throw new Error("Failed to scroll the timeline away from the bottom.");
}

test.beforeAll(async () => {
  test.setTimeout(relaySeedHookTimeoutMs);
  await assertRelaySeeded();
});

test("loads channels from the relay", async ({ page }) => {
  await installRelayBridge(page, "tyler");
  await page.goto("/");

  await expect(page.getByTestId("stream-list")).toContainText("general");
  await expect(page.getByTestId("stream-list")).toContainText("random");
  await expect(page.getByTestId("forum-list")).toContainText("watercooler");
  await expect(page.getByTestId("dm-list")).toContainText("alice-tyler");
});

test("loads the home feed from the relay", async ({ browser }) => {
  const message = `Relay home mention ${Date.now()}`;
  const targetContext = await browser.newContext();
  const senderContext = await browser.newContext();
  const page = await targetContext.newPage();
  const senderPage = await senderContext.newPage();

  try {
    await installRelayBridge(page, "tyler");
    await installRelayBridge(senderPage, "alice");
    await page.goto("/");
    await senderPage.goto("/");

    await expect(page.getByTestId("chat-title")).toHaveText("Home");
    await expect(page.getByTestId("home-inbox")).toBeVisible();

    await sendChannelMessage(senderPage, {
      channelName: "general",
      content: message,
      mentionPubkeys: [TEST_IDENTITIES.tyler.pubkey],
    });

    await expect(page.getByTestId("home-inbox-list")).toContainText(message, {
      timeout: relayDeliveryTimeoutMs,
    });
    await expect(page.getByTestId("home-inbox-detail")).toBeVisible();
  } finally {
    await targetContext.close();
    await senderContext.close();
  }
});

test("shows sent inbox replies immediately in the inbox detail pane", async ({
  browser,
}) => {
  const message = `Relay inbox reply target ${Date.now()}`;
  const reply = `Inbox reply ${Date.now()}`;
  const targetContext = await browser.newContext();
  const senderContext = await browser.newContext();
  const page = await targetContext.newPage();
  const senderPage = await senderContext.newPage();

  try {
    await installRelayBridge(page, "tyler");
    await installRelayBridge(senderPage, "alice");
    await page.goto("/");
    await senderPage.goto("/");

    await sendChannelMessage(senderPage, {
      channelName: "general",
      content: message,
      mentionPubkeys: [TEST_IDENTITIES.tyler.pubkey],
    });

    await page.getByTestId("home-inbox-list").getByText(message).click({
      timeout: relayDeliveryTimeoutMs,
    });
    await expect(page.getByTestId("home-inbox-detail")).toBeVisible();
    await expect(page.getByTestId("message-input")).toBeEnabled();

    await page.getByTestId("message-input").fill(reply);
    await page.getByTestId("send-message").click();

    await expect(page.getByTestId("home-inbox-detail")).toContainText(reply);
  } finally {
    await targetContext.close();
    await senderContext.close();
  }
});

test("creates a relay-backed stream", async ({ page }) => {
  const channelName = `desktop-e2e-${Date.now()}`;

  await installRelayBridge(page, "tyler");
  await page.goto("/");
  await page.getByRole("button", { name: "Create a channel" }).click();
  await page.getByTestId("create-channel-name").fill(channelName);
  await page
    .getByTestId("create-channel-description")
    .fill("Created from Playwright");
  await page.getByTestId("create-channel-submit").click();

  await expect(page.getByTestId("stream-list")).toContainText(channelName);
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
});

test("sends a message through the real relay", async ({ page }) => {
  const message = `Integration message ${Date.now()}`;

  await installRelayBridge(page, "tyler");
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();

  await expectTimelineToContain(page, message);
});

test("delivers a message to a second browser context in real time", async ({
  browser,
}: {
  browser: Browser;
}) => {
  const channelName = `realtime-shared-${Date.now()}`;
  const contextOne = await browser.newContext();
  const contextTwo = await browser.newContext();
  const pageOne = await contextOne.newPage();
  const pageTwo = await contextTwo.newPage();
  const message = `Realtime message ${Date.now()}`;

  try {
    await installRelayBridge(pageOne, "tyler");
    await installRelayBridge(pageTwo, "alice");

    await pageOne.goto("/");
    await pageTwo.goto("/");
    await createAndJoinSharedStream(pageOne, pageTwo, channelName);

    await pageOne.getByTestId("message-input").fill(message);
    await pageOne.getByTestId("send-message").click();

    await expectTimelineToContain(pageTwo, message);
  } finally {
    await contextOne.close();
    await contextTwo.close();
  }
});

test("stays pinned to the latest message when new messages arrive at the bottom", async ({
  browser,
}: {
  browser: Browser;
}) => {
  test.slow();

  const channelName = `pinned-bottom-${Date.now()}`;
  const contextOne = await browser.newContext();
  const contextTwo = await browser.newContext();
  const pageOne = await contextOne.newPage();
  const pageTwo = await contextTwo.newPage();
  const prefix = `Pinned scroll ${Date.now()}`;
  const incomingMessage = `${prefix} incoming`;

  try {
    await installRelayBridge(pageOne, "tyler");
    await installRelayBridge(pageTwo, "alice");

    await pageOne.goto("/");
    await pageTwo.goto("/");
    await createAndJoinSharedStream(pageOne, pageTwo, channelName);

    await ensureTimelineScrollable(pageOne, pageTwo, prefix);
    await expect
      .poll(async () => (await getTimelineMetrics(pageTwo)).distanceFromBottom)
      .toBeLessThan(8);

    await pageOne.getByTestId("message-input").fill(incomingMessage);
    await pageOne.getByTestId("send-message").click();

    await expectTimelineToContain(pageTwo, incomingMessage);
    await expect
      .poll(async () => (await getTimelineMetrics(pageTwo)).distanceFromBottom)
      .toBeLessThan(8);
    await expect(pageTwo.getByTestId("message-scroll-to-latest")).toHaveCount(
      0,
    );
  } finally {
    await contextOne.close();
    await contextTwo.close();
  }
});

test("stays pinned after you send a message and a remote reply arrives right after", async ({
  browser,
}: {
  browser: Browser;
}) => {
  test.slow();

  const channelName = `reply-shared-${Date.now()}`;
  const contextOne = await browser.newContext();
  const contextTwo = await browser.newContext();
  const pageOne = await contextOne.newPage();
  const pageTwo = await contextTwo.newPage();
  const prefix = `Reply after send ${Date.now()}`;
  const localMessage = `${prefix} local`;
  const incomingMessage = `${prefix} incoming`;

  try {
    await installRelayBridge(pageOne, "tyler");
    await installRelayBridge(pageTwo, "alice");

    await pageOne.goto("/");
    await pageTwo.goto("/");
    await createAndJoinSharedStream(pageOne, pageTwo, channelName);

    await ensureTimelineScrollable(pageOne, pageTwo, prefix);
    await expect
      .poll(async () => (await getTimelineMetrics(pageTwo)).distanceFromBottom)
      .toBeLessThan(8);

    await pageTwo.getByTestId("message-input").fill(localMessage);
    await pageTwo.getByTestId("send-message").click();
    await expectTimelineToContain(pageTwo, localMessage);

    await pageOne.getByTestId("message-input").fill(incomingMessage);
    await pageOne.getByTestId("send-message").click();

    await expectTimelineToContain(pageTwo, incomingMessage);
    await expect
      .poll(async () => (await getTimelineMetrics(pageTwo)).distanceFromBottom)
      .toBeLessThan(8);
    await expect(pageTwo.getByTestId("message-scroll-to-latest")).toHaveCount(
      0,
    );
  } finally {
    await contextOne.close();
    await contextTwo.close();
  }
});

test("keeps bottom-pinned scrolling after the composer grows", async ({
  browser,
}: {
  browser: Browser;
}) => {
  test.slow();

  const channelName = `composer-shared-${Date.now()}`;
  const contextOne = await browser.newContext();
  const contextTwo = await browser.newContext();
  const pageOne = await contextOne.newPage();
  const pageTwo = await contextTwo.newPage();
  const prefix = `Composer pinned ${Date.now()}`;
  const incomingMessage = `${prefix} incoming`;
  const receiverInput = pageTwo.getByTestId("message-input");

  try {
    await installRelayBridge(pageOne, "tyler");
    await installRelayBridge(pageTwo, "alice");

    await pageOne.goto("/");
    await pageTwo.goto("/");
    await createAndJoinSharedStream(pageOne, pageTwo, channelName);

    await ensureTimelineScrollable(pageOne, pageTwo, prefix);
    await expect
      .poll(async () => (await getTimelineMetrics(pageTwo)).distanceFromBottom)
      .toBeLessThan(8);

    await receiverInput.fill("Composer pinned line one");
    await receiverInput.press("Enter");
    await receiverInput.type("Composer pinned line two");
    await receiverInput.press("Enter");
    await receiverInput.type("Composer pinned line three");
    await receiverInput.press("Enter");
    await receiverInput.type("Composer pinned line four");

    await expect
      .poll(async () => (await getTimelineMetrics(pageTwo)).distanceFromBottom)
      .toBeLessThan(8);

    await pageOne.getByTestId("message-input").fill(incomingMessage);
    await pageOne.getByTestId("send-message").click();

    await expectTimelineToContain(pageTwo, incomingMessage);
    await expect
      .poll(async () => (await getTimelineMetrics(pageTwo)).distanceFromBottom)
      .toBeLessThan(8);
    await expect(pageTwo.getByTestId("message-scroll-to-latest")).toHaveCount(
      0,
    );
  } finally {
    await contextOne.close();
    await contextTwo.close();
  }
});

test("keeps scroll position when new messages arrive above the fold", async ({
  browser,
}: {
  browser: Browser;
}) => {
  test.slow();

  const channelName = `scroll-shared-${Date.now()}`;
  const contextOne = await browser.newContext();
  const contextTwo = await browser.newContext();
  const pageOne = await contextOne.newPage();
  const pageTwo = await contextTwo.newPage();
  const prefix = `Scroll behavior ${Date.now()}`;
  const incomingMessage = `${prefix} incoming`;

  try {
    await installRelayBridge(pageOne, "tyler");
    await installRelayBridge(pageTwo, "alice");

    await pageOne.goto("/");
    await pageTwo.goto("/");
    await createAndJoinSharedStream(pageOne, pageTwo, channelName);

    await ensureTimelineScrollable(pageOne, pageTwo, prefix);
    await expect
      .poll(async () => (await getTimelineMetrics(pageTwo)).distanceFromBottom)
      .toBeLessThan(8);

    await scrollTimelineAwayFromBottom(pageTwo);

    await pageOne.getByTestId("message-input").fill(incomingMessage);
    await pageOne.getByTestId("send-message").click();

    await expect(pageTwo.getByTestId("message-scroll-to-latest")).toContainText(
      "1 new message",
    );
    await expect
      .poll(async () => (await getTimelineMetrics(pageTwo)).distanceFromBottom)
      .toBeGreaterThan(160);

    await pageTwo.getByTestId("message-scroll-to-latest").click();

    await expectTimelineToContain(pageTwo, incomingMessage);
    await expect
      .poll(async () => (await getTimelineMetrics(pageTwo)).distanceFromBottom)
      .toBeLessThan(8);
  } finally {
    await contextOne.close();
    await contextTwo.close();
  }
});
