import { expect, test } from "@playwright/test";

import { KIND_TYPING_INDICATOR } from "../../src/shared/constants/kinds";
import {
  TEST_IDENTITIES,
  installMockBridge,
  openChannelBrowser,
  openCreateChannelDialog,
  openNewMessagePage,
} from "../helpers/bridge";

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const AGENTS_CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301";
const MOCK_IDENTITY_PUBKEY = "deadbeef".repeat(8);
// Relay-only agent owned by the mock viewer (see e2eBridge.ts
// OWNED_RELAY_AGENT_PUBKEY). Classified as a bot via mockRelayAgents and
// owned-by-viewer via its mockProfiles owner_pubkey, so the sidebar
// view-activity gate (memberIsBot && viewerIsOwner) opens for it.
const OWNED_RELAY_AGENT_PUBKEY =
  "a1b2c3d4e5f60718293a4b5c6d7e8f90112233445566778899aabbccddeeff00";
const DM_RELAY_AGENT_PUBKEY =
  "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

type MockFeedWindow = Window & {
  __BUZZ_E2E_SEED_ACTIVE_TURNS__?: (input: {
    agentPubkey: string;
    channelId: string;
    turnId: string;
    kind?: "turn_started" | "turn_completed";
  }) => void;
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
};

async function hasOutgoingEventWithContent(
  page: import("@playwright/test").Page,
  content: string,
) {
  return page.evaluate((expectedContent) => {
    return (
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: Array<{
            command: string;
            payload: unknown;
          }>;
        }
      ).__BUZZ_E2E_COMMAND_LOG__?.some((entry) => {
        if (entry.command !== "plugin:websocket|send") {
          return false;
        }

        const data = (
          entry.payload as {
            message?: { data?: string; type?: string };
          }
        )?.message?.data;
        if (!data) {
          return false;
        }

        try {
          const frame = JSON.parse(data) as unknown[];
          const event = frame[1] as { content?: unknown } | undefined;
          return frame[0] === "EVENT" && event?.content === expectedContent;
        } catch {
          return false;
        }
      }) ?? false
    );
  }, content);
}

async function readOutgoingChannelId(
  page: import("@playwright/test").Page,
  content: string,
) {
  return page.evaluate((expectedContent) => {
    const entries =
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: Array<{
            command: string;
            payload: unknown;
          }>;
        }
      ).__BUZZ_E2E_COMMAND_LOG__ ?? [];

    for (const entry of entries) {
      if (entry.command !== "plugin:websocket|send") {
        continue;
      }

      const data = (
        entry.payload as { message?: { data?: string } } | undefined
      )?.message?.data;
      if (!data) {
        continue;
      }

      try {
        const frame = JSON.parse(data) as [
          string,
          { content?: string; tags?: string[][] },
        ];
        if (
          frame[0] !== "EVENT" ||
          !frame[1]?.content?.includes(expectedContent)
        ) {
          continue;
        }
        return frame[1].tags?.find((tag) => tag[0] === "h")?.[1] ?? null;
      } catch {}
    }

    return null;
  }, content);
}

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
  await page.getByTestId("auxiliary-panel-close").click();
  await expect(page.getByTestId("channel-management-sheet")).not.toBeVisible();
}

