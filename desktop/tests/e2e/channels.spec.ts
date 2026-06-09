import { expect, test } from "@playwright/test";

import { KIND_TYPING_INDICATOR } from "../../src/shared/constants/kinds";
import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const MOCK_IDENTITY_PUBKEY = "deadbeef".repeat(8);

async function openChannelManagement(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await page.getByTestId(`channel-${channelName}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-management-sheet")).toBeVisible();
}

async function closeChannelManagement(page: import("@playwright/test").Page) {
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("channel-management-sheet")).not.toBeVisible();
}

async function openMembersSidebar(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await page.getByTestId(`channel-${channelName}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  await page.getByTestId("channel-members-trigger").click();
  await expect(page.getByTestId("members-sidebar")).toBeVisible();
}

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
  kind?: number,
) {
  await expect
    .poll(async () => {
      return page.evaluate(
        ({ currentChannelName, kind }) => {
          return (
            (
              window as Window & {
                __SPROUT_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                  channelName: string;
                  kind?: number;
                }) => boolean;
              }
            ).__SPROUT_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: currentChannelName,
              kind,
            }) ?? false
          );
        },
        { currentChannelName: channelName, kind },
      );
    })
    .toBe(true);
}

async function openMemberMenu(
  page: import("@playwright/test").Page,
  pubkey: string,
) {
  const row = page.getByTestId(`sidebar-member-${pubkey}`);
  const trigger = page.getByTestId(`sidebar-member-menu-${pubkey}`);
  await row.scrollIntoViewIfNeeded();
  await row.hover();
  // Workaround: @radix-ui/react-dropdown-menu@2.1.16 ignores pointer-based
  // re-opens after a menu item click (onCloseAutoFocus race). Opening via
  // keyboard (focus + Enter) is reliable. Revisit if Radix fixes this.
  await trigger.focus();
  await trigger.press("Enter");
  await expect(trigger).toHaveAttribute("data-state", "open");
}

async function addGenericAgent(
  page: import("@playwright/test").Page,
  channelName: string,
  agentName: string,
) {
  await page.getByTestId(`channel-${channelName}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  await page.getByTestId("channel-add-bot-trigger").click();
  await expect(page.getByRole("heading", { name: "Add agents" })).toBeVisible();
  await page.getByRole("button", { name: "Generic" }).click();
  await page.locator("#channel-generic-name").fill(agentName);
  await page
    .locator("#channel-generic-prompt")
    .fill("Watch the channel and help when asked.");
  await page.getByRole("button", { name: "Add agent" }).click();
  await expect(page.getByRole("heading", { name: "Add agents" })).toHaveCount(
    0,
  );
}

async function getManagedAgentPubkey(
  page: import("@playwright/test").Page,
  agentName: string,
) {
  await page.getByTestId("open-agents-view").click();
  const managedAgentRow = page
    .locator('[data-testid^="managed-agent-"]')
    .filter({ hasText: agentName });
  await expect(managedAgentRow).toHaveCount(1);
  const managedAgentTestId = await managedAgentRow
    .first()
    .getAttribute("data-testid");
  if (!managedAgentTestId) {
    throw new Error("Managed agent row test id missing.");
  }

  return managedAgentTestId.replace("managed-agent-", "");
}

async function readCommandLog(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    return (
      (
        window as Window & {
          __SPROUT_E2E_COMMANDS__?: string[];
        }
      ).__SPROUT_E2E_COMMANDS__ ?? []
    );
  });
}

async function invokeMockCommand<T>(
  page: import("@playwright/test").Page,
  command: string,
  payload?: Record<string, unknown>,
) {
  await page.waitForFunction(() => {
    return Boolean(
      (
        window as Window & {
          __SPROUT_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        }
      ).__SPROUT_E2E_INVOKE_MOCK_COMMAND__,
    );
  });

  return page.evaluate(
    async ({
      command,
      payload,
    }: {
      command: string;
      payload?: Record<string, unknown>;
    }) => {
      const invoke = (
        window as Window & {
          __SPROUT_E2E_INVOKE_MOCK_COMMAND__?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<unknown>;
        }
      ).__SPROUT_E2E_INVOKE_MOCK_COMMAND__;

      if (!invoke) {
        throw new Error("Mock bridge is not installed.");
      }

      return invoke(command, payload);
    },
    { command, payload },
  ) as Promise<T>;
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("sidebar shows all channel types", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await expect(page.getByTestId("sidebar-agents-count")).toHaveText("0");

  // Streams
  const streamList = page.getByTestId("stream-list");
  await expect(streamList).toContainText("general");
  await expect(streamList).toContainText("random");
  await expect(streamList).toContainText("engineering");
  await expect(streamList).toContainText("agents");

  // Forums
  const forumList = page.getByTestId("forum-list");
  await expect(forumList).toContainText("watercooler");
  await expect(forumList).toContainText("announcements");

  // DMs
  const dmList = page.getByTestId("dm-list");
  await expect(dmList).toContainText("alice-tyler");
  await expect(dmList).toContainText("bob-tyler");
});

test("shows presence in sidebar, DM header, and member list", async ({
  page,
}) => {
  await page.goto("/");

  await expect(page.getByTestId("sidebar-profile-card")).toBeVisible();
  await expect(page.getByTestId("self-presence-badge")).toHaveAttribute(
    "aria-label",
    "Online",
  );
  await expect(page.getByTestId("channel-presence-alice-tyler")).toBeVisible();

  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");
  await expect(page.getByTestId("chat-presence-badge")).toContainText("Online");

  await openMembersSidebar(page, "general");
  await expect(
    page.getByTestId(`sidebar-member-presence-${TEST_IDENTITIES.alice.pubkey}`),
  ).toBeVisible();
  await expect(
    page.getByTestId(`sidebar-member-presence-${TEST_IDENTITIES.bob.pubkey}`),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("members-sidebar")).not.toBeVisible();
});

test("start a new direct message from the sidebar", async ({ page }) => {
  await page.goto("/");

  await page.getByTestId("new-dm-trigger").click();
  await expect(page.getByTestId("new-dm-dialog")).toBeVisible();

  await page.getByTestId("new-dm-search").fill("charlie");
  await page
    .getByTestId(`new-dm-result-${TEST_IDENTITIES.charlie.pubkey}`)
    .click();
  await expect(
    page.getByTestId(`new-dm-selected-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toBeVisible();

  await page.getByTestId("new-dm-submit").click();

  await expect(page.getByTestId("dm-list")).toContainText("charlie");
  await expect(page.getByTestId("chat-title")).toHaveText("charlie");
});

