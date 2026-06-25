import { expect, test, type Browser } from "@playwright/test";

import {
  installRelayBridge,
  openChannelBrowser,
  TEST_IDENTITIES,
} from "../helpers/bridge";
import { openSettings } from "../helpers/settings";
import { assertRelaySeeded } from "../helpers/seed";

const isCi = Boolean(process.env.CI);
const relaySeedHookTimeoutMs = isCi ? 90_000 : 30_000;

async function createStream(
  page: import("@playwright/test").Page,
  channelName: string,
  description?: string,
) {
  await page.getByRole("button", { name: "Create a channel" }).click();
  await page.getByTestId("create-channel-name").fill(channelName);
  if (description !== undefined) {
    await page.getByTestId("create-channel-description").fill(description);
  }
  await page.getByTestId("create-channel-submit").click();

  await expect(page.getByTestId("stream-list")).toContainText(channelName);
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
}

async function openChannelManagement(page: import("@playwright/test").Page) {
  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-management-sheet")).toBeVisible();
}

async function openChannelEditDialog(page: import("@playwright/test").Page) {
  await page.getByTestId("channel-management-edit").click();
  await expect(
    page.getByRole("dialog", { name: "Edit channel" }),
  ).toBeVisible();
}

async function closeChannelManagement(page: import("@playwright/test").Page) {
  await page.getByTestId("channel-management-close").click();
  await expect(page.getByTestId("channel-management-sheet")).not.toBeVisible();
}

async function assertDesktopNotificationsEnabled(
  page: import("@playwright/test").Page,
) {
  await openSettings(page, "notifications");
  await expect(page.getByTestId("settings-notifications")).toBeVisible();
  await expect(page.getByTestId("notifications-desktop-state")).toContainText(
    "On",
  );
  await page.getByTestId("settings-back-to-app").click();
}

async function sendChannelMessage(
  page: import("@playwright/test").Page,
  {
    channelName,
    content,
    kind,
    mentionPubkeys,
  }: {
    channelName: string;
    content: string;
    kind?: number | null;
    mentionPubkeys?: string[];
  },
) {
  await page.evaluate(
    async ({
      channelName: targetChannelName,
      content,
      kind,
      mentionPubkeys,
    }) => {
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
        parentEventId: null,
        mediaTags: null,
        mentionPubkeys: mentionPubkeys ?? null,
        kind: kind ?? null,
      });
    },
    { channelName, content, kind, mentionPubkeys },
  );
}

async function joinChannel(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await page.evaluate(async (targetChannelName) => {
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

    await invoke("join_channel", {
      channelId: channel.id,
    });
  }, channelName);
}

async function getLoggedNotifications(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    const win = window as Window & {
      __BUZZ_E2E_NOTIFICATIONS__?: Array<{
        body: string | null;
        title: string;
      }>;
    };

    return win.__BUZZ_E2E_NOTIFICATIONS__ ?? [];
  });
}

async function getLoggedNotificationCount(
  page: import("@playwright/test").Page,
) {
  return (await getLoggedNotifications(page)).length;
}

async function expectLoggedNotifications(
  page: import("@playwright/test").Page,
  expected: Array<{ body: string | null; title: string }>,
) {
  await expect.poll(() => getLoggedNotifications(page)).toEqual(expected);
}

test.beforeAll(async () => {
  test.setTimeout(relaySeedHookTimeoutMs);
  await assertRelaySeeded();
});

test("create channel and verify in sidebar", async ({ page }) => {
  const channelName = `integration-e2e-${Date.now()}`;

  await installRelayBridge(page, "tyler");
  await page.goto("/");
  await page.getByRole("button", { name: "Create a channel" }).click();
  await page.getByTestId("create-channel-name").fill(channelName);
  await page.getByTestId("create-channel-submit").click();

  await expect(page.getByTestId("stream-list")).toContainText(channelName);
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
});