async function openChannelEditDialog(page: import("@playwright/test").Page) {
  await page.getByTestId("channel-management-edit").click();
  await expect(
    page.getByRole("dialog", { name: "Edit channel" }),
  ).toBeVisible();
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

async function readMembersTriggerCount(page: import("@playwright/test").Page) {
  const label =
    (await page
      .getByTestId("channel-members-trigger")
      .getAttribute("aria-label")) ?? "";
  const match = label.match(/\((\d+)\)$/);
  if (!match) {
    throw new Error(`Could not read member count from label: ${label}`);
  }
  return Number(match[1]);
}

async function expectMembersTriggerCount(
  page: import("@playwright/test").Page,
  count: number,
) {
  await expect(page.getByTestId("channel-members-trigger")).toHaveAttribute(
    "aria-label",
    `View channel members (${count})`,
  );
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
                __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                  channelName: string;
                  kind?: number;
                }) => boolean;
              }
            ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
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
): Promise<string> {
  await page.getByTestId(`channel-${channelName}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  const channelId = await page
    .getByTestId(`channel-${channelName}`)
    .getAttribute("data-channel-id");
  if (!channelId) {
    throw new Error(`Channel ${channelName} is missing a data-channel-id.`);
  }

  await page.waitForFunction(() => {
    return Boolean(
      (
        window as Window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__,
    );
  });
  return page.evaluate(
    async ({ agentName, channelId }) => {
      const invoke = (
        window as Window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<{ agent?: { pubkey: string } }>;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      if (!invoke) {
        throw new Error("Mock bridge is not installed.");
      }

      const created = await invoke("create_managed_agent", {
        input: {
          name: agentName,
          spawnAfterCreate: true,
          systemPrompt: "Watch the channel and help when asked.",
        },
      });
      const pubkey = created.agent?.pubkey;
      if (!pubkey) {
        throw new Error("Mock managed agent creation did not return a pubkey.");
      }

      await invoke("add_channel_members", {
        channelId,
        pubkeys: [pubkey],
        role: "bot",
      });

      await (
        window as Window & {
          __BUZZ_E2E_QUERY_CLIENT__?: {
            invalidateQueries: () => Promise<void>;
          };
        }
      ).__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries();

      return pubkey;
    },
    { agentName, channelId },
  );
}

async function readCommandLog(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    return (
      (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
        .__BUZZ_E2E_COMMANDS__ ?? []
    );
  });
}

function commandCount(commands: string[], command: string) {
  return commands.filter((entry) => entry === command).length;
}

async function readCommandPayloadLog(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    return (
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: Array<{
            command: string;
            payload: unknown;
          }>;
        }
      ).__BUZZ_E2E_COMMAND_LOG__ ?? []
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
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__,
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
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<unknown>;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;

      if (!invoke) {
        throw new Error("Mock bridge is not installed.");
      }

      return invoke(command, payload);
    },
    { command, payload },
  ) as Promise<T>;
}

async function expectSameLeftInset(
  page: import("@playwright/test").Page,
  firstTestId: string,
  secondTestId: string,
) {
  const firstBox = await page.getByTestId(firstTestId).boundingBox();
  const secondBox = await page.getByTestId(secondTestId).first().boundingBox();

  if (!firstBox || !secondBox) {
    throw new Error(`Could not measure ${firstTestId} against ${secondTestId}`);
  }

  expect(Math.abs(firstBox.x - secondBox.x)).toBeLessThanOrEqual(4);
}

async function expectIntroSpacedAboveDayDivider(
  page: import("@playwright/test").Page,
  introTestId: string,
) {
  const introBox = await page.getByTestId(introTestId).boundingBox();
  const dividerBox = await page
    .getByTestId("message-timeline-day-divider")
    .first()
    .boundingBox();
  const messageBox = await page
    .getByTestId("message-row")
    .first()
    .boundingBox();

  if (!introBox || !dividerBox || !messageBox) {
    throw new Error(`Could not measure timeline spacing for ${introTestId}`);
  }

  const gapAboveDivider = dividerBox.y - (introBox.y + introBox.height);
  const gapBelowDivider = messageBox.y - (dividerBox.y + dividerBox.height);

  // The intro is a flex sibling above the timeline; the day divider and first
  // message-row are virtualized items positioned by translateY inside the
  // scroll container. The intro -> divider gap is the wrapper flex spacing the
  // layout controls (8px, stable), so guard THAT with a tight band — a layout
  // regression that collapses or balloons it fails here. The divider -> message
  // gap is NOT a layout-spacing contract: virtualized rows are positioned
  // back-to-back (no inter-item gap), so it is ~0 by construction plus
  // MessageRow avatar/font render jitter, genuinely variable run-to-run. Assert
  // only non-overlap on it (reading order: intro, divider, then message).
  expect(Math.abs(gapAboveDivider - 8)).toBeLessThanOrEqual(2);
  expect(gapBelowDivider).toBeGreaterThanOrEqual(0);
}

async function expectIntroActionCardLayout(
  page: import("@playwright/test").Page,
  actionTestId: string,
) {
  const actionBox = await page.getByTestId(actionTestId).boundingBox();
  const iconBox = await page.getByTestId(`${actionTestId}-icon`).boundingBox();

  if (!actionBox || !iconBox) {
    throw new Error(`Could not measure intro action card: ${actionTestId}`);
  }

  expect(actionBox.height).toBeGreaterThan(actionBox.width);
  expect(Math.round(actionBox.width)).toBe(220);
  expect(Math.round(iconBox.width)).toBe(48);
  expect(Math.round(iconBox.height)).toBe(48);
  const introIconRadius = await page
    .getByTestId("message-channel-intro-icon")
    .evaluate((element) => window.getComputedStyle(element).borderRadius);
  const actionRadius = await page
    .getByTestId(actionTestId)
    .evaluate((element) => window.getComputedStyle(element).borderRadius);
  expect(actionRadius).toBe(introIconRadius);
  await expect(page.getByTestId(`${actionTestId}-title`)).toHaveCSS(
    "white-space",
    "normal",
  );
  await expect(page.getByTestId(`${actionTestId}-description`)).toHaveCSS(
    "white-space",
    "normal",
  );
}

async function expectIntroActionsShareRow(
  page: import("@playwright/test").Page,
  actionTestIds: string[],
) {
  const boxes = await Promise.all(
    actionTestIds.map((testId) => page.getByTestId(testId).boundingBox()),
  );
  const measuredBoxes = boxes.filter(
    (box): box is NonNullable<typeof box> => box !== null,
  );
  const [firstBox] = measuredBoxes;
  if (!firstBox || measuredBoxes.length !== actionTestIds.length) {
    throw new Error("Could not measure intro action row");
  }

  for (const box of measuredBoxes.slice(1)) {
    expect(Math.abs(firstBox.y - box.y)).toBeLessThanOrEqual(1);
  }
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("sidebar shows all channel types", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await expect(page.getByTestId("sidebar-agents-count")).toHaveCount(0);

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

  await openNewMessagePage(page);
  await expect(page.getByTestId("new-message-page")).toBeVisible();
  await expect(page.getByTestId("message-composer")).toBeVisible();
  await expect(page.getByTestId("new-message-recipient-popover")).toBeVisible();
  await expect(page.getByTestId("new-message-body")).toBeEmpty();

  await page.getByTestId("new-dm-search").fill("charlie");
  await expect(
    page.getByTestId(`new-dm-result-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(
    page.getByTestId(`new-dm-selected-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toBeVisible();
  await expect(page.getByTestId("new-dm-search")).toHaveValue("");
  const selectedCharlie = page.getByTestId(
    `new-dm-selected-${TEST_IDENTITIES.charlie.pubkey}`,
  );
  await selectedCharlie.hover();
  const selectedCharlieBox = await selectedCharlie.boundingBox();
  expect(selectedCharlieBox).not.toBeNull();
  if (!selectedCharlieBox) return;
  await page.mouse.move(
    selectedCharlieBox.x + selectedCharlieBox.width / 2,
    selectedCharlieBox.y + selectedCharlieBox.height / 2,
  );
  await page.mouse.down();
  await expect(page.locator(".buzz-poof-burst")).toHaveCount(1);
  await page.mouse.up();
  await expect(selectedCharlie).not.toBeVisible();
  await page.getByTestId("new-dm-search").fill("charlie");
  await expect(
    page.getByTestId(`new-dm-result-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toBeVisible();
  await page
    .getByTestId(`new-dm-result-${TEST_IDENTITIES.charlie.pubkey}`)
    .click();
  await expect(page.getByTestId("new-dm-search")).toHaveValue("");
  await expect(
    page.getByTestId(`new-dm-selected-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toBeVisible();

  await page.getByTestId("message-input").fill("Hello charlie");
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("dm-list")).toContainText("charlie");
  await expect(page.getByTestId("chat-title")).toHaveText("charlie");
  await expect(page.getByTestId("section-actions-dms")).not.toBeFocused();
});

test("keeps typing focus while arrow keys traverse and select DM recipients", async ({
  page,
}) => {
  await page.goto("/");
  await openNewMessagePage(page);

  const search = page.getByTestId("new-dm-search");
  await expect(search).toBeFocused();
  const initialActiveDescendant = await search.getAttribute(
    "aria-activedescendant",
  );
  expect(initialActiveDescendant).toMatch(/^new-dm-option-/);
  await search.press("Enter");
  await expect(
    page.locator("button[data-testid^='new-dm-selected-']"),
  ).toHaveCount(1);
  await expect(page.getByTestId("new-message-recipient-popover")).toBeVisible();

  await search.fill("o");
  await expect(
    page.getByTestId(`new-dm-result-${TEST_IDENTITIES.bob.pubkey}`),
  ).toBeVisible();
  await expect(
    page.getByTestId(`new-dm-result-${TEST_IDENTITIES.outsider.pubkey}`),
  ).toBeVisible();

  await expect(search).toHaveAttribute(
    "aria-activedescendant",
    `new-dm-option-${TEST_IDENTITIES.outsider.pubkey}`,
  );
  await expect(
    page.locator(`#new-dm-option-${TEST_IDENTITIES.outsider.pubkey}`),
  ).toHaveAttribute("aria-selected", "true");

  await search.press("ArrowDown");
  await expect(search).toBeFocused();
  await expect(search).toHaveAttribute(
    "aria-activedescendant",
    `new-dm-option-${TEST_IDENTITIES.bob.pubkey}`,
  );

  await search.press("ArrowUp");
  await expect(search).toHaveAttribute(
    "aria-activedescendant",
    `new-dm-option-${TEST_IDENTITIES.outsider.pubkey}`,
  );
  await search.press("Enter");

  await expect(search).toBeFocused();
  await expect(search).toHaveValue("");
  await expect(search).toHaveAttribute("aria-activedescendant", /.+/);
  await expect(
    page.getByTestId(`new-dm-selected-${TEST_IDENTITIES.outsider.pubkey}`),
  ).toBeVisible();
  await expect(page.getByTestId("new-message-recipient-popover")).toBeVisible();
});

test("sends the first message from the new direct message composer", async ({
  page,
}) => {
  await page.goto("/");
  await openNewMessagePage(page);

  await page.getByTestId("new-dm-search").fill("charlie");
  await page
    .getByTestId(`new-dm-result-${TEST_IDENTITIES.charlie.pubkey}`)
    .click();

  const message = "First message from the new conversation";
  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("chat-title")).toHaveText("charlie");
  await expect(page.getByTestId("message-timeline")).toContainText(message);
});

test("creates the DM before preparing a persona mention", async ({ page }) => {
  await installMockBridge(page, {
    activePersonaIds: ["builtin:fizz"],
    createManagedAgentDelayMs: 1_000,
  });
  await page.goto("/");
  await openNewMessagePage(page);

  await page.getByTestId("new-dm-search").fill("charlie");
  await page
    .getByTestId(`new-dm-result-${TEST_IDENTITIES.charlie.pubkey}`)
    .click();

  const input = page.getByTestId("message-input");
  await input.fill("Ask @fi");
  await expect(
    page
      .getByTestId("message-composer")
      .getByTestId("mention-autocomplete")
      .locator("button", { hasText: "Fizz" }),
  ).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" for a hand");

  const baselineCommands = await readCommandLog(page);
  const baselineCommandPayloads = await readCommandPayloadLog(page);
  const baselineOpenDmCount = commandCount(baselineCommands, "open_dm");
  const baselineCreateCount = commandCount(
    baselineCommands,
    "create_managed_agent",
  );

  await page.getByTestId("send-message").click();

  await expect
    .poll(async () => commandCount(await readCommandLog(page), "open_dm"))
    .toBeGreaterThan(baselineOpenDmCount);
  await expect(
    page.getByTestId(`new-dm-selected-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toBeDisabled();
  await expect(page.getByTestId("new-dm-search")).toBeDisabled();
  await expect(page.getByTestId("new-message-recipient-popover")).toBeHidden();
  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "create_managed_agent"),
    )
    .toBeGreaterThan(baselineCreateCount);
  await expect(page.getByTestId("chat-title")).toContainText("charlie");
  await expect(page.getByTestId("chat-title")).toContainText("Fizz");

  const sendCommands = (await readCommandLog(page)).slice(
    baselineCommands.length,
  );
  const sendCommandPayloads = (await readCommandPayloadLog(page)).slice(
    baselineCommandPayloads.length,
  );
  expect(commandCount(sendCommands, "open_dm")).toBe(2);
  const firstOpenIndex = sendCommands.indexOf("open_dm");
  const createIndex = sendCommands.indexOf("create_managed_agent");
  const expandedOpenIndex = sendCommands.lastIndexOf("open_dm");
  const startIndex = sendCommands.indexOf("start_managed_agent");
  expect(firstOpenIndex).toBeLessThan(createIndex);
  expect(createIndex).toBeLessThan(expandedOpenIndex);
  expect(expandedOpenIndex).toBeLessThan(startIndex);
  expect(sendCommands).not.toContain("add_channel_members");

  const sentMessageCommand = sendCommandPayloads.find((entry) => {
    if (entry.command !== "plugin:websocket|send") {
      return false;
    }
    const data = (entry.payload as { message?: { data?: string } } | undefined)
      ?.message?.data;
    if (!data) {
      return false;
    }
    const frame = JSON.parse(data) as unknown[];
    return (
      frame[0] === "EVENT" &&
      (frame[1] as { content?: string } | undefined)?.content.includes(
        "for a hand",
      )
    );
  });
  const sentMessageData = (
    sentMessageCommand?.payload as { message?: { data?: string } } | undefined
  )?.message?.data;
  expect(sentMessageData).toBeTruthy();
  const sentMessageEvent = (
    JSON.parse(sentMessageData ?? "[]") as [string, { tags?: string[][] }]
  )[1];
  const sentChannelId = sentMessageEvent.tags?.find(
    (tag) => tag[0] === "h",
  )?.[1];
  expect(sentChannelId).toBeTruthy();
  await expect(
    page.locator("[data-active='true'][data-channel-id]"),
  ).toHaveAttribute("data-channel-id", sentChannelId ?? "");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "for a hand",
  );
  await expect(
    page
      .getByTestId("message-row")
      .last()
      .locator("[data-mention].agent-mention-highlight", { hasText: "Fizz" }),
  ).toBeVisible();
});

test("routes an agent mention from an existing DM to the expanded conversation", async ({
  page,
}) => {
  await installMockBridge(page, {
    activePersonaIds: ["builtin:fizz"],
  });
  await page.goto("/");

  const sourceDm = page.getByTestId("channel-alice-tyler");
  const sourceDmId = await sourceDm.getAttribute("data-channel-id");
  expect(sourceDmId).toBeTruthy();
  await sourceDm.click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");

  const messageTail = "in this DM";
  const input = page.getByTestId("message-input");
  await input.fill("Ask @fi");
  await expect(
    page
      .getByTestId("message-composer")
      .getByTestId("mention-autocomplete")
      .locator("button", { hasText: "Fizz" }),
  ).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" in this DM");
  const baselineCommands = await readCommandPayloadLog(page);
  await page.getByTestId("send-message").click();

  await expect
    .poll(async () => readOutgoingChannelId(page, messageTail))
    .not.toBeNull();
  const sentChannelId = await readOutgoingChannelId(page, messageTail);
  expect(sentChannelId).not.toBe(sourceDmId);
  await expect(
    page.locator("[data-active='true'][data-channel-id]"),
  ).toHaveAttribute("data-channel-id", sentChannelId ?? "");
  await expect(page.getByTestId("chat-title")).toContainText("alice");
  await expect(page.getByTestId("chat-title")).toContainText("Fizz");
  await expect(sourceDm).not.toContainText("Fizz");
  const sendCommands = (await readCommandPayloadLog(page)).slice(
    baselineCommands.length,
  );
  expect(sendCommands).not.toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        command: "add_channel_members",
        payload: expect.objectContaining({ channelId: sourceDmId }),
      }),
    ]),
  );
  expect(sendCommands.map((entry) => entry.command)).toEqual(
    expect.arrayContaining(["open_dm", "start_managed_agent"]),
  );
});

test("routes a relay-agent mention from an existing DM to the expanded conversation", async ({
  page,
}) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: DM_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: [MOCK_IDENTITY_PUBKEY],
      },
    ],
  });
  await page.goto("/");

  const sourceDm = page.getByTestId("channel-alice-tyler");
  const sourceDmId = await sourceDm.getAttribute("data-channel-id");
  expect(sourceDmId).toBeTruthy();
  await sourceDm.click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");

  const messageTail = "with the relay agent";
  const input = page.getByTestId("message-input");
  await input.fill("Ask @qui");
  await expect(
    page
      .getByTestId("message-composer")
      .getByTestId("mention-autocomplete")
      .locator("button", { hasText: "quinn" }),
  ).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(` ${messageTail}`);
  const baselineCommands = await readCommandPayloadLog(page);
  await page.getByTestId("send-message").click();

  await expect
    .poll(async () => readOutgoingChannelId(page, messageTail))
    .not.toBeNull();
  const sentChannelId = await readOutgoingChannelId(page, messageTail);
  expect(sentChannelId).not.toBe(sourceDmId);
  await expect(
    page.locator("[data-active='true'][data-channel-id]"),
  ).toHaveAttribute("data-channel-id", sentChannelId ?? "");
  await expect(page.getByTestId("chat-title")).toContainText("alice");
  await expect(page.getByTestId("chat-title")).toContainText(
    DM_RELAY_AGENT_PUBKEY.slice(0, 8),
  );

  const sendCommands = (await readCommandPayloadLog(page)).slice(
    baselineCommands.length,
  );
  expect(sendCommands.map((entry) => entry.command)).toContain("open_dm");
  expect(sendCommands.map((entry) => entry.command)).not.toContain(
    "start_managed_agent",
  );
  expect(sendCommands.map((entry) => entry.command)).not.toContain(
    "add_channel_members",
  );
});

test("does not reroute an expanded DM after the user navigates away", async ({
  page,
}) => {
  await installMockBridge(page, {
    activePersonaIds: ["builtin:fizz"],
    sendMessageDelayMs: 1_000,
  });
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");

  const input = page.getByTestId("message-input");
  await input.fill("Ask @fi");
  await expect(
    page
      .getByTestId("message-composer")
      .getByTestId("mention-autocomplete")
      .locator("button", { hasText: "Fizz" }),
  ).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" while I leave");
  await page.getByTestId("send-message").click();

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.waitForTimeout(1_250);
  await expect(page).toHaveURL(
    new RegExp(`/channels/${GENERAL_CHANNEL_ID}(?:\\?|$)`),
  );
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("does not reroute an expanded DM after the channel pane unmounts", async ({
  page,
}) => {
  await installMockBridge(page, {
    activePersonaIds: ["builtin:fizz"],
    openDmDelayMs: 1_000,
  });
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");

  const input = page.getByTestId("message-input");
  await input.fill("Ask @fi");
  await expect(
    page
      .getByTestId("message-composer")
      .getByTestId("mention-autocomplete")
      .locator("button", { hasText: "Fizz" }),
  ).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" while I open settings");
  await page.getByTestId("send-message").click();

  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await expect(page.getByTestId("settings-view")).toBeVisible();
  await page.waitForTimeout(1_250);
  await expect(page.getByTestId("settings-view")).toBeVisible();
  await expect
    .poll(async () => readOutgoingChannelId(page, "while I open settings"))
    .toBeNull();
});

test("drops an expanded DM after the first message fails", async ({ page }) => {
  const retryMessage = "Retry without the agent";
  const sendError = "Mock first DM send failed.";
  await installMockBridge(page, {
    activePersonaIds: ["builtin:fizz"],
    sendMessageErrors: [sendError],
  });
  await page.goto("/");
  await openNewMessagePage(page);

  await page.getByTestId("new-dm-search").fill("charlie");
  await page
    .getByTestId(`new-dm-result-${TEST_IDENTITIES.charlie.pubkey}`)
    .click();

  const input = page.getByTestId("message-input");
  await input.fill("Ask @fi");
  await expect(
    page
      .getByTestId("message-composer")
      .getByTestId("mention-autocomplete")
      .locator("button", { hasText: "Fizz" }),
  ).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" for a hand");
  await page.getByTestId("send-message").click();

  await expect(page.getByText(sendError)).toBeVisible();
  await expect(input).toContainText("Fizz");

  const commandsAfterFailure = await readCommandPayloadLog(page);
  const failedSendChannelId = await readOutgoingChannelId(page, "for a hand");
  expect(failedSendChannelId).toBeTruthy();
  expect(commandsAfterFailure.map((entry) => entry.command)).not.toContain(
    "add_channel_members",
  );
  const baselineOpenDmCount = commandCount(
    commandsAfterFailure.map((entry) => entry.command),
    "open_dm",
  );

  await input.fill(retryMessage);
  const retryBaseline = commandsAfterFailure.length;
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("chat-title")).toHaveText("charlie");
  await expect(page.getByTestId("message-timeline")).toContainText(
    retryMessage,
  );

  const allCommands = await readCommandPayloadLog(page);
  expect(
    commandCount(
      allCommands.map((entry) => entry.command),
      "open_dm",
    ),
  ).toBe(baselineOpenDmCount + 1);
  const retryCommands = allCommands.slice(retryBaseline);
  const retrySend = retryCommands.find((entry) => {
    if (entry.command !== "plugin:websocket|send") {
      return false;
    }
    const data = (entry.payload as { message?: { data?: string } } | undefined)
      ?.message?.data;
    if (!data) {
      return false;
    }
    const frame = JSON.parse(data) as unknown[];
    return (
      frame[0] === "EVENT" &&
      (frame[1] as { content?: string } | undefined)?.content === retryMessage
    );
  });
  const retrySendData = (
    retrySend?.payload as { message?: { data?: string } } | undefined
  )?.message?.data;
  expect(retrySendData).toBeTruthy();
  const retryEvent = (
    JSON.parse(retrySendData ?? "[]") as [string, { tags?: string[][] }]
  )[1];
  const retryChannelId = retryEvent.tags?.find((tag) => tag[0] === "h")?.[1];
  expect(retryChannelId).toBeTruthy();
  expect(retryChannelId).not.toBe(failedSendChannelId);
  await expect(
    page.locator("[data-active='true'][data-channel-id]"),
  ).toHaveAttribute("data-channel-id", retryChannelId ?? "");
});

test("drops an expanded DM after agent startup fails", async ({ page }) => {
  const retryMessage = "Retry after agent startup failed";
  const startError = "Mock agent startup failed.";
  await installMockBridge(page, {
    activePersonaIds: ["builtin:fizz"],
    startManagedAgentErrors: [startError],
  });
  await page.goto("/");
  await openNewMessagePage(page);

  await page.getByTestId("new-dm-search").fill("charlie");
  await page
    .getByTestId(`new-dm-result-${TEST_IDENTITIES.charlie.pubkey}`)
    .click();

  const input = page.getByTestId("message-input");
  await input.fill("Ask @fi");
  await expect(
    page
      .getByTestId("message-composer")
      .getByTestId("mention-autocomplete")
      .locator("button", { hasText: "Fizz" }),
  ).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" before startup fails");
  await page.getByTestId("send-message").click();

  await expect(
    page.getByText(startError, { exact: false }).first(),
  ).toBeVisible();
  await expect(input).toContainText("Fizz");

  const commandsAfterFailure = await readCommandPayloadLog(page);
  const openDmCallsAfterFailure = commandsAfterFailure.filter(
    (entry) => entry.command === "open_dm",
  );
  expect(openDmCallsAfterFailure).toHaveLength(2);
  expect(
    (openDmCallsAfterFailure.at(-1)?.payload as { pubkeys?: string[] })
      ?.pubkeys,
  ).toEqual(expect.arrayContaining([TEST_IDENTITIES.charlie.pubkey]));
  expect(
    (openDmCallsAfterFailure.at(-1)?.payload as { pubkeys?: string[] })
      ?.pubkeys,
  ).toHaveLength(2);

  await input.fill(retryMessage);
  const retryBaseline = commandsAfterFailure.length;
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("chat-title")).toHaveText("charlie");
  await expect(page.getByTestId("message-timeline")).toContainText(
    retryMessage,
  );

  const retryCommands = (await readCommandPayloadLog(page)).slice(
    retryBaseline,
  );
  const retryOpenDm = retryCommands.find(
    (entry) => entry.command === "open_dm",
  );
  expect(
    (retryOpenDm?.payload as { pubkeys?: string[] } | undefined)?.pubkeys,
  ).toEqual([TEST_IDENTITIES.charlie.pubkey]);
  await expect(page.getByTestId("chat-title")).not.toContainText("Fizz");
});

test("closes direct message results while opening", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => {
    const testWindow = window as Window & {
      __BUZZ_E2E__?: { mock?: { openDmDelayMs?: number } };
    };
    testWindow.__BUZZ_E2E__ ??= {};
    testWindow.__BUZZ_E2E__.mock ??= {};
    testWindow.__BUZZ_E2E__.mock.openDmDelayMs = 1_000;
  });

  await openNewMessagePage(page);
  await page.getByTestId("new-dm-search").fill("charlie");
  await page
    .getByTestId(`new-dm-result-${TEST_IDENTITIES.charlie.pubkey}`)
    .click();
  await expect(
    page.getByTestId(`new-dm-selected-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toBeVisible();
  await expect(page.getByTestId("new-message-recipient-popover")).toBeVisible();

  await page.getByTestId("message-input").fill("Open this conversation");
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("new-dm-opening")).toContainText("Opening");
  await expect(page.getByTestId("new-message-recipient-popover")).toBeHidden();

  await expect(page.getByTestId("chat-title")).toHaveText("charlie");
});

test("does not reopen a direct message after leaving the composer", async ({
  page,
}) => {
  await installMockBridge(page, { openDmDelayMs: 1_000 });
  await page.goto("/");
  await openNewMessagePage(page);

  await page.getByTestId("new-dm-search").fill("charlie");
  await page
    .getByTestId(`new-dm-result-${TEST_IDENTITIES.charlie.pubkey}`)
    .click();
  const staleMessage = "Do not send after leaving";
  await page.getByTestId("message-input").fill(staleMessage);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("new-dm-opening")).toContainText("Opening");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("dm-list")).toContainText("charlie");
  // The DM appearing proves the delayed open completed. Give the abandoned
  // continuation time to run before asserting that it did not publish.
  await page.waitForTimeout(1_250);
  expect(await hasOutgoingEventWithContent(page, staleMessage)).toBe(false);
  await expect(page).toHaveURL(
    new RegExp(`/channels/${GENERAL_CHANNEL_ID}(?:\\?|$)`),
  );
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("does not reopen a sent direct message after leaving during cache reseed", async ({
  page,
}) => {
  await page.goto("/");
  await openNewMessagePage(page);

  await page.getByTestId("new-dm-search").fill("charlie");
  await page
    .getByTestId(`new-dm-result-${TEST_IDENTITIES.charlie.pubkey}`)
    .click();
  const staleMessage = "Stay on the channel after cache reseed";
  await page.getByTestId("message-input").fill(staleMessage);
  await page.evaluate(() => {
    const testWindow = window as Window & {
      __BUZZ_E2E__?: { mock?: { channelsReadDelayMs?: number } };
    };
    testWindow.__BUZZ_E2E__ ??= {};
    testWindow.__BUZZ_E2E__.mock ??= {};
    testWindow.__BUZZ_E2E__.mock.channelsReadDelayMs = 1_000;
  });

  await page.getByTestId("send-message").click();
  await expect
    .poll(async () => hasOutgoingEventWithContent(page, staleMessage))
    .toBe(true);

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.waitForTimeout(1_250);
  await expect(page).toHaveURL(
    new RegExp(`/channels/${GENERAL_CHANNEL_ID}(?:\\?|$)`),
  );
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("shows capped participant stack in group direct message header", async ({
  page,
}) => {
  await page.goto("/");

  await openNewMessagePage(page);
  await expect(page.getByTestId("new-message-page")).toBeVisible();

  for (const identity of [
    TEST_IDENTITIES.alice,
    TEST_IDENTITIES.bob,
    TEST_IDENTITIES.charlie,
    TEST_IDENTITIES.outsider,
  ]) {
    await page.getByTestId("new-dm-search").fill(identity.username);
    await page.getByTestId(`new-dm-result-${identity.pubkey}`).click();
  }

  await page.getByTestId("message-input").fill("Hello group");
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("channel-dm-count-Group DM (5)")).toHaveText(
    "4",
  );
  await expect(page.getByTestId("chat-title")).toContainText("alice");
  await expect(page.getByTestId("chat-title")).toContainText("bob");
  await expect(page.getByTestId("chat-title")).toContainText("charlie");
  await expect(page.getByTestId("chat-title")).toContainText("+1 more");
  await expect(page.getByTestId("chat-title")).not.toContainText("outsider");
  const chatTitle = await page.getByTestId("chat-title").innerText();
  await expect(
    page.getByTestId("message-input").locator("[data-placeholder]").first(),
  ).toHaveAttribute("data-placeholder", `Message ${chatTitle}`);
  const composerColors = await page
    .getByTestId("message-input")
    .evaluate((element) => {
      const placeholderElement =
        element.querySelector<HTMLElement>("[data-placeholder]");
      if (!placeholderElement) {
        return null;
      }

      return {
        placeholderColor: window.getComputedStyle(
          placeholderElement,
          "::before",
        ).color,
        textColor: window.getComputedStyle(element).color,
      };
    });
  expect(composerColors).not.toBeNull();
  expect(composerColors?.placeholderColor).not.toBe(composerColors?.textColor);
  await expect(page.getByTestId("chat-header-dm-avatar")).toHaveCount(0);
  await expect(page.getByTestId("chat-header-dm-avatar-stack")).toBeVisible();
  await expect(page.getByTestId("chat-presence-badge")).toHaveCount(0);
  await expect(
    page.getByTestId("chat-header-dm-avatar-stack-participant"),
  ).toHaveCount(3);
  await expect(page.getByTestId("chat-header-dm-avatar-stack-more")).toHaveText(
    "+1",
  );
  const headerStackBox = await page
    .getByTestId("chat-header-dm-avatar-stack")
    .boundingBox();
  const headerTitleBox = await page.getByTestId("chat-title").boundingBox();
  expect(headerStackBox).not.toBeNull();
  expect(headerTitleBox).not.toBeNull();
  expect((headerStackBox?.x ?? 0) + (headerStackBox?.width ?? 0)).toBeLessThan(
    headerTitleBox?.x ?? 0,
  );
  await expect(page.getByTestId("message-dm-intro")).toContainText("alice");
  await expect(page.getByTestId("message-dm-intro")).toContainText("bob");
  await expect(page.getByTestId("message-dm-intro")).toContainText("charlie");
  await expect(page.getByTestId("message-dm-intro")).toContainText("+1 more");
  await expect(page.getByTestId("message-dm-intro")).not.toContainText(
    "outsider",
  );
  await expect(page.getByTestId("message-dm-intro-avatar-stack")).toBeVisible();
  await expect(
    page.getByTestId("message-dm-intro-avatar-stack-participant"),
  ).toHaveCount(3);
  await expect(
    page.getByTestId("message-dm-intro-avatar-stack-more"),
  ).toHaveText("+1");
});

test("create stream with name and description", async ({ page }) => {
  const channelName = `my-new-stream-${Date.now()}`;

  await page.goto("/");
  await openCreateChannelDialog(page);
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
  await openCreateChannelDialog(page);
  await page.getByTestId("create-channel-name").fill(channelName);
  await page
    .getByTestId("create-channel-description")
    .fill("Auto-cleaned test stream");
  await page.getByRole("button", { name: "Channel duration: Ongoing" }).click();
  await page
    .getByLabel("Ephemeral - auto-archives after 7 days of inactivity")
    .click();
  await page.getByTestId("create-channel-submit").click();

  await expect(page.getByTestId("stream-list")).toContainText(channelName);
  await expect(page.getByTestId("chat-title")).toContainText(channelName);
  await expect(
    page.getByTestId(`channel-ephemeral-${channelName}`),
  ).toBeVisible();
  await expect(page.getByTestId("chat-ephemeral-badge")).toBeVisible();
  await expect(page.getByTestId("chat-ephemeral-badge")).toHaveAttribute(
    "aria-label",
    /Ephemeral channel\. Cleans up in 7 days\./,
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
  const shiftedTime = new Date("2026-04-15T02:00:00.000Z");

  await page.clock.setFixedTime(initialTime);
  await page.goto("/");

  for (const channelName of [firstChannelName, secondChannelName]) {
    await openCreateChannelDialog(page);
    await page.getByTestId("create-channel-name").fill(channelName);
    await page
      .getByTestId("create-channel-description")
      .fill("Auto-cleaned test stream");
    await page
      .getByRole("button", { name: "Channel duration: Ongoing" })
      .click();
    await page
      .getByLabel("Ephemeral - auto-archives after 7 days of inactivity")
      .click();
    await page.getByTestId("create-channel-submit").click();
    await expect(page.getByTestId("chat-title")).toContainText(channelName);
  }

  await page.clock.setFixedTime(shiftedTime);
  await expect
    .poll(() => page.evaluate(() => Date.now()))
    .toBe(shiftedTime.getTime());

  await page.getByTestId(`channel-${firstChannelName}`).click();
  await expect(page.getByTestId("chat-title")).toContainText(firstChannelName);
  await expect(page.getByTestId("chat-ephemeral-badge")).toHaveAttribute(
    "aria-label",
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
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__,
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
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<{ id: string }>;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;

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
  await openCreateChannelDialog(page);
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

test("empty channel shows intro actions", async ({ page }) => {
  await page.goto("/");

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(page.getByTestId("message-channel-intro")).toBeVisible();
  await expect(page.getByTestId("message-channel-intro")).toContainText(
    "This is the beginning of the regular channel.",
  );
  await expect(
    page.getByTestId("channel-intro-action-create-channel"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("channel-intro-action-create-agent"),
  ).toBeVisible();
  await expect(
    page.getByTestId("channel-intro-action-add-people"),
  ).toBeVisible();
  await expect(page.getByTestId("welcome-composer-guide-banner")).toHaveCount(
    0,
  );
  await expectIntroActionCardLayout(page, "channel-intro-action-create-agent");
  await expectIntroActionsShareRow(page, [
    "channel-intro-action-create-agent",
    "channel-intro-action-add-people",
  ]);

  await page.getByTestId("channel-intro-action-add-people").click();
  await expect(page.getByTestId("members-sidebar")).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.getByTestId("members-sidebar")).not.toBeVisible();

  await page.getByTestId("channel-intro-action-create-agent").click();
  await expect(page.getByRole("heading", { name: "Add agents" })).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.getByRole("heading", { name: "Add agents" })).toHaveCount(
    0,
  );
});

test("short channel with messages shows intro actions on open", async ({
  page,
}) => {
  const channelName = `short-intro-${Date.now()}`;
  const message = "Only message in a short channel";

  await page.goto("/");
  await openCreateChannelDialog(page);
  await page.getByTestId("create-channel-name").fill(channelName);
  await page.getByTestId("create-channel-submit").click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);

  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(message);

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.getByTestId(`channel-${channelName}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  await expect(page.getByTestId("message-timeline")).toContainText(message);
  await expect(page.getByTestId("message-channel-intro")).toBeVisible();
  await expect(page.getByTestId("message-channel-intro")).toContainText(
    "This is the beginning of the regular channel.",
  );
  await expect(
    page.getByTestId("channel-intro-action-create-agent"),
  ).toBeVisible();
  await expect(
    page.getByTestId("channel-intro-action-add-people"),
  ).toBeVisible();
});

test("scrollable channel with recent messages hides intro actions until top", async ({
  page,
}) => {
  const channelName = `long-intro-${Date.now()}`;
  const messages = Array.from(
    { length: 24 },
    (_, index) => `Scrollable channel message ${index + 1}`,
  );

  await page.goto("/");
  await openCreateChannelDialog(page);
  await page.getByTestId("create-channel-name").fill(channelName);
  await page.getByTestId("create-channel-submit").click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);

  for (const message of messages) {
    await page.getByTestId("message-input").fill(message);
    await page.getByTestId("send-message").click();
    await expect(page.getByTestId("message-timeline")).toContainText(message);
  }

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.getByTestId(`channel-${channelName}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  await expect(page.getByTestId("message-channel-intro")).not.toBeInViewport();
  await expect(
    page.getByTestId("channel-intro-action-create-agent"),
  ).not.toBeInViewport();
  await expect(
    page.getByTestId("channel-intro-action-add-people"),
  ).not.toBeInViewport();
  await expect(page.getByTestId("message-timeline")).toContainText(
    messages[messages.length - 1],
  );

  await page.getByTestId("message-timeline").evaluate((element) => {
    element.scrollTop = 0;
    element.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect(page.getByTestId("message-channel-intro")).toBeVisible();
  await expect(
    page.getByTestId("channel-intro-action-create-agent"),
  ).toBeVisible();
  await expect(
    page.getByTestId("channel-intro-action-add-people"),
  ).toBeVisible();
});

test("channel with messages shows content", async ({ page }) => {
  await page.goto("/");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-channel-intro")).toBeVisible();
  await expect(
    page.getByTestId("channel-intro-action-create-channel"),
  ).toHaveCount(0);
  await expect(page.getByTestId("welcome-composer-guide-banner")).toHaveCount(
    0,
  );
  await expect(page.getByTestId("message-timeline-day-divider")).toBeVisible();
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Welcome to #general",
  );
});

test("channel date divider keeps the date sticky while the separator rule scrolls", async ({
  page,
}) => {
  await page.goto("/");

  await page.getByTestId("channel-engineering").click();
  await expect(page.getByTestId("chat-title")).toHaveText("engineering");
  await waitForMockLiveSubscription(page, "engineering");

  await page.evaluate(() => {
    const firstDay = 1_700_000_000;
    for (let day = 0; day < 2; day += 1) {
      for (let index = 0; index < 14; index += 1) {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "engineering",
          content: `date handoff day ${day + 1} row ${index + 1}\nsecond line for scroll height`,
          createdAt: firstDay + day * 86_400 + index,
          pubkey:
            "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
        });
      }
    }
  });

  await expect(page.getByTestId("message-timeline-day-group")).toHaveCount(2);
  await expect(page.getByTestId("message-timeline-day-divider")).toHaveCount(2);

  const timeline = page.getByTestId("message-timeline");
  await timeline.evaluate((element) => {
    const firstGroup = element.querySelector<HTMLElement>(
      '[data-testid="message-timeline-day-group"]',
    );
    if (!firstGroup) {
      throw new Error("missing first day group");
    }
    const groupRect = firstGroup.getBoundingClientRect();
    const stickyTop = Number.parseFloat(
      getComputedStyle(
        firstGroup.querySelector<HTMLElement>(
          '[data-testid="message-timeline-day-divider"]',
        ) ?? firstGroup,
      ).top,
    );
    element.scrollTop +=
      groupRect.top - (element.getBoundingClientRect().top + stickyTop - 32);
    element.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(50);

  const metrics = await timeline.evaluate((element) => {
    const firstGroup = element.querySelector<HTMLElement>(
      '[data-testid="message-timeline-day-group"]',
    );
    const firstDivider = firstGroup?.querySelector<HTMLElement>(
      '[data-testid="message-timeline-day-divider"]',
    );
    const firstDividerPill = firstDivider?.querySelector<HTMLElement>("p");
    if (!firstGroup || !firstDivider || !firstDividerPill) {
      throw new Error("missing day group or divider");
    }

    const groupRect = firstGroup.getBoundingClientRect();
    const dividerRect = firstDivider.getBoundingClientRect();
    const groupBefore = getComputedStyle(firstGroup, "::before");
    const dividerBefore = getComputedStyle(firstDivider, "::before");

    return {
      dividerBeforeContent: dividerBefore.content,
      dividerPillBackground: getComputedStyle(firstDividerPill).backgroundColor,
      dividerPillShadow: getComputedStyle(firstDividerPill).boxShadow,
      dividerPosition: getComputedStyle(firstDivider).position,
      dividerTop: dividerRect.top,
      dividerZIndex: getComputedStyle(firstDivider).zIndex,
      groupBeforeContent: groupBefore.content,
      groupBeforePosition: groupBefore.position,
      groupTop: groupRect.top,
      ruleTop: groupRect.top + Number.parseFloat(groupBefore.top),
    };
  });

  expect(metrics.dividerPosition).toBe("sticky");
  expect(Number.parseInt(metrics.dividerZIndex, 10)).toBeGreaterThan(10);
  await expect
    .poll(async () => {
      const headerZIndex = await page
        .getByTestId("chat-header")
        .evaluate(
          (element) =>
            getComputedStyle(element.parentElement ?? element).zIndex,
        );
      return Number.parseInt(headerZIndex, 10);
    })
    .toBeGreaterThan(Number.parseInt(metrics.dividerZIndex, 10));
  await expect
    .poll(async () => {
      const sharedBackdropZIndex = await page
        .getByTestId("channel-shared-header-backdrop")
        .evaluate((element) => getComputedStyle(element).zIndex);
      return Number.parseInt(sharedBackdropZIndex, 10);
    })
    .toBeGreaterThan(Number.parseInt(metrics.dividerZIndex, 10));
  const composerOverlay = page.getByTestId("channel-composer-overlay");
  await expect(composerOverlay).toBeVisible();
  await expect(composerOverlay).toHaveCSS("backdrop-filter", "none");
  await expect(composerOverlay).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await expect(composerOverlay.getByTestId("message-composer")).not.toHaveCSS(
    "backdrop-filter",
    "none",
  );
  const composerActivityRow = composerOverlay.getByTestId(
    "channel-composer-activity-row",
  );
  await expect(composerActivityRow).toHaveCSS("backdrop-filter", "none");
  await expect(composerActivityRow).not.toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await expect
    .poll(async () => {
      const composerOverlayZIndex = await page
        .getByTestId("channel-composer-overlay")
        .evaluate((element) => getComputedStyle(element).zIndex);
      return Number.parseInt(composerOverlayZIndex, 10);
    })
    .toBeGreaterThan(Number.parseInt(metrics.dividerZIndex, 10));
  expect(metrics.dividerPillBackground).not.toBe("rgba(0, 0, 0, 0)");
  expect(metrics.dividerPillBackground).not.toBe("transparent");
  expect(metrics.dividerPillShadow).toBe("none");
  expect(metrics.dividerBeforeContent).toBe("none");
  expect(metrics.groupBeforePosition).toBe("absolute");
  expect(metrics.groupBeforeContent).not.toBe("none");
  expect(metrics.groupTop).toBeLessThan(metrics.dividerTop - 8);
  expect(metrics.ruleTop).toBeLessThan(metrics.dividerTop - 8);
});

test("shows and clears activity indicators for active channel agents", async ({
  page,
}) => {
  await page.goto("/");

  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");
  await waitForMockLiveSubscription(page, "agents", KIND_TYPING_INDICATOR);

  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
      channelName: "agents",
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  await expect(page.getByTestId("bot-activity-composer-trigger")).toBeVisible();
  await expect(
    page.getByTestId("bot-activity-composer-trigger"),
  ).not.toContainText("View activity");
  await page.getByTestId("bot-activity-composer-trigger").click();
  const aliceActivityItem = page.getByTestId(
    `bot-activity-composer-item-${TEST_IDENTITIES.alice.pubkey}`,
  );
  await expect(aliceActivityItem).toBeVisible();
  await expect(aliceActivityItem).toContainText("View activity");
  await aliceActivityItem.click({ force: true });
  await expect(page.getByTestId("agent-session-thread-panel")).toBeVisible();
  await expect(page.getByTestId("agent-session-thread-panel")).toContainText(
    "alice",
  );
  // Opened from the composer with no prior pane: there is nowhere to go
  // "back" to, so the header shows only the close affordance.
  await expect(page.getByTestId("agent-session-back")).toHaveCount(0);
  await expect(page.getByTestId("auxiliary-panel-close")).toBeVisible();
  await expect(page.getByTestId("agent-transcript-now-summary")).toHaveCount(0);
  await page.getByTestId("agent-session-settings-menu-trigger").click();
  await expect(page.getByTestId("agent-session-stop-turn")).toBeVisible();
  await expect(page.getByTestId("agent-session-stop-turn")).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("agent-session-thread-panel")).toContainText(
    "No ACP activity yet",
  );
  await expect(page.getByTestId("message-typing-indicator")).toHaveCount(0);

  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
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
    window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
      channelName: "agents",
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  await expect(page.getByTestId("bot-activity-composer-trigger")).toHaveCount(
    0,
  );
});

test("members sidebar exposes view-activity for a viewer-owned relay agent", async ({
  page,
}) => {
  // nadia is no longer in the default relay-agent seeds (the no-relay-record
  // owner path claims that fixture), so seed her relay registry entry here to
  // exercise the relay-bot classification.
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: OWNED_RELAY_AGENT_PUBKEY,
        name: "nadia",
        agentType: "goose",
        capabilities: ["search", "summaries"],
        channelNames: ["agents"],
        respondTo: "anyone",
      },
    ],
  });
  await page.goto("/");

  await openMembersSidebar(page, "agents");

  // nadia is a relay-only agent (no local managed record) owned by the mock
  // viewer. The view-activity affordance must open via the declared-owner
  // path, not the local-managed path.
  await expect(
    page.getByTestId(`sidebar-member-${OWNED_RELAY_AGENT_PUBKEY}`),
  ).toBeVisible();
  await openMemberMenu(page, OWNED_RELAY_AGENT_PUBKEY);
  await expect(
    page.getByTestId(`sidebar-view-activity-${OWNED_RELAY_AGENT_PUBKEY}`),
  ).toBeVisible();
});

test("profile renders live activity for a viewer-owned relay agent", async ({
  page,
}) => {
  await page.goto("/");

  await openMembersSidebar(page, "agents");
  await page
    .getByTestId(`sidebar-member-open-profile-${OWNED_RELAY_AGENT_PUBKEY}`)
    .click();
  await expect(page.getByTestId("members-sidebar")).not.toBeVisible();
  await expect(
    page.getByTestId(`user-profile-view-activity-${OWNED_RELAY_AGENT_PUBKEY}`),
  ).toBeVisible();

  await page.waitForFunction(
    () =>
      typeof (window as MockFeedWindow).__BUZZ_E2E_SEED_ACTIVE_TURNS__ ===
      "function",
  );
  await page.evaluate(
    ({ agentPubkey, channelId }) => {
      const seedActiveTurns = (window as MockFeedWindow)
        .__BUZZ_E2E_SEED_ACTIVE_TURNS__;
      if (!seedActiveTurns) {
        throw new Error("Mock active-turn helper is not installed.");
      }
      seedActiveTurns({
        agentPubkey,
        channelId,
        turnId: "owned-relay-profile-turn",
      });
    },
    {
      agentPubkey: OWNED_RELAY_AGENT_PUBKEY,
      channelId: AGENTS_CHANNEL_ID,
    },
  );

  const liveActivity = page.getByTestId(
    `user-profile-live-activity-${OWNED_RELAY_AGENT_PUBKEY}`,
  );
  await expect(liveActivity).toBeVisible();
  await expect(liveActivity).toContainText("Latest Activity");
  await expect(liveActivity).toContainText("#agents");
  await expect(
    page.getByTestId(`user-profile-activity-dot-${AGENTS_CHANNEL_ID}`),
  ).toHaveCount(0);

  await page.evaluate(
    ({ agentPubkey, channelId, turnId }) => {
      const seedActiveTurns = (window as MockFeedWindow)
        .__BUZZ_E2E_SEED_ACTIVE_TURNS__;
      if (!seedActiveTurns) {
        throw new Error("Mock active-turn helper is not installed.");
      }
      seedActiveTurns({
        agentPubkey,
        channelId,
        turnId,
        kind: "turn_completed",
      });
    },
    {
      agentPubkey: OWNED_RELAY_AGENT_PUBKEY,
      channelId: AGENTS_CHANNEL_ID,
      turnId: "owned-relay-profile-turn",
    },
  );

  await expect(liveActivity).toBeVisible();
  await expect(liveActivity).toContainText("Latest Activity");
  await expect(
    page.getByTestId(`user-profile-view-activity-${OWNED_RELAY_AGENT_PUBKEY}`),
  ).not.toBeVisible();
});

test("profile activity carousel switches channels via progress dots", async ({
  page,
}) => {
  await page.goto("/");

  await openMembersSidebar(page, "agents");
  await page
    .getByTestId(`sidebar-member-open-profile-${OWNED_RELAY_AGENT_PUBKEY}`)
    .click();
  await expect(page.getByTestId("members-sidebar")).not.toBeVisible();
  await expect(
    page.getByTestId(`user-profile-view-activity-${OWNED_RELAY_AGENT_PUBKEY}`),
  ).toBeVisible();

  await page.waitForFunction(
    () =>
      typeof (window as MockFeedWindow).__BUZZ_E2E_SEED_ACTIVE_TURNS__ ===
      "function",
  );

  await page.evaluate(
    ({ agentPubkey, channels }) => {
      const seedActiveTurns = (window as MockFeedWindow)
        .__BUZZ_E2E_SEED_ACTIVE_TURNS__;
      if (!seedActiveTurns) {
        throw new Error("Mock active-turn helper is not installed.");
      }

      for (const [index, channelId] of channels.entries()) {
        seedActiveTurns({
          agentPubkey,
          channelId,
          turnId: `owned-relay-profile-turn-${index}`,
        });
      }
    },
    {
      agentPubkey: OWNED_RELAY_AGENT_PUBKEY,
      channels: [AGENTS_CHANNEL_ID, GENERAL_CHANNEL_ID],
    },
  );

  const liveActivity = page.getByTestId(
    `user-profile-live-activity-${OWNED_RELAY_AGENT_PUBKEY}`,
  );
  await expect(liveActivity).toBeVisible();
  await expect(liveActivity).toContainText("#agents");

  await expect(
    page.getByTestId(`user-profile-activity-dot-${AGENTS_CHANNEL_ID}`),
  ).toBeVisible();
  await expect(
    page.getByTestId(`user-profile-activity-dot-${GENERAL_CHANNEL_ID}`),
  ).toBeVisible();

  await expect(
    page.getByTestId(`user-profile-activity-slide-${GENERAL_CHANNEL_ID}`),
  ).toHaveAttribute("data-mounted", "false");

  await page
    .getByTestId(`user-profile-activity-dot-${GENERAL_CHANNEL_ID}`)
    .click();

  await expect(
    page.getByTestId("user-profile-activity-channel-label"),
  ).toContainText("#general");
  await expect(
    page.getByTestId(`user-profile-activity-slide-${GENERAL_CHANNEL_ID}`),
  ).toHaveAttribute("data-mounted", "true");
  await expect(
    page.getByTestId(`user-profile-activity-slide-${AGENTS_CHANNEL_ID}`),
  ).toHaveAttribute("data-mounted", "true");
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
    window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
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
    window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
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
    window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
      channelName: "random",
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  await expect(
    page.getByTestId("message-typing-indicator-label"),
  ).toContainText("alice and bob are typing");

  // Bob re-broadcasts — order should still stay "alice and bob"
  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
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
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: "Unread update for #random",
        kind: 40002,
        pubkey,
      });
    },
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );

  await expect(page.getByTestId("channel-random")).toHaveCSS(
    "font-weight",
    "600",
  );
  await expect(page.getByTestId("channel-unread-random")).toHaveCount(0);

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
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "watercooler",
        content: "Unread update for the forum",
        kind: 45001,
        pubkey,
      });
    },
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );

  await expect(page.getByTestId("channel-watercooler")).toHaveCSS(
    "font-weight",
    "600",
  );
  await expect(page.getByTestId("channel-unread-watercooler")).toHaveCount(0);

  await page.getByTestId("channel-watercooler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");
  await expect(page.getByTestId("channel-unread-watercooler")).toHaveCount(0);
});