test("create stream with name and description", async ({ page }) => {
  const channelName = `my-new-stream-${Date.now()}`;

  await page.goto("/");
  await page.getByRole("button", { name: "Create a channel" }).click();
  await page.getByTestId("create-channel-name").fill(channelName);
  await page
    .getByTestId("create-channel-description")
    .fill("A stream for testing channel creation");
  await page.getByTestId("create-channel-submit").click();

  await expect(page.getByTestId("stream-list")).toContainText(channelName);
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
});

test("create ephemeral stream shows sidebar and header affordances", async ({
  page,
}) => {
  const channelName = `ephemeral-stream-${Date.now()}`;

  await page.goto("/");
  await page.getByRole("button", { name: "Create a channel" }).click();
  await page.getByTestId("create-channel-name").fill(channelName);
  await page
    .getByTestId("create-channel-description")
    .fill("Auto-cleaned test stream");
  await page
    .getByLabel("Ephemeral — auto-archives after 1 day of inactivity")
    .click();
  await page.getByTestId("create-channel-submit").click();

  await expect(page.getByTestId("stream-list")).toContainText(channelName);
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  await expect(
    page.getByTestId(`channel-ephemeral-${channelName}`),
  ).toBeVisible();
  await expect(page.getByTestId("chat-ephemeral-badge")).toBeVisible();
  await expect(page.getByTestId("chat-ephemeral-badge")).toHaveAttribute(
    "title",
    /Ephemeral channel\. Cleans up (tomorrow|in \d+ hours?)\./,
  );

  await page
    .getByRole("button", { name: "Toggle Sidebar", exact: true })
    .click();
  await expect(
    page.getByTestId(`channel-ephemeral-${channelName}`),
  ).toBeVisible();
});