test("two users see the same channel", async ({
  browser,
}: {
  browser: Browser;
}) => {
  const channelName = `shared-channel-${Date.now()}`;
  const contextOne = await browser.newContext();
  const contextTwo = await browser.newContext();
  const pageOne = await contextOne.newPage();
  const pageTwo = await contextTwo.newPage();

  try {
    await installRelayBridge(pageOne, "tyler");
    await installRelayBridge(pageTwo, "alice");

    await pageOne.goto("/");
    await pageOne.getByRole("button", { name: "Create a channel" }).click();
    await pageOne.getByTestId("create-channel-name").fill(channelName);
    await pageOne.getByTestId("create-channel-submit").click();
    await expect(pageOne.getByTestId("stream-list")).toContainText(channelName);

    await pageTwo.goto("/");
    await openChannelBrowser(pageTwo);
    await expect(pageTwo.getByTestId("channel-browser-dialog")).toBeVisible();
    await pageTwo
      .getByTestId(`browse-channel-${channelName}`)
      .getByRole("button", { name: "Join" })
      .click();
    await expect(pageTwo.getByTestId("stream-list")).toContainText(channelName);
  } finally {
    await contextOne.close();
    await contextTwo.close();
  }
});

test("message delivery across users", async ({
  browser,
}: {
  browser: Browser;
}) => {
  const message = `Cross-user message ${Date.now()}`;
  const contextOne = await browser.newContext();
  const contextTwo = await browser.newContext();
  const pageOne = await contextOne.newPage();
  const pageTwo = await contextTwo.newPage();

  try {
    await installRelayBridge(pageOne, "tyler");
    await installRelayBridge(pageTwo, "alice");

    await pageOne.goto("/");
    await pageTwo.goto("/");

    await pageOne.getByTestId("channel-general").click();
    await pageTwo.getByTestId("channel-general").click();
    await expect(pageOne.getByTestId("chat-title")).toHaveText("general");
    await expect(pageTwo.getByTestId("chat-title")).toHaveText("general");

    await pageOne.getByTestId("message-input").fill(message);
    await pageOne.getByTestId("send-message").click();

    await expect(pageTwo.getByTestId("message-timeline")).toContainText(
      message,
    );
  } finally {
    await contextOne.close();
    await contextTwo.close();
  }
});

test("live mentions refetch the home feed without waiting for polling", async ({
  browser,
}: {
  browser: Browser;
}) => {
  const stamp = Date.now();
  const targetContext = await browser.newContext();
  const senderContext = await browser.newContext();
  const targetPage = await targetContext.newPage();
  const senderPage = await senderContext.newPage();

  try {
    await installRelayBridge(targetPage, "tyler");
    await installRelayBridge(senderPage, "alice");

    await targetPage.goto("/");
    await senderPage.goto("/");
    await assertDesktopNotificationsEnabled(targetPage);

    await targetPage.getByTestId("channel-general").click();
    await expect(targetPage.getByTestId("chat-title")).toHaveText("general");

    const message = `Heads up @tyler live mention ${stamp}`;
    await sendChannelMessage(senderPage, {
      channelName: "general",
      content: message,
      mentionPubkeys: [TEST_IDENTITIES.tyler.pubkey],
    });

    await expect(targetPage.getByTestId("message-timeline")).toContainText(
      message,
    );

    await expectLoggedNotifications(targetPage, [
      {
        body: message,
        title: "alice mentioned you in #general",
      },
    ]);

    // The inbox feed should have been refetched live (the original purpose
    // of this test). The home badge stays at 0 while the user is actively
    // reading #general — reading in-channel advances the NIP-RS marker past
    // the new mention — so the assertion that the refetch happened is the
    // inbox-list content, not the badge.
    await targetPage
      .getByTestId("app-sidebar")
      .getByRole("button", { name: "Inbox" })
      .click();
    await expect(targetPage.getByTestId("home-inbox-list")).toBeVisible();
    await expect(targetPage.getByTestId("home-inbox-list")).toContainText(
      message,
    );
    await expect(targetPage.getByTestId("sidebar-home-count")).toHaveCount(0);
    await expect.poll(() => getLoggedNotificationCount(targetPage)).toBe(1);
  } finally {
    await targetContext.close();
    await senderContext.close();
  }
});