test("sidebar clears unread indicator after opening a DM", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByTestId("channel-unread-alice-tyler")).toHaveCount(0);
  await waitForMockLiveSubscription(page, "alice-tyler");

  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "alice-tyler",
      content: "Unread update for the DM",
      pubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  await expect(page.getByTestId("channel-unread-alice-tyler")).toBeVisible();

  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");
  await expect(page.getByTestId("message-dm-intro")).toBeVisible();
  await expect(page.getByTestId("message-dm-intro")).toContainText(
    "This is the beginning of your direct message with",
  );
  await expect(page.getByTestId("message-timeline-day-divider")).toBeVisible();
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Unread update for the DM",
  );
  await expectSameLeftInset(page, "message-dm-intro", "message-row");
  await expectIntroSpacedAboveDayDivider(page, "message-dm-intro");
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
  await openChannelEditDialog(page);
  const editDialog = page.getByRole("dialog", { name: "Edit channel" });

  await editDialog.getByTestId("channel-management-name").fill(newName);
  await editDialog
    .getByTestId("channel-management-description")
    .fill(newDescription);
  await editDialog.getByTestId("channel-management-topic").fill(newTopic);
  await editDialog.getByTestId("channel-management-purpose").fill(newPurpose);
  await editDialog.getByTestId("channel-management-save-changes").click();
  await expect(editDialog).toHaveCount(0);

  await expect(page.getByTestId("chat-title")).toHaveText(newName);
  await expect(page.getByTestId("stream-list")).toContainText(newName);
  await expect(page.getByTestId("channel-management-name-row")).toContainText(
    newName,
  );
  await expect(
    page.getByTestId("channel-management-description"),
  ).toContainText(newDescription);
  await expect(page.getByTestId("channel-management-topic")).toContainText(
    newTopic,
  );
  await expect(page.getByTestId("channel-management-purpose")).toContainText(
    newPurpose,
  );

  await closeChannelManagement(page);

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");

  await page.getByTestId("stream-list").getByText(newName).click();
  await expect(page.getByTestId("chat-title")).toHaveText(newName);
  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-management-sheet")).toBeVisible();
  await openChannelEditDialog(page);
  const reopenedEditDialog = page.getByRole("dialog", {
    name: "Edit channel",
  });

  await expect(
    reopenedEditDialog.getByTestId("channel-management-name"),
  ).toHaveValue(newName);
  await expect(
    reopenedEditDialog.getByTestId("channel-management-description"),
  ).toHaveValue(newDescription);
  await expect(
    reopenedEditDialog.getByTestId("channel-management-topic"),
  ).toHaveValue(newTopic);
  await expect(
    reopenedEditDialog.getByTestId("channel-management-purpose"),
  ).toHaveValue(newPurpose);
});