test("ephemeral countdown refreshes when switching channels after a clock jump", async ({
  page,
}) => {
  const firstChannelName = "ephemeral-alpha";
  const secondChannelName = "ephemeral-beta";
  const initialTime = new Date("2026-04-09T00:00:00.000Z");
  const shiftedTime = new Date("2026-04-09T02:00:00.000Z");

  await page.clock.setFixedTime(initialTime);
  await page.goto("/");

  for (const channelName of [firstChannelName, secondChannelName]) {
    await page.getByRole("button", { name: "Create a channel" }).click();
    await page.getByTestId("create-channel-name").fill(channelName);
    await page
      .getByTestId("create-channel-description")
      .fill("Auto-cleaned test stream");
    await page
      .getByLabel("Ephemeral — auto-archives after 1 day of inactivity")
      .click();
    await page.getByTestId("create-channel-submit").click();
    await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  }

  await page.clock.setFixedTime(shiftedTime);
  await expect
    .poll(() => page.evaluate(() => Date.now()))
    .toBe(shiftedTime.getTime());

  await page.getByTestId(`channel-${firstChannelName}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(firstChannelName);
  await expect(page.getByTestId("chat-ephemeral-badge")).toHaveAttribute(
    "title",
    /Ephemeral channel\. Cleans up in 22 hours\./,
  );
});

test("archived channels stay out of all sidebar sections", async ({ page }) => {
  const archivedStreamName = `archived-stream-${Date.now()}`;
  const archivedForumName = `archived-forum-${Date.now()}`;

  await page.goto("/");
  await page.waitForFunction(() => {
    return Boolean(
      (
        window as Window & {
          __SPROUT_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        }
      ).__SPROUT_E2E_INVOKE_MOCK_COMMAND__,
    );
  });
  await page.evaluate(
    async ({
      archivedForumName,
      archivedStreamName,
      outsiderPubkey,
    }: {
      archivedForumName: string;
      archivedStreamName: string;
      outsiderPubkey: string;
    }) => {
      const invoke = (
        window as Window & {
          __SPROUT_E2E_INVOKE_MOCK_COMMAND__?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<{ id: string }>;
        }
      ).__SPROUT_E2E_INVOKE_MOCK_COMMAND__;

      if (!invoke) {
        throw new Error("Mock bridge is not installed.");
      }

      const stream = await invoke("create_channel", {
        channelType: "stream",
        name: archivedStreamName,
        visibility: "open",
      });
      const forum = await invoke("create_channel", {
        channelType: "forum",
        name: archivedForumName,
        visibility: "open",
      });
      const directMessage = await invoke("open_dm", {
        pubkeys: [outsiderPubkey],
      });

      for (const channel of [stream, forum, directMessage]) {
        await invoke("archive_channel", { channelId: channel.id });
      }
    },
    {
      archivedForumName,
      archivedStreamName,
      outsiderPubkey: TEST_IDENTITIES.outsider.pubkey,
    },
  );

  await page.reload();

  await expect(page.getByTestId("stream-list")).not.toContainText(
    archivedStreamName,
  );
  await expect(page.getByTestId("forum-list")).not.toContainText(
    archivedForumName,
  );
  await expect(page.getByTestId("dm-list")).toContainText("alice-tyler");
  await expect(page.getByTestId("dm-list")).not.toContainText("outsider");
});

test("create stream with special characters", async ({ page }) => {
  const channelName = `dev ops-${Date.now()}`;

  await page.goto("/");
  await page.getByRole("button", { name: "Create a channel" }).click();
  await page.getByTestId("create-channel-name").fill(channelName);
  await page
    .getByTestId("create-channel-description")
    .fill("Stream with spaces and hyphens");
  await page.getByTestId("create-channel-submit").click();

  await expect(page.getByTestId("stream-list")).toContainText(channelName);
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
});

test("switch between streams", async ({ page }) => {
  await page.goto("/");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");

  await page.getByTestId("channel-engineering").click();
  await expect(page.getByTestId("chat-title")).toHaveText("engineering");
});

test("switch between channel types", async ({ page }) => {
  await page.goto("/");

  // Stream
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Forum
  await page.getByTestId("channel-watercooler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");

  // DM
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");
});

test("empty channel shows empty state", async ({ page }) => {
  await page.goto("/");

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(page.getByTestId("message-empty")).toBeVisible();
});

test("channel with messages shows content", async ({ page }) => {
  await page.goto("/");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Welcome to #general",
  );
});

test("shows and clears activity indicators for active channel agents", async ({
  page,
}) => {
  await page.goto("/");

  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");
  await waitForMockLiveSubscription(page, "agents", KIND_TYPING_INDICATOR);

  await page.evaluate((pubkey) => {
    window.__SPROUT_E2E_EMIT_MOCK_TYPING__?.({
      channelName: "agents",
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  await expect(page.getByTestId("bot-activity-composer-trigger")).toBeVisible();
  await page.getByTestId("bot-activity-composer-trigger").click();
  await expect(
    page.getByTestId(
      `bot-activity-composer-item-${TEST_IDENTITIES.alice.pubkey}`,
    ),
  ).toBeVisible();
  await page
    .getByTestId(`bot-activity-composer-item-${TEST_IDENTITIES.alice.pubkey}`)
    .click({ force: true });
  await expect(page.getByTestId("agent-session-thread-panel")).toBeVisible();
  await expect(page.getByTestId("agent-session-thread-panel")).toContainText(
    "alice",
  );
  await expect(page.getByTestId("message-typing-indicator")).toHaveCount(0);

  await page.evaluate((pubkey) => {
    window.__SPROUT_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "agents",
      content: "Done.",
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  await expect(page.getByTestId("message-timeline")).toContainText("Done.");
  await expect(page.getByTestId("bot-activity-composer-trigger")).toHaveCount(
    0,
  );

  await page.waitForTimeout(1_200);
  await page.evaluate((pubkey) => {
    window.__SPROUT_E2E_EMIT_MOCK_TYPING__?.({
      channelName: "agents",
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  await expect(page.getByTestId("bot-activity-composer-trigger")).toHaveCount(
    0,
  );
});

test("typing indicator shows avatars and maintains stable name order", async ({
  page,
}) => {
  await page.goto("/");

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await waitForMockLiveSubscription(page, "random", KIND_TYPING_INDICATOR);

  // Alice starts typing first
  await page.evaluate((pubkey) => {
    window.__SPROUT_E2E_EMIT_MOCK_TYPING__?.({
      channelName: "random",
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  await expect(page.getByTestId("message-typing-indicator")).toBeVisible();
  await expect(
    page.getByTestId("message-typing-indicator-label"),
  ).toContainText("alice is typing");

  // Verify avatar is rendered for the typing user
  const avatars = page
    .getByTestId("message-typing-indicator")
    .locator("[data-testid='message-typing-avatar']");
  await expect(avatars).toHaveCount(1);

  // Bob starts typing second
  await page.evaluate((pubkey) => {
    window.__SPROUT_E2E_EMIT_MOCK_TYPING__?.({
      channelName: "random",
      pubkey,
    });
  }, TEST_IDENTITIES.bob.pubkey);

  await expect(
    page.getByTestId("message-typing-indicator-label"),
  ).toContainText("alice and bob are typing");
  await expect(avatars).toHaveCount(2);

  // Alice re-broadcasts — order should stay "alice and bob", not flip
  await page.evaluate((pubkey) => {
    window.__SPROUT_E2E_EMIT_MOCK_TYPING__?.({
      channelName: "random",
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  await expect(
    page.getByTestId("message-typing-indicator-label"),
  ).toContainText("alice and bob are typing");

  // Bob re-broadcasts — order should still stay "alice and bob"
  await page.evaluate((pubkey) => {
    window.__SPROUT_E2E_EMIT_MOCK_TYPING__?.({
      channelName: "random",
      pubkey,
    });
  }, TEST_IDENTITIES.bob.pubkey);

  await expect(
    page.getByTestId("message-typing-indicator-label"),
  ).toContainText("alice and bob are typing");
});

test("sidebar shows unread indicator for newly active channels", async ({
  page,
}) => {
  await page.goto("/");

  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);
  await waitForMockLiveSubscription(page, "random");

  // The unread tracker ignores the current user's own messages, so emit as
  // alice — simulating a real "another user posted while I was elsewhere".
  await page.evaluate(
    ({ pubkey }) => {
      window.__SPROUT_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: "Unread update for #random",
        kind: 40002,
        pubkey,
      });
    },
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );

  await expect(page.getByTestId("channel-unread-random")).toBeVisible();

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Unread update for #random",
  );
  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);
});

test("sidebar shows unread indicator for new forum posts", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByTestId("channel-unread-watercooler")).toHaveCount(0);
  await waitForMockLiveSubscription(page, "watercooler");

  // Emit as alice — the unread tracker ignores self-authored messages.
  await page.evaluate(
    ({ pubkey }) => {
      window.__SPROUT_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "watercooler",
        content: "Unread update for the forum",
        kind: 45001,
        pubkey,
      });
    },
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );

  await expect(page.getByTestId("channel-unread-watercooler")).toBeVisible();

  await page.getByTestId("channel-watercooler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");
  await expect(page.getByTestId("channel-unread-watercooler")).toHaveCount(0);
});

test("sidebar clears unread indicator after opening a DM", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByTestId("channel-unread-alice-tyler")).toHaveCount(0);
  await waitForMockLiveSubscription(page, "alice-tyler");

  await page.evaluate((pubkey) => {
    window.__SPROUT_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "alice-tyler",
      content: "Unread update for the DM",
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  await expect(page.getByTestId("channel-unread-alice-tyler")).toBeVisible();

  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Unread update for the DM",
  );
  await expect(page.getByTestId("channel-unread-alice-tyler")).toHaveCount(0);
});

test("sidebar persists after channel switch", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  await page.getByTestId("channel-watercooler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
});

test("manage channel updates details and context", async ({ page }) => {
  const stamp = Date.now();
  const newName = `release-hub-${stamp}`;
  const newDescription = `Release coordination ${stamp}`;
  const newTopic = `Launch plan ${stamp}`;
  const newPurpose = `Track blockers and owners ${stamp}`;

  await page.goto("/");
  await openChannelManagement(page, "general");

  await page.getByTestId("channel-management-name").fill(newName);
  await page.getByTestId("channel-management-description").fill(newDescription);
  await page.getByTestId("channel-management-save-details").click();

  await expect(page.getByTestId("chat-title")).toHaveText(newName);
  await expect(page.getByTestId("stream-list")).toContainText(newName);

  const saveTopicButton = page.getByTestId("channel-management-save-topic");
  const savePurposeButton = page.getByTestId("channel-management-save-purpose");

  await page.getByTestId("channel-management-topic").fill(newTopic);
  await saveTopicButton.click();
  await expect(saveTopicButton).toHaveText("Save topic");
  await expect(page.getByTestId("channel-management-topic")).toHaveValue(
    newTopic,
  );

  await page.getByTestId("channel-management-purpose").fill(newPurpose);
  await savePurposeButton.click();
  await expect(savePurposeButton).toHaveText("Save purpose");
  await expect(page.getByTestId("channel-management-purpose")).toHaveValue(
    newPurpose,
  );

  await closeChannelManagement(page);

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");

  await page.getByTestId("stream-list").getByText(newName).click();
  await expect(page.getByTestId("chat-title")).toHaveText(newName);
  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-management-sheet")).toBeVisible();

  await expect(page.getByTestId("channel-management-name")).toHaveValue(
    newName,
  );
  await expect(page.getByTestId("channel-management-description")).toHaveValue(
    newDescription,
  );
  await expect(page.getByTestId("channel-management-topic")).toHaveValue(
    newTopic,
  );
  await expect(page.getByTestId("channel-management-purpose")).toHaveValue(
    newPurpose,
  );
});

test("manage channel keeps canvas near the top of the sheet", async ({
  page,
}) => {
  await page.goto("/");
  await openChannelManagement(page, "general");

  const sheet = page.getByTestId("channel-management-sheet");

  // Canvas section should appear before the name input in the DOM.
  const canvasBox = await sheet
    .getByTestId("channel-canvas-section")
    .boundingBox();
  const nameBox = await sheet
    .getByTestId("channel-management-name")
    .boundingBox();

  expect(canvasBox).not.toBeNull();
  expect(nameBox).not.toBeNull();
  expect(canvasBox?.y).toBeLessThan(nameBox?.y);
});

test("members sidebar can invite and remove members", async ({ page }) => {
  await page.goto("/");
  await openMembersSidebar(page, "general");
  await expect(page.getByTestId("channel-members-trigger")).toContainText("3");
  await expect(page.getByTestId("channel-management-add-pubkeys")).toHaveCount(
    0,
  );

  await page.getByTestId("channel-management-search-users").fill("char");
  await expect(
    page.getByTestId(
      `channel-user-search-result-${TEST_IDENTITIES.charlie.pubkey}`,
    ),
  ).toBeVisible();
  await page
    .getByTestId(`channel-user-search-result-${TEST_IDENTITIES.charlie.pubkey}`)
    .click();
  await expect(
    page.getByTestId(`selected-invitee-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toContainText("charlie");

  await page.getByTestId("channel-management-add-role").selectOption("admin");
  await page.getByTestId("channel-management-add-members").click();

  await expect(
    page.getByTestId(`selected-invitee-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toHaveCount(0);
  await expect(page.getByTestId("channel-management-search-users")).toHaveValue(
    "",
  );
  await expect(
    page.getByTestId(`sidebar-member-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toContainText("charlie");
  await expect(page.getByTestId("channel-members-trigger")).toContainText("4");

  await openMemberMenu(page, TEST_IDENTITIES.charlie.pubkey);
  await page
    .getByTestId(`sidebar-remove-member-${TEST_IDENTITIES.charlie.pubkey}`)
    .click();

  await expect(
    page.getByTestId(`sidebar-member-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toHaveCount(0);
  await expect(page.getByTestId("channel-members-trigger")).toContainText("3");
});

test("members sidebar keeps direct pubkey entry behind a toggle", async ({
  page,
}) => {
  await page.goto("/");
  await openMembersSidebar(page, "general");

  await expect(page.getByTestId("channel-management-add-pubkeys")).toHaveCount(
    0,
  );

  await page.getByTestId("channel-management-toggle-direct-pubkeys").click();
  await expect(
    page.getByTestId("channel-management-add-pubkeys"),
  ).toBeVisible();

  await page
    .getByTestId("channel-management-add-pubkeys")
    .fill(TEST_IDENTITIES.outsider.pubkey);
  await page.getByTestId("channel-management-add-members").click();

  await expect(
    page.getByTestId(`sidebar-member-${TEST_IDENTITIES.outsider.pubkey}`),
  ).toContainText("outsider");
  await expect(page.getByTestId("channel-members-trigger")).toContainText("4");
});

test("open-channel members can add agents from the header", async ({
  page,
}) => {
  await page.goto("/");

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await page.setViewportSize({ width: 1280, height: 420 });

  const addAgentTrigger = page.getByTestId("channel-add-bot-trigger");
  await expect(addAgentTrigger).toBeEnabled();

  await addAgentTrigger.click();
  await expect(page.getByRole("heading", { name: "Add agents" })).toBeVisible();
  await expect(page.getByTestId("add-channel-bot-dialog-header")).toBeVisible();
  await expect(
    page.getByTestId("add-channel-bot-dialog-scroll-area"),
  ).toBeVisible();
  await expect(
    page.getByTestId("add-channel-bot-dialog-scroll-area"),
  ).toHaveCSS("overflow-y", "auto");
  expect(
    await page
      .getByTestId("add-channel-bot-dialog-scroll-area")
      .evaluate(
        (element) =>
          element.scrollHeight > element.clientHeight &&
          element.clientHeight > 0,
      ),
  ).toBe(true);
  await expect(page.getByTestId("add-channel-bot-dialog-footer")).toBeVisible();
});

test("private-channel members can add members and bots without admin", async ({
  page,
}) => {
  await page.goto("/");
  // secret-projects is a private (non-DM) channel where the current user is a
  // plain member, not owner/admin. They should still be able to add members
  // and bots — only granting elevated roles is reserved for owners/admins.
  await openMembersSidebar(page, "secret-projects");

  // The invite card is shown to any member, not just owners/admins.
  await expect(
    page.getByTestId("channel-management-search-users"),
  ).toBeVisible();
  await expect(
    page.getByTestId("channel-management-add-members"),
  ).toBeVisible();

  // The role dropdown hides the elevated "admin" option for non-admins — the
  // relay rejects it anyway — while keeping the non-elevated roles a member
  // may grant (member, guest, bot).
  const roleSelect = page.getByTestId("channel-management-add-role");
  await expect(roleSelect.locator("option")).toHaveText([
    "member",
    "guest",
    "bot",
  ]);
});

test("removing a channel-scoped agent preserves the managed agent record", async ({
  page,
}) => {
  const agentName = `cleanup-agent-${Date.now()}`;

  await page.goto("/");
  await addGenericAgent(page, "general", agentName);
  const agentPubkey = await getManagedAgentPubkey(page, agentName);

  await page.getByTestId("channel-general").click();
  await openMembersSidebar(page, "general");

  await openMemberMenu(page, agentPubkey);
  await page.getByTestId(`sidebar-remove-member-${agentPubkey}`).click();
  await expect(page.getByTestId(`sidebar-member-${agentPubkey}`)).toHaveCount(
    0,
  );
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("members-sidebar")).not.toBeVisible();

  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId(`managed-agent-${agentPubkey}`)).toHaveCount(1);
});

test("members sidebar can respawn a stopped managed bot", async ({ page }) => {
  const agentName = `sidebar-agent-${Date.now()}`;

  await page.goto("/");
  await addGenericAgent(page, "general", agentName);

  const agentPubkey = await getManagedAgentPubkey(page, agentName);
  const baselineCommands = await readCommandLog(page);
  const baselineStartCount = baselineCommands.filter(
    (command) => command === "start_managed_agent",
  ).length;
  const baselineStopCount = baselineCommands.filter(
    (command) => command === "stop_managed_agent",
  ).length;

  await openMembersSidebar(page, "general");

  const agentStatus = page.getByTestId(
    `sidebar-managed-agent-status-${agentPubkey}`,
  );
  const agentAction = page.getByTestId(`sidebar-agent-action-${agentPubkey}`);

  await expect(agentStatus).toContainText("Running");
  await openMemberMenu(page, agentPubkey);
  await expect(agentAction).toContainText("Stop");
  await agentAction.click();

  await expect(agentStatus).toContainText("Stopped");
  await openMemberMenu(page, agentPubkey);
  await expect(agentAction).toContainText("Respawn");
  await agentAction.click();

  await expect(agentStatus).toContainText("Running");
  await expect(
    page
      .locator("[data-sonner-toast]")
      .filter({ hasText: `Respawned ${agentName}.` }),
  ).toBeVisible();

  const commands = await readCommandLog(page);
  expect(
    commands.filter((command) => command === "start_managed_agent").length,
  ).toBe(baselineStartCount + 1);
  expect(
    commands.filter((command) => command === "stop_managed_agent").length,
  ).toBe(baselineStopCount + 1);
});

test("members sidebar supports bulk remove for managed bots from channel", async ({
  page,
}) => {
  const firstAgentName = `sidebar-remove-a-${Date.now()}`;
  const secondAgentName = `sidebar-remove-b-${Date.now()}`;

  await page.goto("/");
  await addGenericAgent(page, "general", firstAgentName);
  await addGenericAgent(page, "general", secondAgentName);

  const firstAgentPubkey = await getManagedAgentPubkey(page, firstAgentName);
  const secondAgentPubkey = await getManagedAgentPubkey(page, secondAgentName);

  await openMembersSidebar(page, "general");
  await expect(
    page.getByTestId("members-sidebar-agent-controls"),
  ).toBeVisible();

  await page.getByTestId("members-sidebar-agent-controls").click();
  await page.getByTestId("members-sidebar-remove-all").click();
  await expect(
    page
      .locator("[data-sonner-toast]")
      .filter({ hasText: "Removed 2 managed bots from this channel." }),
  ).toBeVisible();
  await expect(
    page.getByTestId(`sidebar-member-${firstAgentPubkey}`),
  ).toHaveCount(0);
  await expect(
    page.getByTestId(`sidebar-member-${secondAgentPubkey}`),
  ).toHaveCount(0);

  await page.keyboard.press("Escape");
  await expect(page.getByTestId("members-sidebar")).not.toBeVisible();

  await page.getByTestId("open-agents-view").click();
  await expect(
    page.getByTestId(`managed-agent-${firstAgentPubkey}`),
  ).toHaveCount(1);
  await expect(
    page.getByTestId(`managed-agent-${secondAgentPubkey}`),
  ).toHaveCount(1);

  const commands = await readCommandLog(page);
  expect(
    commands.filter((command) => command === "remove_channel_member"),
  ).toHaveLength(2);
});

test("removing a multi-channel managed bot preserves its record after removal from all channels", async ({
  page,
}) => {
  const agentName = `multi-channel-agent-${Date.now()}`;
  const secondChannelName = `multi-home-${Date.now()}`;

  await page.goto("/");
  await addGenericAgent(page, "general", agentName);
  const agentPubkey = await getManagedAgentPubkey(page, agentName);

  await page.getByRole("button", { name: "Create a channel" }).click();
  await page.getByTestId("create-channel-name").fill(secondChannelName);
  await page
    .getByTestId("create-channel-description")
    .fill("Second home for managed bot cleanup coverage");
  await page.getByTestId("create-channel-submit").click();
  await expect(page.getByTestId("chat-title")).toHaveText(secondChannelName);

  const secondChannelId = await page
    .getByTestId(`channel-${secondChannelName}`)
    .getAttribute("data-channel-id");
  if (!secondChannelId) {
    throw new Error("Second channel id missing.");
  }

  await invokeMockCommand(page, "add_channel_members", {
    channelId: secondChannelId,
    pubkeys: [agentPubkey],
    role: "bot",
  });

  // Snapshot command counts before removals so assertions are relative.
  const baseline = await readCommandLog(page);
  const baselineRemoves = baseline.filter(
    (c) => c === "remove_channel_member",
  ).length;

  await openMembersSidebar(page, "general");
  await openMemberMenu(page, agentPubkey);
  await page.getByTestId(`sidebar-remove-member-${agentPubkey}`).click();
  await expect(page.getByTestId(`sidebar-member-${agentPubkey}`)).toHaveCount(
    0,
  );
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("members-sidebar")).not.toBeVisible();

  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId(`managed-agent-${agentPubkey}`)).toHaveCount(1);

  let commands = await readCommandLog(page);
  // First removal: 1 remove_channel_member, agent record preserved.
  expect(
    commands.filter((c) => c === "remove_channel_member").length -
      baselineRemoves,
  ).toBe(1);

  await openMembersSidebar(page, secondChannelName);
  await openMemberMenu(page, agentPubkey);
  await page.getByTestId(`sidebar-remove-member-${agentPubkey}`).click();
  await expect(page.getByTestId(`sidebar-member-${agentPubkey}`)).toHaveCount(
    0,
  );
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("members-sidebar")).not.toBeVisible();

  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId(`managed-agent-${agentPubkey}`)).toHaveCount(1);

  commands = await readCommandLog(page);
  // Second removal: agent is preserved even after removal from all channels.
  expect(
    commands.filter((c) => c === "remove_channel_member").length -
      baselineRemoves,
  ).toBe(2);
});

test("bulk remove stays hidden when row-level remove is not allowed", async ({
  page,
}) => {
  const alicePubkey =
    "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";

  await page.goto("/");

  // Join the "design" channel (unjoined by default) via the channel browser.
  // The user becomes a regular member — not admin/owner.
  await page.getByTestId("browse-channels").click();
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();
  await page
    .getByTestId("browse-channel-design")
    .getByRole("button", { name: "Join" })
    .click();
  await expect(page.getByTestId("chat-title")).toHaveText("design");

  await openMembersSidebar(page, "design");

  // Alice is a relay-observed bot in design (present in mockRelayAgents) that
  // the user does not manage locally. Since there is no local managed agent
  // for alice, hasActions is false and no 3-dot menu renders.
  await expect(page.getByTestId(`sidebar-member-${alicePubkey}`)).toBeVisible();
  await expect(
    page.getByTestId(`sidebar-member-menu-${alicePubkey}`),
  ).toHaveCount(0);

  // Bulk agent controls only render when hasControllableManagedBots is true.
  // Since no bots in design have a local managed agent, the controls are absent.
  await expect(page.getByTestId("members-sidebar-agent-controls")).toHaveCount(
    0,
  );
});

test("open channel management supports join and leave", async ({ page }) => {
  await page.goto("/");

  // Navigate to "design" (an unjoined channel) via the channel browser
  await page.getByTestId("browse-channels").click();
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();
  await page
    .getByTestId("browse-channel-design")
    .getByRole("button", { name: "Join" })
    .click();
  await expect(page.getByTestId("chat-title")).toHaveText("design");

  // Open members sidebar — should show current user after joining
  await page.getByTestId("channel-members-trigger").click();
  await expect(page.getByTestId("members-sidebar")).toBeVisible();
  await expect(
    page.getByTestId(`sidebar-member-${MOCK_IDENTITY_PUBKEY}`),
  ).toContainText("You");
  await page.keyboard.press("Escape");

  // Open channel management — should show Leave since we just joined
  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-management-sheet")).toBeVisible();
  await expect(page.getByTestId("channel-management-join")).toHaveCount(0);
  await expect(page.getByTestId("channel-management-leave")).toBeVisible();

  // Leave the channel
  await page.getByTestId("channel-management-leave").click();
  await expect(page.getByTestId("channel-management-sheet")).not.toBeVisible();

  // After leaving, the app navigates away — re-open browser and find design
  await page.getByTestId("browse-channels").click();
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();

  // "design" should be back in the unjoined section with a Join button
  await expect(
    page
      .getByTestId("browse-channel-design")
      .getByRole("button", { name: "Join" }),
  ).toBeVisible();
});

test("manage channel can archive and unarchive a stream", async ({ page }) => {
  await page.goto("/");
  await openChannelManagement(page, "general");

  await page.getByTestId("channel-management-archive").click();
  await expect(page.getByTestId("channel-management-unarchive")).toBeVisible();

  await closeChannelManagement(page);
  await expect(page.getByTestId("stream-list")).not.toContainText("general");
  await expect(page.getByTestId("message-input")).toHaveAttribute(
    "contenteditable",
    "false",
  );
  await expect(page.getByTestId("send-message")).toBeDisabled();

  await page.getByTestId("browse-channels").click();
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();
  await expect(page.getByTestId("browse-channel-general")).toContainText(
    "archived",
  );
  await page.getByTestId("browse-channel-general").click();
  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-management-sheet")).toBeVisible();
  await page.getByTestId("channel-management-unarchive").click();
  await expect(page.getByTestId("channel-management-archive")).toBeVisible();

  await closeChannelManagement(page);
  await expect(page.getByTestId("stream-list")).toContainText("general");
  await expect(page.getByTestId("message-input")).toHaveAttribute(
    "contenteditable",
    "true",
  );
});

test("manage channel can delete an owned stream", async ({ page }) => {
  const channelName = `delete-me-${Date.now()}`;

  await page.goto("/");
  await page.getByRole("button", { name: "Create a channel" }).click();
  await page.getByTestId("create-channel-name").fill(channelName);
  await page.getByTestId("create-channel-submit").click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);

  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-management-sheet")).toBeVisible();
  await page.getByTestId("channel-management-delete").click();
  await expect(
    page.getByTestId("channel-delete-confirmation-dialog"),
  ).toBeVisible();
  await page.getByTestId("channel-delete-confirm").click();

  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  await expect(page.getByTestId("stream-list")).not.toContainText(channelName);
});

test("canceling channel deletion keeps the owned stream", async ({ page }) => {
  const channelName = `keep-me-${Date.now()}`;

  await page.goto("/");
  await page.getByRole("button", { name: "Create a channel" }).click();
  await page.getByTestId("create-channel-name").fill(channelName);
  await page.getByTestId("create-channel-submit").click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);

  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-management-sheet")).toBeVisible();
  await page.getByTestId("channel-management-delete").click();
  await expect(
    page.getByTestId("channel-delete-confirmation-dialog"),
  ).toBeVisible();
  await page.getByTestId("channel-delete-cancel").click();

  await expect(
    page.getByTestId("channel-delete-confirmation-dialog"),
  ).not.toBeVisible();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  await expect(page.getByTestId("stream-list")).toContainText(channelName);
});