test("live forum mentions refetch the home feed without waiting for polling", async ({
  browser,
}: {
  browser: Browser;
}) => {
  const stamp = Date.now();
  const targetContext = await browser.newContext();
  const senderContext = await browser.newContext();
  const targetPage = await targetContext.newPage();
  const senderPage = await senderContext.newPage();

  try {
    await installRelayBridge(targetPage, "tyler");
    await installRelayBridge(senderPage, "alice");

    await targetPage.goto("/");
    await senderPage.goto("/");
    await assertDesktopNotificationsEnabled(targetPage);

    await targetPage.getByTestId("channel-general").click();
    await expect(targetPage.getByTestId("chat-title")).toHaveText("general");
    await joinChannel(senderPage, "watercooler");

    const message = `Forum ping @tyler ${stamp}`;
    await sendChannelMessage(senderPage, {
      channelName: "watercooler",
      content: message,
      kind: 45001,
      mentionPubkeys: [TEST_IDENTITIES.tyler.pubkey],
    });

    await expect(targetPage.getByTestId("sidebar-home-count")).toHaveText("1");

    await expectLoggedNotifications(targetPage, [
      {
        body: message,
        title: "alice mentioned you in #watercooler",
      },
    ]);

    await targetPage
      .getByTestId("app-sidebar")
      .getByRole("button", { name: "Inbox" })
      .click();
    await expect(targetPage.getByTestId("home-inbox-list")).toBeVisible();
    await expect(targetPage.getByTestId("home-inbox-list")).toBeVisible();
    await expect(targetPage.getByTestId("home-inbox-list")).toContainText(
      message,
    );
    await expect(targetPage.getByTestId("sidebar-home-count")).toHaveCount(0);
    await expect.poll(() => getLoggedNotificationCount(targetPage)).toBe(1);
  } finally {
    await targetContext.close();
    await senderContext.close();
  }
});

test("DM channel appears in sidebar", async ({ page }) => {
  await installRelayBridge(page, "tyler");
  await page.goto("/");

  await expect(page.getByTestId("dm-list")).toContainText("alice-tyler");
});

test("send message to DM", async ({ page }) => {
  const message = `DM message ${Date.now()}`;

  await installRelayBridge(page, "tyler");
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");

  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("message-timeline")).toContainText(message);
});

test("forum channel appears in sidebar", async ({ page }) => {
  await installRelayBridge(page, "tyler");
  await page.goto("/");

  await expect(page.getByTestId("forum-list")).toContainText("watercooler");
});

test("create channel with description", async ({ page }) => {
  const channelName = `desc-channel-${Date.now()}`;
  const description = `Description for ${channelName}`;

  await installRelayBridge(page, "tyler");
  await page.goto("/");
  await createStream(page, channelName, description);

  await expect(page.getByTestId("chat-title")).toHaveAttribute(
    "title",
    description,
  );
});

test("multiple channels independent", async ({ page }) => {
  const channelA = `channel-a-${Date.now()}`;
  const channelB = `channel-b-${Date.now()}`;
  const messageA = `Message in A ${Date.now()}`;

  await installRelayBridge(page, "tyler");
  await page.goto("/");

  await page.getByRole("button", { name: "Create a channel" }).click();
  await page.getByTestId("create-channel-name").fill(channelA);
  await page.getByTestId("create-channel-submit").click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelA);

  await page.getByRole("button", { name: "Create a channel" }).click();
  await page.getByTestId("create-channel-name").fill(channelB);
  await page.getByTestId("create-channel-submit").click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelB);

  // Navigate to channel A and send a message
  await page.getByTestId(`channel-${channelA}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelA);
  await page.getByTestId("message-input").fill(messageA);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(messageA);

  // Switch to channel B — message from A should not appear
  await page.getByTestId(`channel-${channelB}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelB);
  await expect(page.getByTestId("message-timeline")).not.toContainText(
    messageA,
  );
});