test("manage channel updates visibility and ephemeral lifecycle independently", async ({
  page,
}) => {
  await page.goto("/");
  await openChannelManagement(page, "general");
  await openChannelEditDialog(page);

  let saveChangesButton = page.getByTestId("channel-management-save-changes");

  await expect(saveChangesButton).toBeDisabled();

  await page.getByTestId("channel-management-private-toggle").click();
  await page.getByTestId("channel-management-ephemeral-toggle").click();
  await expect(page.getByTestId("channel-management-ttl")).toBeVisible();
  await expect(saveChangesButton).toBeEnabled();

  const commandCountBeforeEnable = (await readCommandPayloadLog(page)).length;
  await saveChangesButton.click();
  await expect
    .poll(async () =>
      (await readCommandPayloadLog(page)).slice(commandCountBeforeEnable),
    )
    .toContainEqual(
      expect.objectContaining({
        command: "update_channel",
        payload: expect.objectContaining({
          input: expect.objectContaining({ ttlSeconds: 604800 }),
        }),
      }),
    );
  await expect(page.getByRole("dialog", { name: "Edit channel" })).toHaveCount(
    0,
  );

  const channelAfterEnable = await invokeMockCommand<{
    ttl_seconds: number | null;
    visibility: string;
  }>(page, "get_channel_details", {
    channelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
  });
  expect(channelAfterEnable).toMatchObject({
    ttl_seconds: 604800,
    visibility: "private",
  });

  await closeChannelManagement(page);
  await openChannelManagement(page, "general");
  await openChannelEditDialog(page);
  saveChangesButton = page.getByTestId("channel-management-save-changes");

  await expect(
    page.getByTestId("channel-management-private-toggle"),
  ).toHaveAttribute("data-state", "checked");
  await expect(
    page.getByTestId("channel-management-ephemeral-toggle"),
  ).toHaveAttribute("data-state", "checked");
  await expect(page.getByTestId("channel-management-ttl")).toHaveValue("7d");

  await page.getByTestId("channel-management-private-toggle").click();
  await page.getByTestId("channel-management-ephemeral-toggle").click();
  await expect(saveChangesButton).toBeEnabled();

  const commandCountBeforeDisable = (await readCommandPayloadLog(page)).length;
  await saveChangesButton.click();
  await expect
    .poll(async () =>
      (await readCommandPayloadLog(page)).slice(commandCountBeforeDisable),
    )
    .toContainEqual(
      expect.objectContaining({
        command: "update_channel",
        payload: expect.objectContaining({
          input: expect.objectContaining({ ttlSeconds: null }),
        }),
      }),
    );
  await expect(page.getByRole("dialog", { name: "Edit channel" })).toHaveCount(
    0,
  );

  const channelAfterDisable = await invokeMockCommand<{
    ttl_seconds: number | null;
    visibility: string;
  }>(page, "get_channel_details", {
    channelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
  });
  expect(channelAfterDisable).toMatchObject({
    ttl_seconds: null,
    visibility: "open",
  });

  await closeChannelManagement(page);
  await openChannelManagement(page, "general");
  await openChannelEditDialog(page);

  await expect(
    page.getByTestId("channel-management-private-toggle"),
  ).toHaveAttribute("data-state", "unchecked");
  await expect(
    page.getByTestId("channel-management-ephemeral-toggle"),
  ).toHaveAttribute("data-state", "unchecked");
  await expect(page.getByTestId("channel-management-ttl")).toHaveCount(0);
});

test("manage channel keeps canvas near the top of the sheet", async ({
  page,
}) => {
  await page.goto("/");
  await openChannelManagement(page, "general");

  const sheet = page.getByTestId("channel-management-sheet");
  const sheetBox = await sheet.boundingBox();
  const timelineBox = await page.getByTestId("message-timeline").boundingBox();

  // Canvas ingress should appear before the channel metadata rows in the DOM.
  const canvasBox = await sheet
    .getByTestId("channel-canvas-ingress")
    .boundingBox();
  const nameBox = await sheet
    .getByTestId("channel-management-name-row")
    .boundingBox();

  expect(sheetBox).not.toBeNull();
  expect(timelineBox).not.toBeNull();
  if (!sheetBox || !timelineBox) {
    throw new Error("Expected channel management panel and timeline boxes.");
  }
  expect(timelineBox.x + timelineBox.width).toBeLessThanOrEqual(sheetBox.x + 1);
  await page.mouse.click(timelineBox.x + 24, timelineBox.y + 180);
  await expect(sheet).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(sheet).toBeVisible();
  await page.setViewportSize({ height: 720, width: 820 });
  await expect(page.getByTestId("message-timeline")).toHaveCount(0);
  await expect(page.getByTestId("channel-drop-zone")).toHaveCount(0);
  await expect(sheet).toBeVisible();
  const narrowSheetBox = await sheet.boundingBox();
  if (!narrowSheetBox) {
    throw new Error("Expected narrow channel management panel box.");
  }
  expect(narrowSheetBox.width).toBeGreaterThan(500);
  expect(canvasBox).not.toBeNull();
  expect(nameBox).not.toBeNull();
  expect(canvasBox?.y).toBeLessThan(nameBox?.y);
});