test("manage sheet updates channel details and context through the relay", async ({
  page,
}) => {
  const stamp = Date.now();
  const initialName = `manage-integration-${stamp}`;
  const renamedChannel = `manage-renamed-${stamp}`;
  const initialDescription = `Initial description ${stamp}`;
  const updatedDescription = `Updated description ${stamp}`;
  const updatedTopic = `Updated topic ${stamp}`;
  const updatedPurpose = `Updated purpose ${stamp}`;

  await installRelayBridge(page, "tyler");
  await page.goto("/");
  await createStream(page, initialName, initialDescription);

  await openChannelManagement(page);
  await openChannelEditDialog(page);
  const editDialog = page.getByRole("dialog", { name: "Edit channel" });

  await editDialog.getByTestId("channel-management-name").fill(renamedChannel);
  await editDialog
    .getByTestId("channel-management-description")
    .fill(updatedDescription);
  await editDialog.getByTestId("channel-management-topic").fill(updatedTopic);
  await editDialog
    .getByTestId("channel-management-purpose")
    .fill(updatedPurpose);
  await editDialog.getByTestId("channel-management-save-changes").click();
  await expect(editDialog).toHaveCount(0);

  await expect(page.getByTestId("chat-title")).toHaveText(renamedChannel);
  await expect(page.getByTestId("stream-list")).toContainText(renamedChannel);

  await closeChannelManagement(page);
  await page.reload();

  await page.getByTestId(`channel-${renamedChannel}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(renamedChannel);
  // channelDescription deduplicates by showing only the first non-empty field
  await expect(page.getByTestId("chat-title")).toHaveAttribute(
    "title",
    updatedTopic,
  );

  await openChannelManagement(page);
  await openChannelEditDialog(page);
  const reopenedEditDialog = page.getByRole("dialog", {
    name: "Edit channel",
  });

  await expect(
    reopenedEditDialog.getByTestId("channel-management-name"),
  ).toHaveValue(renamedChannel);
  await expect(
    reopenedEditDialog.getByTestId("channel-management-description"),
  ).toHaveValue(updatedDescription);
  await expect(
    reopenedEditDialog.getByTestId("channel-management-topic"),
  ).toHaveValue(updatedTopic);
  await expect(
    reopenedEditDialog.getByTestId("channel-management-purpose"),
  ).toHaveValue(updatedPurpose);
});

test("manage sheet archive and unarchive survives a reload through the relay", async ({
  page,
}) => {
  const channelName = `archive-integration-${Date.now()}`;

  await installRelayBridge(page, "tyler");
  await page.goto("/");
  await createStream(page, channelName, "Archive integration channel");

  await openChannelManagement(page);
  await page.getByTestId("channel-management-archive").click();
  await expect(page.getByTestId("channel-management-unarchive")).toBeVisible();
  await closeChannelManagement(page);

  await expect(page.getByTestId("stream-list")).not.toContainText(channelName);
  await expect(page.getByTestId("message-input")).toHaveAttribute(
    "contenteditable",
    "false",
  );
  await expect(page.getByTestId("send-message")).toBeDisabled();

  await page.reload();

  await expect(page.getByTestId("stream-list")).not.toContainText(channelName);
  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();
  await expect(page.getByTestId(`browse-channel-${channelName}`)).toContainText(
    "archived",
  );
  await page.getByTestId(`browse-channel-${channelName}`).click();
  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  await expect(page.getByTestId("message-input")).toHaveAttribute(
    "contenteditable",
    "false",
  );

  await openChannelManagement(page);
  await page.getByTestId("channel-management-unarchive").click();
  await expect(page.getByTestId("channel-management-archive")).toBeVisible();
  await closeChannelManagement(page);

  await expect(page.getByTestId("stream-list")).toContainText(channelName);
  await expect(page.getByTestId("message-input")).toHaveAttribute(
    "contenteditable",
    "true",
  );
});