async function seedHomeInboxMention(
  page: import("@playwright/test").Page,
  itemId: string,
  tags?: string[][],
) {
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  await page.waitForFunction(
    () =>
      typeof (window as MockFeedWindow).__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ ===
      "function",
  );

  await page.evaluate(
    ({
      channelId,
      createdAt,
      currentPubkey,
      itemId: id,
      senderPubkey,
      tags: seededTags,
    }) => {
      const pushFeedItem = (window as MockFeedWindow)
        .__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
      if (!pushFeedItem) {
        throw new Error("Mock feed injection helper is not installed.");
      }

      pushFeedItem({
        id,
        kind: 9,
        pubkey: senderPubkey,
        content: "Please review the home panel routing.",
        created_at: createdAt,
        channel_id: channelId,
        channel_name: "general",
        tags: seededTags ?? [
          ["e", channelId],
          ["p", currentPubkey],
        ],
        category: "mention",
      });
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      createdAt: Math.floor(Date.now() / 1000),
      currentPubkey: TEST_IDENTITIES.tyler.pubkey,
      itemId,
      senderPubkey: TEST_IDENTITIES.alice.pubkey,
      tags,
    },
  );

  await page.getByTestId(`home-inbox-item-${itemId}`).click();
}

test("home inbox channel label navigates to the channel message", async ({
  page,
}) => {
  await seedHomeInboxMention(page, "mock-feed-home-channel-navigate");

  await page
    .getByTestId("home-inbox-detail")
    .getByRole("button", { exact: true, name: "general" })
    .click();

  await expect(page).toHaveURL(
    new RegExp(`#/channels/${GENERAL_CHANNEL_ID}\\?`),
  );
  await expect(page).toHaveURL(/messageId=mock-feed-home-channel-navigate/);
  await expect(page).not.toHaveURL(/threadRootId=/);
  await expect(page.getByTestId("message-timeline")).toBeVisible();
  await expect(page.getByTestId("home-inbox-list")).toHaveCount(0);
});

test("home inbox thread reply mention carries threadRootId to the channel", async ({
  page,
}) => {
  const rootEventId = "mock-feed-home-thread-root";
  await seedHomeInboxMention(page, "mock-feed-home-thread-navigate", [
    ["e", rootEventId, "", "root"],
    ["e", "mock-feed-home-thread-parent", "", "reply"],
    ["p", TEST_IDENTITIES.tyler.pubkey],
  ]);

  await page
    .getByTestId("home-inbox-detail")
    .getByRole("button", { exact: true, name: "general" })
    .click();

  await expect(page).toHaveURL(
    new RegExp(`#/channels/${GENERAL_CHANNEL_ID}\\?`),
  );
  await expect(page).toHaveURL(/messageId=mock-feed-home-thread-navigate/);
  await expect(page).toHaveURL(new RegExp(`threadRootId=${rootEventId}`));
  await expect(page.getByTestId("message-timeline")).toBeVisible();
  await expect(page.getByTestId("home-inbox-list")).toHaveCount(0);
});

test("home inbox manage affordance opens management without leaving home", async ({
  page,
}) => {
  await seedHomeInboxMention(page, "mock-feed-home-channel-panel");

  await page
    .getByTestId("home-inbox-detail")
    .getByTestId("channel-management-trigger")
    .click();

  await expect(page.getByTestId("channel-management-sheet")).toBeVisible();
  await expect(page.getByTestId("channel-management-name-row")).toContainText(
    "general",
  );
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  const detailBox = await page.getByTestId("home-inbox-detail").boundingBox();
  const sheetBox = await page
    .getByTestId("channel-management-sheet")
    .boundingBox();
  if (!detailBox || !sheetBox) {
    throw new Error("Expected home detail pane and channel management boxes.");
  }
  expect(detailBox.x + detailBox.width).toBeLessThanOrEqual(sheetBox.x + 1);
  expect(sheetBox.width).toBeGreaterThanOrEqual(300);
  await page
    .getByTestId("home-inbox-list")
    .click({ position: { x: 24, y: 80 } });
  await expect(page.getByTestId("channel-management-sheet")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("channel-management-sheet")).toBeVisible();
  await page.setViewportSize({ height: 720, width: 820 });
  await expect(page.getByTestId("home-inbox-list")).toHaveCount(0);
  const narrowHomeBox = await page.getByTestId("home-inbox").boundingBox();
  const narrowSheetBox = await page
    .getByTestId("channel-management-sheet")
    .boundingBox();
  if (!narrowHomeBox || !narrowSheetBox) {
    throw new Error("Expected narrow home and channel management boxes.");
  }
  expect(narrowSheetBox.width).toBeGreaterThanOrEqual(narrowHomeBox.width - 1);
  await expect(page).not.toHaveURL(/#\/channels\//);
});

test("members sidebar can invite and remove members", async ({ page }) => {
  await page.goto("/");
  await openMembersSidebar(page, "general");
  const initialMemberCount = await readMembersTriggerCount(page);
  await expect(page.getByTestId("channel-management-add-pubkeys")).toHaveCount(
    0,
  );

  await expect(page.getByText(/Members · \d+/)).toBeVisible();
  await page.getByTestId("channel-management-search-users").fill("a");
  await expect(page.getByText("Members", { exact: true })).toBeVisible();
  await expect(page.getByText(/Members · \d+/)).toHaveCount(0);
  await expect(
    page.getByText("Not in this channel", { exact: true }),
  ).toBeVisible();

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
    page.getByTestId(`sidebar-member-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toContainText("charlie");
  await expect(
    page.getByTestId(`selected-invitee-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toHaveCount(0);
  await expect(page.getByTestId("channel-management-add-members")).toHaveCount(
    0,
  );
  await expect(page.getByTestId("channel-management-search-users")).toHaveValue(
    "char",
  );
  await expectMembersTriggerCount(page, initialMemberCount + 1);

  await openMemberMenu(page, TEST_IDENTITIES.charlie.pubkey);
  await page
    .getByTestId(`sidebar-remove-member-${TEST_IDENTITIES.charlie.pubkey}`)
    .click();

  await expect(
    page.getByTestId(`sidebar-member-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toHaveCount(0);
  await expectMembersTriggerCount(page, initialMemberCount);
});

test("members sidebar pages add-member search beyond the first 50 people", async ({
  page,
}) => {
  const searchProfiles = Array.from({ length: 55 }, (_, index) => ({
    pubkey: `${(index + 1).toString(16).padStart(64, "0")}`,
    displayName: `Alex ${String(index + 1).padStart(2, "0")}`,
  }));
  await installMockBridge(page, { searchProfiles });

  await page.goto("/");
  await openMembersSidebar(page, "general");
  await page.getByTestId("channel-management-search-users").fill("Alex");

  const results = page.locator("[data-testid^='channel-user-search-result-']");
  await expect(results).toHaveCount(50);

  await page
    .getByTestId("members-sidebar-people")
    .evaluate((node) => node.scrollTo(0, node.scrollHeight));

  await expect(results).toHaveCount(55);
  await expect(page.getByText("Alex 55")).toBeVisible();

  const searchCalls = (await readCommandPayloadLog(page)).filter(
    (entry) => entry.command === "search_users",
  );
  expect(searchCalls).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        payload: expect.objectContaining({ cursor: null, limit: 50 }),
      }),
      expect.objectContaining({
        payload: expect.objectContaining({ cursor: "2", limit: 50 }),
      }),
    ]),
  );
});

test("shows loading skeletons without stale recipient results", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [],
    relayAgents: [],
    userSearchDelayMs: 1_000,
  });
  await page.goto("/");
  await openNewMessagePage(page);

  const search = page.getByTestId("new-dm-search");
  await search.fill("Alex");

  await expect(page.getByTestId("new-dm-loading")).toBeVisible();
  await expect(
    page.locator("[data-testid^='new-dm-result-']:visible"),
  ).toHaveCount(0);
  await expect(search).not.toHaveAttribute("aria-activedescendant");
  await search.press("Enter");
  await expect(
    page.locator("button[data-testid^='new-dm-selected-']"),
  ).toHaveCount(0);

  await expect(page.getByTestId("new-dm-loading")).toBeHidden();
  await expect(page.locator("[data-testid^='new-dm-result-']")).toHaveCount(0);
  await expect(page.getByTestId("new-dm-empty")).toContainText(
    "No matching users.",
  );
});

test("new DM picker pages people search beyond the first 50 results", async ({
  page,
}) => {
  const searchProfiles = Array.from({ length: 55 }, (_, index) => ({
    pubkey: `${(index + 1).toString(16).padStart(64, "0")}`,
    displayName: `Alex ${String(index + 1).padStart(2, "0")}`,
  }));
  await installMockBridge(page, { searchProfiles });

  await page.goto("/");
  await openNewMessagePage(page);
  await expect(page.getByTestId("new-message-page")).toBeVisible();
  await page.getByTestId("new-dm-search").fill("Alex");

  const results = page.locator("[data-testid^='new-dm-result-']");
  await expect(results).toHaveCount(50);

  await page
    .getByTestId("new-dm-results")
    .evaluate((node) => node.scrollTo(0, node.scrollHeight));

  await expect(results).toHaveCount(55);
  await expect(page.getByText("Alex 55")).toBeVisible();

  const searchCalls = (await readCommandPayloadLog(page)).filter(
    (entry) => entry.command === "search_users",
  );
  expect(searchCalls).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        payload: expect.objectContaining({ cursor: null, limit: 50 }),
      }),
      expect.objectContaining({
        payload: expect.objectContaining({ cursor: "2", limit: 50 }),
      }),
    ]),
  );
});

test("member people search starts at two characters", async ({ page }) => {
  const jmPubkey =
    "abababababababababababababababababababababababababababababababab";
  await installMockBridge(page, {
    searchProfiles: [{ pubkey: jmPubkey, displayName: "jm" }],
  });

  await page.goto("/");
  await openMembersSidebar(page, "general");
  await page.getByTestId("channel-management-search-users").fill("j");
  await expect(
    page.getByTestId(`channel-user-search-result-${jmPubkey}`),
  ).toHaveCount(0);
  expect(
    (await readCommandPayloadLog(page)).filter(
      (entry) =>
        entry.command === "search_users" &&
        (entry.payload as { query?: string }).query === "j",
    ),
  ).toHaveLength(0);

  await page.getByTestId("channel-management-search-users").fill("jm");
  await expect(
    page.getByTestId(`channel-user-search-result-${jmPubkey}`),
  ).toContainText("jm");

  const searchCalls = (await readCommandPayloadLog(page)).filter(
    (entry) => entry.command === "search_users",
  );
  expect(searchCalls).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        payload: expect.objectContaining({ query: "jm" }),
      }),
    ]),
  );
});

test("members modal does not show direct pubkey entry", async ({ page }) => {
  await page.goto("/");
  await openMembersSidebar(page, "general");

  await expect(page.getByTestId("channel-management-add-pubkeys")).toHaveCount(
    0,
  );
  await expect(
    page.getByTestId("channel-management-toggle-direct-pubkeys"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("channel-management-search-users"),
  ).toHaveAttribute("placeholder", "Add people and agents");
});

test("channel header omits the add agent action", async ({ page }) => {
  await page.goto("/");

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await page.setViewportSize({ width: 1280, height: 420 });

  await expect(page.getByTestId("channel-add-bot-trigger")).toHaveCount(0);
  await expect(page.getByTestId("channel-members-trigger")).toBeVisible();
  await expect(page.getByTestId("channel-start-huddle-trigger")).toBeVisible();
  await expect(page.getByTestId("channel-management-trigger")).toBeVisible();
});

test("channel header actions show tooltips", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");

  for (const [testId, label] of [
    ["channel-members-trigger", "Channel members"],
    ["channel-huddle-tooltip-trigger", "Huddle"],
    ["channel-management-trigger", "Channel settings"],
  ] as const) {
    await page.getByTestId(testId).hover();
    await expect(page.getByRole("tooltip", { name: label })).toBeVisible();
  }
});

test("members sidebar collapses same-persona managed agents", async ({
  page,
}) => {
  const inChannelAgentPubkey =
    "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
  const outOfChannelAgentPubkey =
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: outOfChannelAgentPubkey,
        name: "Pinky",
        personaId: "builtin:fizz",
        status: "stopped",
      },
      {
        pubkey: inChannelAgentPubkey,
        name: "Pinky",
        personaId: "builtin:fizz",
        status: "running",
        channelNames: ["general"],
      },
    ],
    searchProfiles: [
      {
        pubkey: outOfChannelAgentPubkey,
        displayName: "Pinky",
        ownerPubkey: MOCK_IDENTITY_PUBKEY,
        isAgent: true,
      },
      {
        pubkey: inChannelAgentPubkey,
        displayName: "Pinky",
        ownerPubkey: MOCK_IDENTITY_PUBKEY,
        isAgent: true,
      },
    ],
    userSearchDelayMs: 1_000,
  });
  await page.goto("/");
  await openMembersSidebar(page, "general");

  await page.getByTestId("channel-management-search-users").fill("pi");

  await expect(
    page.getByTestId(`channel-user-search-result-${inChannelAgentPubkey}`),
  ).toHaveCount(0);
  await expect(
    page.getByTestId(`channel-user-search-result-${outOfChannelAgentPubkey}`),
  ).toHaveCount(0);
  await expect(page.getByText("Pinky", { exact: true })).toHaveCount(1);
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
  await page.getByTestId("channel-management-search-users").fill("char");
  await page
    .getByTestId(`channel-user-search-result-${TEST_IDENTITIES.charlie.pubkey}`)
    .click();
  await expect(
    page.getByTestId(`sidebar-member-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toContainText("charlie");

  // The modal no longer exposes member/guest/bot role choices or a staged
  // submit button; selected people are added immediately as members and
  // selected agents are added as bots.
  await expect(page.getByTestId("channel-management-add-role")).toHaveCount(0);
  await expect(page.getByTestId("channel-management-add-members")).toHaveCount(
    0,
  );
});

test("removing a channel-scoped agent preserves the managed agent record", async ({
  page,
}) => {
  const agentName = `cleanup-agent-${Date.now()}`;

  await page.goto("/");
  const agentPubkey = await addGenericAgent(page, "general", agentName);

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
  const agentPubkey = await addGenericAgent(page, "general", agentName);
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

test("members sidebar omits bulk controls for managed bots", async ({
  page,
}) => {
  const firstAgentName = `sidebar-remove-a-${Date.now()}`;
  const secondAgentName = `sidebar-remove-b-${Date.now()}`;

  await page.goto("/");
  const firstAgentPubkey = await addGenericAgent(
    page,
    "general",
    firstAgentName,
  );
  const secondAgentPubkey = await addGenericAgent(
    page,
    "general",
    secondAgentName,
  );

  await openMembersSidebar(page, "general");
  await expect(
    page.getByTestId(`sidebar-member-${firstAgentPubkey}`),
  ).toBeVisible();
  await expect(
    page.getByTestId(`sidebar-member-${secondAgentPubkey}`),
  ).toBeVisible();
  await expect(page.getByTestId("members-sidebar-agent-controls")).toHaveCount(
    0,
  );
  await expect(page.getByTestId("members-sidebar-remove-all")).toHaveCount(0);

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
  ).toHaveLength(0);
});

test("removing a multi-channel managed bot preserves its record after removal from all channels", async ({
  page,
}) => {
  const agentName = `multi-channel-agent-${Date.now()}`;
  const secondChannelName = `multi-home-${Date.now()}`;

  await page.goto("/");
  const agentPubkey = await addGenericAgent(page, "general", agentName);

  await openCreateChannelDialog(page);
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
  await openChannelBrowser(page);
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
  await openChannelBrowser(page);
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
  await openChannelBrowser(page);
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

  await openChannelBrowser(page);
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
  await openCreateChannelDialog(page);
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
  await openCreateChannelDialog(page);
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
