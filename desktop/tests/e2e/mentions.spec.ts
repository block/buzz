import { expect, test } from "@playwright/test";

import {
  installMockBridge,
  openChannelBrowser,
  TEST_IDENTITIES,
} from "../helpers/bridge";

const MOCK_VIEWER_PUBKEY = "deadbeef".repeat(8);

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

const IN_CHANNEL_MANAGED_AGENT_PUBKEY =
  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OUT_OF_CHANNEL_MANAGED_AGENT_PUBKEY =
  "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const OUT_OF_CHANNEL_PROVIDER_AGENT_PUBKEY =
  "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const REUSABLE_PERSONA_AGENT_PUBKEY =
  "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const ALLOWLIST_RELAY_AGENT_PUBKEY =
  "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const DELAYED_RELAY_AGENT_PUBKEY =
  "9999999999999999999999999999999999999999999999999999999999999999";
const CASEY_PROFILE_PUBKEY =
  "1111111111111111111111111111111111111111111111111111111111111111";
const PROFILE_ONLY_AGENT_PUBKEY =
  "8f83d6b7f3d74f7d933ae3a54dd8c6cc85c7f98e531c16e5a827b953441a8d67";
const OWNED_AGENT_PROFILE_PUBKEY =
  "1212121212121212121212121212121212121212121212121212121212121212";
const SYSTEM_MESSAGE_KIND = 40099;
const DM_THREAD_AGENT_MENTION_ERROR_TEXT =
  "Agents must already be in a DM to be mentioned in its threads. Start a new conversation that includes the agent.";
const DM_THREAD_MEMBERS_LOADING_ERROR_TEXT =
  "Checking conversation members. Try again in a moment.";

/** Locator scoped to the mention autocomplete dropdown inside the composer. */
function autocomplete(page: import("@playwright/test").Page) {
  return page
    .getByTestId("message-composer")
    .getByTestId("mention-autocomplete");
}

async function readCommandLog(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    return (
      (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
        .__BUZZ_E2E_COMMANDS__ ?? []
    );
  });
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

function commandCount(commands: string[], command: string) {
  return commands.filter((entry) => entry === command).length;
}

async function emitMockMessage(
  page: import("@playwright/test").Page,
  channelName: string,
  content: string,
  options?: {
    kind?: number;
    mentionPubkeys?: string[];
    parentEventId?: string;
    pubkey?: string;
  },
) {
  const event = await page.evaluate(
    ({ ch, kind, mentionPubkeys, msg, parentEventId, pubkey }) => {
      return (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            kind?: number;
            mentionPubkeys?: string[];
            parentEventId?: string | null;
            pubkey?: string;
          }) => { id: string; created_at: number; pubkey: string };
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: ch,
        content: msg,
        kind,
        mentionPubkeys,
        parentEventId: parentEventId ?? undefined,
        pubkey: pubkey ?? undefined,
      });
    },
    {
      ch: channelName,
      kind: options?.kind,
      mentionPubkeys: options?.mentionPubkeys,
      msg: content,
      parentEventId: options?.parentEventId ?? null,
      pubkey: options?.pubkey ?? TEST_IDENTITIES.alice.pubkey,
    },
  );
  if (!event) {
    throw new Error("Mock message emitter is not installed");
  }
  return event;
}

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
  kind?: number,
) {
  await expect
    .poll(async () => {
      return page.evaluate(
        ({ currentChannelName, kind: expectedKind }) => {
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
              kind: expectedKind,
            }) ?? false
          );
        },
        { currentChannelName: channelName, kind },
      );
    })
    .toBe(true);
}

// The channel timeline renders off a `useDeferredValue` snapshot that lags the
// latest `messages` by a commit; the list wrapper carries
// `data-render-pending="true"` while that commit is in flight and drops the
// attribute once it settles. Poll for its absence before asserting on
// freshly-sent content so the assertion does not race the deferred commit.
async function waitForTimelineSettled(page: import("@playwright/test").Page) {
  await expect(page.locator("[data-render-pending]")).toHaveCount(0);
}

async function expectAgentProfileMessageOnly(
  profilePopover: import("@playwright/test").Locator,
  pubkey: string,
) {
  await expect(
    profilePopover.getByTestId(`user-profile-popover-message-${pubkey}`),
  ).toBeVisible();
  await expect(
    profilePopover.getByTestId(`user-profile-popover-wave-${pubkey}`),
  ).toHaveCount(0);
  await expect(
    profilePopover.getByTestId(`user-profile-popover-huddle-${pubkey}`),
  ).toHaveCount(0);
}

async function expectAgentProfileActionsHidden(
  profilePopover: import("@playwright/test").Locator,
  pubkey: string,
) {
  await expect(
    profilePopover.getByTestId(`user-profile-popover-message-${pubkey}`),
  ).toHaveCount(0);
  await expect(
    profilePopover.getByTestId(`user-profile-popover-wave-${pubkey}`),
  ).toHaveCount(0);
  await expect(
    profilePopover.getByTestId(`user-profile-popover-huddle-${pubkey}`),
  ).toHaveCount(0);
}

test("@ trigger prioritizes channel members before runnable personas and other agents", async ({
  page,
}) => {
  await installMockBridge(page, {
    activePersonaIds: ["builtin:fizz"],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@");

  const dropdown = autocomplete(page);
  await expect(dropdown).toBeVisible();
  await expect(dropdown.getByText("alice")).toBeVisible();
  await expect(dropdown.getByText("bob")).toBeVisible();
  await expect(dropdown.getByText("Fizz")).toBeVisible();
  await expect(dropdown.getByText("charlie")).toBeVisible();
  await expect(dropdown.getByText("outsider")).toHaveCount(0);
  const charlieRow = dropdown.locator("button", { hasText: "charlie" });
  await expect(charlieRow.getByTestId("mention-agent-icon")).toBeVisible();
  await expect(charlieRow.getByText("not in channel")).toBeVisible();
  await expect(
    dropdown
      .locator("button", { hasText: "alice" })
      .getByText("not in channel"),
  ).not.toBeVisible();

  const suggestions = dropdown.locator("button");
  const suggestionText = await suggestions.allInnerTexts();
  const fizzIndex = suggestionText.findIndex((text) => text.includes("Fizz"));
  const aliceIndex = suggestionText.findIndex((text) => text.includes("alice"));
  const bobIndex = suggestionText.findIndex((text) => text.includes("bob"));
  const charlieIndex = suggestionText.findIndex((text) =>
    text.includes("charlie"),
  );
  const outsiderIndex = suggestionText.findIndex((text) =>
    text.includes("outsider"),
  );
  expect(fizzIndex).toBeGreaterThanOrEqual(0);
  expect(aliceIndex).toBeGreaterThanOrEqual(0);
  expect(bobIndex).toBeGreaterThanOrEqual(0);
  expect(charlieIndex).toBeGreaterThanOrEqual(0);
  expect(outsiderIndex).toEqual(-1);
  expect(aliceIndex).toBeLessThan(fizzIndex);
  expect(bobIndex).toBeLessThan(fizzIndex);
  expect(fizzIndex).toBeLessThan(charlieIndex);
});

test("thread autocomplete keeps multiple long names readable in a narrow panel", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey:
          "9999999999999999999999999999999999999999999999999999999999999999",
        name: "Brain With A Very Long Name",
        status: "stopped",
      },
      {
        pubkey:
          "9999999999999999999999999999999999999999999999999999999999999998",
        name: "Brainstorming Assistant With A Long Name",
        status: "stopped",
      },
      {
        pubkey:
          "9999999999999999999999999999999999999999999999999999999999999997",
        name: "Brainy Helper With Another Long Name",
        status: "stopped",
      },
    ],
  });
  await page.setViewportSize({ width: 900, height: 640 });
  await page.addInitScript(() => {
    window.sessionStorage.setItem("buzz.desktop.thread-panel-width", "300");
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.setViewportSize({ width: 760, height: 640 });

  await emitMockMessage(page, "general", "Reply to open the thread", {
    parentEventId: "mock-general-welcome",
  });
  const threadSummary = page.getByTestId("message-thread-summary").first();
  await expect(threadSummary).toBeVisible();
  await threadSummary.click();

  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();
  const panelBox = await threadPanel.boundingBox();
  expect(panelBox?.width ?? Number.POSITIVE_INFINITY).toBeLessThanOrEqual(320);

  const input = threadPanel.getByTestId("message-input");
  await input.fill("@Brain");

  const dropdown = threadPanel.getByTestId("mention-autocomplete");
  await expect(dropdown).toBeVisible();

  for (const name of [
    "Brain With A Very Long Name",
    "Brainstorming Assistant With A Long Name",
    "Brainy Helper With Another Long Name",
  ]) {
    const row = dropdown.locator("button", { hasText: name });
    await expect(row).toBeVisible();
    await expect(
      row.getByTestId("mention-suggestion-avatar-fallback"),
    ).toBeVisible();
    await expect(row.getByText("agent")).toBeVisible();
    await expect(row.getByText("managed by you")).toBeVisible();

    await expect(row.getByText(name)).not.toHaveCSS(
      "text-overflow",
      "ellipsis",
    );
  }
});

test("blocks non-participant persona mentions in DM threads", async ({
  page,
}) => {
  await installMockBridge(page, {
    activePersonaIds: ["builtin:fizz"],
  });
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");
  await waitForMockLiveSubscription(page, "bob-tyler");

  const threadRoot = await emitMockMessage(
    page,
    "bob-tyler",
    "Thread before adding an agent",
  );
  await emitMockMessage(page, "bob-tyler", "Existing thread reply", {
    parentEventId: threadRoot.id,
  });
  const threadSummary = page.getByTestId("message-thread-summary").first();
  await expect(threadSummary).toBeVisible();
  await threadSummary.click();

  const threadPanel = page.getByTestId("message-thread-panel");
  const input = threadPanel.getByTestId("message-input");
  await input.fill("Ask @fi");
  await expect(
    threadPanel
      .getByTestId("mention-autocomplete")
      .locator("button", { hasText: "Fizz" }),
  ).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" in this thread");
  const baselineCommands = await readCommandLog(page);

  await threadPanel.getByTestId("send-message").click();

  await expect(
    page.getByText(
      "Agents must already be in a DM to be mentioned in its threads. Start a new conversation that includes the agent.",
    ),
  ).toBeVisible();
  const commands = await readCommandLog(page);
  expect(commandCount(commands, "create_managed_agent")).toBe(
    commandCount(baselineCommands, "create_managed_agent"),
  );
  expect(commandCount(commands, "add_channel_members")).toBe(
    commandCount(baselineCommands, "add_channel_members"),
  );
  await expect(input).toContainText("Fizz");
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");
});

test("defers agent mentions until DM members finish loading", async ({
  page,
}) => {
  await installMockBridge(page, {
    channelMembersReadDelayMs: 5_000,
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.alice.pubkey,
        name: "alice",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");
  await waitForMockLiveSubscription(page, "alice-tyler");

  const threadRoot = await emitMockMessage(
    page,
    "alice-tyler",
    "Thread while members load",
  );
  await emitMockMessage(page, "alice-tyler", "Existing thread reply", {
    parentEventId: threadRoot.id,
  });
  await page.getByTestId("message-thread-summary").first().click();

  const threadPanel = page.getByTestId("message-thread-panel");
  const input = threadPanel.getByTestId("message-input");
  await input.fill("Ask @ali");
  await expect(
    threadPanel
      .getByTestId("mention-autocomplete")
      .locator("button", { hasText: "alice" }),
  ).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" before members resolve");
  const baselineCommands = await readCommandLog(page);
  await threadPanel.getByTestId("send-message").click();

  await expect(
    page.getByText(DM_THREAD_MEMBERS_LOADING_ERROR_TEXT).first(),
  ).toBeVisible();
  await page.mouse.move(0, 0);
  expect(commandCount(await readCommandLog(page), "add_channel_members")).toBe(
    commandCount(baselineCommands, "add_channel_members"),
  );
  await expect(input).toContainText("before members resolve");

  await page.waitForTimeout(5_100);
  await threadPanel.getByTestId("send-message").click();

  await expect(page.getByText(DM_THREAD_AGENT_MENTION_ERROR_TEXT)).toHaveCount(
    0,
  );
  expect(commandCount(await readCommandLog(page), "add_channel_members")).toBe(
    commandCount(baselineCommands, "add_channel_members"),
  );
  await expect(input).toBeEmpty();
  await expect(threadPanel).toContainText("before members resolve");
});

test("autocomplete filters suggestions as user types", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@ali");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("alice")).toBeVisible();
  await expect(dropdown.getByText("bob")).not.toBeVisible();
});

test("autocomplete searches global non-member people from the first typed character", async ({
  page,
}) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: CASEY_PROFILE_PUBKEY,
        displayName: "tessa",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@t");

  const dropdown = autocomplete(page);
  const tessaRow = dropdown.locator("button", { hasText: "tessa" });
  await expect(tessaRow).toBeVisible();
  await expect(tessaRow.getByText("not in channel")).toBeVisible();
});

test("mention autocomplete pages global people search beyond the first 50 results", async ({
  page,
}) => {
  const searchProfiles = Array.from({ length: 55 }, (_, index) => ({
    pubkey: `${(index + 1).toString(16).padStart(64, "0")}`,
    displayName: `Alex ${String(index + 1).padStart(2, "0")}`,
  }));
  await installMockBridge(page, { searchProfiles });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill("@Alex");

  const dropdown = autocomplete(page);
  await expect(dropdown.locator("button")).toHaveCount(50);
  await dropdown.evaluate((node) => node.scrollTo(0, node.scrollHeight));

  await expect(dropdown.locator("button")).toHaveCount(55);
  await expect(dropdown.getByText("Alex 55")).toBeVisible();
  await expect(dropdown.getByText("not in channel").last()).toBeVisible();

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

test("selecting a person mention inserts @Name into input", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Hey @bo");

  const dropdown = autocomplete(page);
  await dropdown.getByText("bob").click();

  await expect(input).toHaveText("Hey @bob ");
  const mentionChip = input.locator(".mention-chip", {
    hasText: "@bob",
  });
  await expect(mentionChip).toBeVisible();
  await expect(mentionChip).not.toHaveClass(/agent-mention-highlight/);
});

test("selecting an agent mention inserts @Name into input", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Hey @ali");

  const dropdown = autocomplete(page);
  await dropdown.getByText("alice").click();

  await expect(input).toHaveText("Hey @alice ");
  const agentMentionChip = input.locator(".agent-mention-highlight", {
    hasText: "alice",
  });
  await expect(agentMentionChip).toBeVisible();
  await expect(agentMentionChip).toHaveText("alice");
  await expect(agentMentionChip).toHaveCSS("display", "inline-flex");
  await expect(agentMentionChip).toHaveCSS("border-top-width", "0px");
});

test("selecting a persona mention creates a channel agent before sending", async ({
  page,
}) => {
  await installMockBridge(page, {
    activePersonaIds: ["builtin:fizz"],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Ask @fi");

  const dropdown = autocomplete(page);
  const fizzRow = dropdown.locator("button", { hasText: "Fizz" });
  await expect(fizzRow).toBeVisible();
  await expect(fizzRow.getByTestId("mention-agent-icon")).toBeVisible();
  await expect(fizzRow.getByText("agent")).toBeVisible();
  await expect(fizzRow.getByText("not in channel")).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" for a hand");

  const composerChip = input.locator(".agent-mention-highlight", {
    hasText: "Fizz",
  });
  await expect(composerChip).toBeVisible();
  await expect(composerChip).toHaveText("Fizz");

  const baselineCommands = await readCommandLog(page);
  const baselineCreateCount = commandCount(
    baselineCommands,
    "create_managed_agent",
  );
  const baselineAddCount = commandCount(
    baselineCommands,
    "add_channel_members",
  );
  const baselineStartCount = commandCount(
    baselineCommands,
    "start_managed_agent",
  );

  await page.getByTestId("send-message").click();
  await expect(page.getByRole("alertdialog")).toHaveCount(0);

  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "create_managed_agent"),
    )
    .toBeGreaterThan(baselineCreateCount);
  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "add_channel_members"),
    )
    .toBeGreaterThan(baselineAddCount);
  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "start_managed_agent"),
    )
    .toBeGreaterThan(baselineStartCount);
  await expect
    .poll(async () => commandCount(await readCommandLog(page), "sign_event"))
    .toBeGreaterThan(commandCount(baselineCommands, "sign_event"));

  const commandsAfterSend = (await readCommandLog(page)).slice(
    baselineCommands.length,
  );
  const startIndex = commandsAfterSend.indexOf("start_managed_agent");
  const sendIndex = commandsAfterSend.indexOf("sign_event");
  expect(startIndex).toBeGreaterThanOrEqual(0);
  expect(sendIndex).toBeGreaterThanOrEqual(0);
  expect(startIndex).toBeLessThan(sendIndex);

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention].agent-mention-highlight", { hasText: "Fizz" });
  await expect(mentionChip).toBeVisible();
  await expect(mentionChip).toHaveText("Fizz");
});

test("selecting a persona mention reuses an existing persona agent", async ({
  page,
}) => {
  await installMockBridge(page, {
    activePersonaIds: ["builtin:fizz"],
    managedAgents: [
      {
        pubkey: REUSABLE_PERSONA_AGENT_PUBKEY,
        name: "Fizz",
        personaId: "builtin:fizz",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Ask @fi");

  const dropdown = autocomplete(page);
  const fizzRow = dropdown.locator("button", { hasText: "Fizz" });
  await expect(fizzRow).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" for a hand");

  const baselineCommands = await readCommandLog(page);
  const baselineCreateCount = commandCount(
    baselineCommands,
    "create_managed_agent",
  );
  const baselineAddCount = commandCount(
    baselineCommands,
    "add_channel_members",
  );
  const baselineStartCount = commandCount(
    baselineCommands,
    "start_managed_agent",
  );

  await page.getByTestId("send-message").click();
  await expect(page.getByRole("alertdialog")).toHaveCount(0);

  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "add_channel_members"),
    )
    .toBeGreaterThan(baselineAddCount);
  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "start_managed_agent"),
    )
    .toBeGreaterThan(baselineStartCount);
  expect(
    commandCount(await readCommandLog(page), "create_managed_agent"),
  ).toEqual(baselineCreateCount);

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention].agent-mention-highlight", { hasText: "Fizz" });
  await expect(mentionChip).toBeVisible();
  await expect(mentionChip).toHaveText("Fizz");
});

test("relay-profile agents with member roles use the agent composer style", async ({
  page,
}) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();
  await page
    .getByTestId("browse-channel-sales")
    .getByRole("button", { name: "Join" })
    .click();
  await expect(page.getByTestId("chat-title")).toHaveText("sales");

  const input = page.getByTestId("message-input");
  await input.fill("@char");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("charlie")).toBeVisible();
  await expect(dropdown.getByText("agent")).toBeVisible();
  await input.press("Enter");

  const agentMentionChip = input.locator(".agent-mention-highlight", {
    hasText: "charlie",
  });
  await expect(agentMentionChip).toBeVisible();
  await expect(agentMentionChip).toHaveText("charlie");
});

test("other-owned agents without a shared channel are hidden from mentions", async ({
  page,
}) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: PROFILE_ONLY_AGENT_PUBKEY,
        displayName: "mira",
        ownerPubkey: TEST_IDENTITIES.outsider.pubkey,
        isAgent: true,
      },
    ],
    userSearchDelayMs: 1_000,
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@mira");

  const dropdown = autocomplete(page);
  await expect(dropdown).not.toBeVisible();
  await expect(input.locator(".mention-chip")).toHaveCount(0);
});

test("own profile-only agents are hidden from channel mentions", async ({
  page,
}) => {
  await installMockBridge(page, { userSearchDelayMs: 1_000 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@mira");

  await expect(autocomplete(page)).toHaveCount(0);
});

test("allowlisted relay agents are visible in channel mentions", async ({
  page,
}) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: ["deadbeef".repeat(8)],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@quinn");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("quinn")).toBeVisible();
  await expect(dropdown.getByText("agent")).toBeVisible();
});

test("non-allowlisted relay agents stay hidden from channel mentions", async ({
  page,
}) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: [TEST_IDENTITIES.outsider.pubkey],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@quinn");

  await expect(autocomplete(page)).toHaveCount(0);
});

test("mentioning an in-channel stopped managed agent starts it before sending", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: IN_CHANNEL_MANAGED_AGENT_PUBKEY,
        name: "fizz",
        status: "stopped",
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Hey @fizz");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("fizz")).toBeVisible();
  await expect(dropdown.getByText("agent")).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" can you help?");

  const baselineStartCount = commandCount(
    await readCommandLog(page),
    "start_managed_agent",
  );
  await page.getByTestId("send-message").click();

  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "start_managed_agent"),
    )
    .toBeGreaterThan(baselineStartCount);

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention].agent-mention-highlight", { hasText: "fizz" });
  await expect(mentionChip).toBeVisible();
});

test("mentioning an in-channel provider managed agent deploys it before sending", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: OUT_OF_CHANNEL_PROVIDER_AGENT_PUBKEY,
        name: "portal",
        status: "not_deployed",
        channelNames: ["general"],
        backend: {
          type: "provider",
          id: "portal",
          config: { region: "test" },
        },
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Hey @portal");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("portal")).toBeVisible();
  await expect(dropdown.getByText("agent")).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" can you help?");

  const baselineStartCount = commandCount(
    await readCommandLog(page),
    "start_managed_agent",
  );
  await page.getByTestId("send-message").click();

  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "start_managed_agent"),
    )
    .toBeGreaterThan(baselineStartCount);

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention].agent-mention-highlight", { hasText: "portal" });
  await expect(mentionChip).toBeVisible();
});

test("mentioning a non-member managed agent adds and starts it before sending", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: OUT_OF_CHANNEL_MANAGED_AGENT_PUBKEY,
        name: "fizz",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Loop in @fizz");

  const dropdown = autocomplete(page);
  const fizzRow = dropdown.locator("button", { hasText: "fizz" });
  await expect(fizzRow).toBeVisible();
  await expect(fizzRow.getByText("not in channel")).toBeVisible();
  await input.press("Enter");

  const baselineCommands = await readCommandLog(page);
  const baselineAddCount = commandCount(
    baselineCommands,
    "add_channel_members",
  );
  const baselineStartCount = commandCount(
    baselineCommands,
    "start_managed_agent",
  );

  await page.getByTestId("send-message").click();
  await expect(page.getByRole("alertdialog")).toHaveCount(0);

  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "add_channel_members"),
    )
    .toBeGreaterThan(baselineAddCount);
  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "start_managed_agent"),
    )
    .toBeGreaterThan(baselineStartCount);

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention].agent-mention-highlight", { hasText: "fizz" });
  await expect(mentionChip).toBeVisible();
});

test("mentioning a non-member provider managed agent deploys it before sending", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: OUT_OF_CHANNEL_PROVIDER_AGENT_PUBKEY,
        name: "portal",
        status: "not_deployed",
        backend: {
          type: "provider",
          id: "portal",
          config: { region: "test" },
        },
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Loop in @portal");

  const dropdown = autocomplete(page);
  const portalRow = dropdown.locator("button", { hasText: "portal" });
  await expect(portalRow).toBeVisible();
  await expect(portalRow.getByText("not in channel")).toBeVisible();
  await input.press("Enter");

  const baselineCommands = await readCommandLog(page);
  const baselineAddCount = commandCount(
    baselineCommands,
    "add_channel_members",
  );
  const baselineStartCount = commandCount(
    baselineCommands,
    "start_managed_agent",
  );

  await page.getByTestId("send-message").click();
  await expect(page.getByRole("alertdialog")).toHaveCount(0);

  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "add_channel_members"),
    )
    .toBeGreaterThan(baselineAddCount);
  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "start_managed_agent"),
    )
    .toBeGreaterThan(baselineStartCount);

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention].agent-mention-highlight", { hasText: "portal" });
  await expect(mentionChip).toBeVisible();
});

test("system add rows use plain names while remove rows retain agent mention styling", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: OUT_OF_CHANNEL_PROVIDER_AGENT_PUBKEY,
        name: "portal",
        status: "deployed",
        backend: {
          type: "provider",
          id: "portal",
          config: { region: "test" },
        },
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general", SYSTEM_MESSAGE_KIND);

  await page.evaluate(
    ({ actorPubkey, kind, targetPubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: JSON.stringify({
          type: "member_joined",
          actor: actorPubkey,
          target: targetPubkey,
        }),
        kind,
      });
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: JSON.stringify({
          type: "member_removed",
          actor: actorPubkey,
          target: targetPubkey,
        }),
        kind,
      });
    },
    {
      actorPubkey: TEST_IDENTITIES.tyler.pubkey,
      kind: SYSTEM_MESSAGE_KIND,
      targetPubkey: OUT_OF_CHANNEL_PROVIDER_AGENT_PUBKEY,
    },
  );

  const addedRow = page
    .getByTestId("system-message-row")
    .filter({ hasText: "portal" })
    .filter({ hasText: "was added by" });
  const removedRow = page
    .getByTestId("system-message-row")
    .filter({ hasText: "removed portal from the channel" });

  const addedName = addedRow.getByText("portal", { exact: true });
  await expect(addedName).toBeVisible();
  await expect(addedName).not.toHaveAttribute("data-mention");
  await expect(
    removedRow.locator("[data-mention].agent-mention-highlight", {
      hasText: "portal",
    }),
  ).toHaveText("portal");
});

test("groups member additions and joins with hidden names in the standard tooltip", async ({
  page,
}) => {
  const actor = {
    pubkey: "10".repeat(32),
    displayName: "Alice Chen",
  };
  const targets = [
    { pubkey: "11".repeat(32), displayName: "Erica Chapman" },
    { pubkey: "12".repeat(32), displayName: "Peter Griffin" },
    { pubkey: "13".repeat(32), displayName: "Marcia Thomas" },
    { pubkey: "14".repeat(32), displayName: "Jordan Lee" },
    { pubkey: "15".repeat(32), displayName: "Olivia Park" },
    { pubkey: "16".repeat(32), displayName: "Sam Rivera" },
  ];
  await installMockBridge(page, {
    searchProfiles: [actor, ...targets],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general", SYSTEM_MESSAGE_KIND);

  await page.evaluate(
    ({ actorPubkey, addedTargets, kind }) => {
      const createdAt = Math.floor(Date.now() / 1_000);
      for (const [index, target] of addedTargets.entries()) {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "general",
          content: JSON.stringify({
            type: "member_joined",
            actor: actorPubkey,
            target: target.pubkey,
          }),
          createdAt: createdAt + index,
          kind,
        });
      }
    },
    {
      actorPubkey: actor.pubkey,
      addedTargets: targets,
      kind: SYSTEM_MESSAGE_KIND,
    },
  );
  await waitForTimelineSettled(page);

  const groupedRow = page
    .getByTestId("system-message-row")
    .filter({ hasText: "was added by Alice Chen" });
  for (const visibleName of [
    "Erica Chapman",
    "Peter Griffin",
    "Marcia Thomas",
    "Jordan Lee",
  ]) {
    await expect(groupedRow).toContainText(visibleName);
  }
  await expect(
    groupedRow.locator("p").filter({ hasText: "was added by" }),
  ).toContainText(
    "was added by Alice Chen, along with Peter Griffin, Marcia Thomas, Jordan Lee, and 2 others",
  );
  await expect(groupedRow.locator("[data-mention]")).toHaveCount(0);

  const visibleName = groupedRow.getByText("Peter Griffin", { exact: true });
  await expect(visibleName).toHaveCSS("text-decoration-line", "none");
  await visibleName.hover();
  await expect(visibleName).toHaveCSS("text-decoration-line", "underline");

  const othersTrigger = groupedRow.getByRole("button", { name: "2 others" });
  await expect(othersTrigger).toHaveCSS("text-decoration-line", "none");
  await othersTrigger.hover();
  await expect(othersTrigger).toHaveCSS("text-decoration-line", "underline");

  const tooltip = page.getByRole("tooltip");
  await expect(tooltip).toContainText("Olivia Park");
  await expect(tooltip).toContainText("Sam Rivera");

  await page.evaluate(
    ({ addedTargets, kind }) => {
      const createdAt = Math.floor(Date.now() / 1_000) + 60;
      for (const [index, target] of addedTargets.entries()) {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "general",
          content: JSON.stringify({
            type: "member_joined",
            actor: target.pubkey,
            target: target.pubkey,
          }),
          createdAt: createdAt + index,
          kind,
        });
      }
    },
    { addedTargets: targets, kind: SYSTEM_MESSAGE_KIND },
  );
  await waitForTimelineSettled(page);

  const joinedRow = page
    .getByTestId("system-message-row")
    .filter({ hasText: "joined the channel" })
    .filter({ hasText: "Erica Chapman" });
  await expect(
    joinedRow.locator("p").filter({ hasText: "joined the channel" }),
  ).toContainText(
    "joined the channel along with Peter Griffin, Marcia Thomas, Jordan Lee, and 2 others",
  );
  await expect(joinedRow.locator("[data-mention]")).toHaveCount(0);

  const joinedOthersTrigger = joinedRow.getByRole("button", {
    name: "2 others",
  });
  await expect(joinedOthersTrigger).toHaveCSS("text-decoration-line", "none");
  await joinedOthersTrigger.hover();
  await expect(page.getByRole("tooltip")).toContainText("Olivia Park");
  await expect(page.getByRole("tooltip")).toContainText("Sam Rivera");
});

test("system agent profile only exposes message action", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general", SYSTEM_MESSAGE_KIND);

  await page.evaluate(
    ({ actorPubkey, kind, targetPubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: JSON.stringify({
          type: "member_joined",
          actor: actorPubkey,
          target: targetPubkey,
        }),
        kind,
      });
    },
    {
      actorPubkey: TEST_IDENTITIES.tyler.pubkey,
      kind: SYSTEM_MESSAGE_KIND,
      targetPubkey: PROFILE_ONLY_AGENT_PUBKEY,
    },
  );
  await waitForTimelineSettled(page);

  const joinedRow = page
    .getByTestId("system-message-row")
    .filter({ hasText: "mira" })
    .filter({ hasText: "was added by" });
  const agentName = joinedRow.getByText("mira", { exact: true });
  await expect(agentName).toHaveText("mira");
  await expect(agentName).not.toHaveAttribute("data-mention");
  await agentName.hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expectAgentProfileMessageOnly(
    profilePopover,
    PROFILE_ONLY_AGENT_PUBKEY,
  );
});

test("system agent avatar only exposes message action", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await waitForMockLiveSubscription(page, "random", SYSTEM_MESSAGE_KIND);

  await page.evaluate(
    ({ kind, targetPubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: JSON.stringify({
          type: "member_joined",
          actor: targetPubkey,
          target: targetPubkey,
        }),
        kind,
      });
    },
    {
      kind: SYSTEM_MESSAGE_KIND,
      targetPubkey: PROFILE_ONLY_AGENT_PUBKEY,
    },
  );
  await waitForTimelineSettled(page);

  const joinedRow = page
    .getByTestId("system-message-row")
    .filter({ hasText: "mira" })
    .filter({ hasText: "joined the channel" });
  await joinedRow.getByTestId("system-message-avatar").hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expectAgentProfileMessageOnly(
    profilePopover,
    PROFILE_ONLY_AGENT_PUBKEY,
  );
});

test("profile-only agent author hides actions without agent access", async ({
  page,
}) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: PROFILE_ONLY_AGENT_PUBKEY,
        displayName: "mira",
        isAgent: true,
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Mira status update.", {
    pubkey: PROFILE_ONLY_AGENT_PUBKEY,
  });
  await waitForTimelineSettled(page);

  const messageRow = page
    .getByTestId("message-row")
    .filter({ hasText: "Mira status update." })
    .first();
  await messageRow.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expectAgentProfileActionsHidden(
    profilePopover,
    PROFILE_ONLY_AGENT_PUBKEY,
  );
});

test("system member-joined rows render the joined person as a plain profile name", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general", SYSTEM_MESSAGE_KIND);

  await page.evaluate(
    ({ kind, pubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: JSON.stringify({
          type: "member_joined",
          actor: pubkey,
          target: pubkey,
        }),
        kind,
      });
    },
    { kind: SYSTEM_MESSAGE_KIND, pubkey: TEST_IDENTITIES.bob.pubkey },
  );
  await waitForTimelineSettled(page);

  const joinedRow = page
    .getByTestId("system-message-row")
    .filter({ hasText: "bob" })
    .filter({ hasText: "joined the channel" });
  const joinedPersonName = joinedRow.getByText("bob", { exact: true });

  await expect(joinedPersonName).toBeVisible();
  await expect(joinedPersonName).not.toHaveAttribute("data-mention");
});

test("selecting a non-member agent from a DM inserts @Name into input", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");

  const input = page.getByTestId("message-input");
  await input.fill("@char");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("charlie")).toBeVisible();
  await expect(autocomplete(page)).toHaveCount(1);
  await expect(input.locator(".mention-chip")).toHaveCount(0);
  await input.press("Enter");

  await expect(input).toHaveText("@charlie ");
  await expect(input.locator(".mention-chip")).toBeVisible();
});

test("global non-member people can be selected from channel mentions", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Loop in @out");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("outsider")).toBeVisible();
  await expect(dropdown.getByText("not in channel")).toBeVisible();
});

test("duplicate global people with the same visible identity collapse in channel mentions", async ({
  page,
}) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: CASEY_PROFILE_PUBKEY,
        displayName: "Pip",
      },
      {
        pubkey:
          "2222222222222222222222222222222222222222222222222222222222222222",
        displayName: "Pip",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@pip");

  const dropdown = autocomplete(page);
  await expect(dropdown.locator("button", { hasText: "Pip" })).toHaveCount(1);
});

test("sent non-member person mention uses the normal mention style", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");

  const input = page.getByTestId("message-input");
  await input.fill("Loop in @out");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("outsider")).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" please");
  await page.getByTestId("send-message").click();

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention]", { hasText: "@outsider" });
  await expect(mentionChip).toBeVisible();
  await expect(mentionChip.locator("svg")).toHaveCount(0);
});

test("sent non-member agent mention uses the agent mention style", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");

  const input = page.getByTestId("message-input");
  await input.fill("Loop in @char");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("charlie")).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" too");
  await page.getByTestId("send-message").click();

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention]", { hasText: "charlie" });
  await expect(mentionChip).toBeVisible();
  await expect(mentionChip).toHaveText("charlie");
  await expect(mentionChip).toHaveClass(/agent-mention-highlight/);
});

test("mention button opens autocomplete and inserts a selected member", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Hey ");
  await page.getByTestId("message-insert-mention").click();

  const dropdown = autocomplete(page);
  await expect(dropdown).toBeVisible();
  await dropdown.getByText("bob").click();

  await expect(input).toHaveText("Hey @bob ");
});

test("inserting a mention preserves Shift+Enter newlines (regression: bug #2)", async ({
  page,
}) => {
  // Before PR #618, mention insertion round-tripped through
  // `setContent(markdown)`, which collapsed every Shift+Enter hard
  // break to a single space. After the fix, autocomplete uses a
  // native ProseMirror `tr.insertText` transaction and the line
  // breaks survive.
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.click();
  await page.keyboard.type("line one");
  await page.keyboard.press("Shift+Enter");
  await page.keyboard.type("line two @bo");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("bob")).toBeVisible();
  await dropdown.getByText("bob").click();

  // Both lines must still be present, separated by a real line break
  // (rendered as a `<br>` by Tiptap; the projection sees `\n`).
  await expect(input).toHaveText(/line one[\s\S]*line two @bob/);
  await expect(input.locator("br")).toHaveCount(1);
});

test("keyboard navigation selects mention with Enter", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@bo");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("bob")).toBeVisible();

  // Press Enter to select the first (and only) suggestion
  await input.press("Enter");

  // Should insert @bob and NOT send the message
  await expect(input).toHaveText("@bob ");
});

test("Escape dismisses autocomplete dropdown", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@");

  const dropdown = autocomplete(page);
  await expect(dropdown).toBeVisible();

  await input.press("Escape");

  await expect(dropdown).not.toBeVisible();
});

test("mention text is highlighted in sent messages", async ({ page }) => {
  const suffix = ` check this out ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Hey @bo");
  await autocomplete(page).getByText("bob").click();
  await expect(input).toHaveText("Hey @bob ");
  await page.keyboard.type(suffix);
  await page.getByTestId("send-message").click();

  await waitForTimelineSettled(page);

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention].mention-chip", { hasText: "bob" });
  await expect(mentionChip).toBeVisible();
  await expect(mentionChip.locator(".mention-chip-prefix")).toHaveText("@");
  await expect(mentionChip.locator("svg")).toHaveCount(0);
});

test("clicking author name opens user profile panel", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // The seed message in general is from the mock identity (npub1mock...)
  const firstMessage = page.getByTestId("message-row").first();
  const authorButton = firstMessage.locator("button", {
    hasText: "npub1mock...",
  });
  await authorButton.click();

  // Click now opens the full profile panel instead of the popover
  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("deadbeef");
});

test("hovering avatar opens popover, clicking opens profile panel", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const firstMessage = page.getByTestId("message-row").first();
  const avatarButton = firstMessage.locator("button").first();

  // Hover should open the popover
  await avatarButton.hover();
  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();

  // Click should close the popover and open the profile panel
  await avatarButton.click();
  await expect(profilePopover).toHaveCount(0);
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
});

test("clicking a mention chip in the timeline opens the profile panel", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Ping @bob about the launch", {
    mentionPubkeys: [TEST_IDENTITIES.bob.pubkey],
  });
  await waitForTimelineSettled(page);

  const mentionChip = page
    .getByTestId("message-row")
    .filter({ hasText: "Ping @bob about the launch" })
    .locator("[data-mention]", { hasText: "@bob" });
  await expect(mentionChip).toBeVisible();
  await mentionChip.click();

  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("bob");
});

test("mention text matching the kind-0 name alias resolves and opens the profile panel", async ({
  page,
}) => {
  // bob's mock profile has display_name "bob" and kind-0 name "bobby". A
  // message that says "@bobby" (how agents/CLI resolve mentions at send time)
  // must still render a clickable chip bound to bob's pubkey.
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Ask @bobby to review the doc", {
    mentionPubkeys: [TEST_IDENTITIES.bob.pubkey],
  });
  await waitForTimelineSettled(page);

  const mentionChip = page
    .getByTestId("message-row")
    .filter({ hasText: "Ask @bobby to review the doc" })
    .locator("[data-mention]", { hasText: "@bobby" });
  await expect(mentionChip).toBeVisible();
  await mentionChip.click();

  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("bob");
});

test("clicking a mention chip in a forum post opens the profile panel", async ({
  page,
}) => {
  await page.goto("/");
  // Seed the forum post before entering the channel — forum views load from
  // the mock store on fetch, so no live subscription is needed.
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await emitMockMessage(page, "watercooler", "Welcome aboard @bob!", {
    kind: 45001,
    mentionPubkeys: [TEST_IDENTITIES.bob.pubkey],
  });

  await page.getByTestId("channel-watercooler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");

  const mentionChip = page.locator("[data-mention]", { hasText: "@bob" });
  await expect(mentionChip).toBeVisible();
  await mentionChip.click();

  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("bob");
  // The chip click must not bubble into the card and open the thread view.
  await expect(page.getByRole("button", { name: "Back to posts" })).toHaveCount(
    0,
  );
});

test("agent profile popover shows its owner", async ({ page }) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: OWNED_AGENT_PROFILE_PUBKEY,
        displayName: "Bumble",
        ownerPubkey: TEST_IDENTITIES.bob.pubkey,
        isAgent: true,
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Bumble checking in.", {
    pubkey: OWNED_AGENT_PROFILE_PUBKEY,
  });
  await waitForTimelineSettled(page);

  const bumbleMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Bumble checking in." })
    .first();
  await bumbleMessage.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-owner-${OWNED_AGENT_PROFILE_PUBKEY}`,
    ),
  ).toHaveText("managed by bob");
});

test("agent profile popover labels an agent owned by the viewer as you", async ({
  page,
}) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: OWNED_AGENT_PROFILE_PUBKEY,
        displayName: "Bumble",
        ownerPubkey: MOCK_VIEWER_PUBKEY,
        isAgent: true,
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Bumble checking in.", {
    pubkey: OWNED_AGENT_PROFILE_PUBKEY,
  });
  await waitForTimelineSettled(page);

  const bumbleMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Bumble checking in." })
    .first();
  await bumbleMessage.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-owner-${OWNED_AGENT_PROFILE_PUBKEY}`,
    ),
  ).toHaveText("managed by you");
});

test("agent profile popover falls back to the owner's pubkey", async ({
  page,
}) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: OWNED_AGENT_PROFILE_PUBKEY,
        displayName: "Bumble",
        ownerPubkey: CASEY_PROFILE_PUBKEY,
        isAgent: true,
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Bumble checking in.", {
    pubkey: OWNED_AGENT_PROFILE_PUBKEY,
  });
  await waitForTimelineSettled(page);

  const bumbleMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Bumble checking in." })
    .first();
  await bumbleMessage.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-owner-${OWNED_AGENT_PROFILE_PUBKEY}`,
    ),
  ).toHaveText("managed by 11111111…1111");
});

test("human profile popover does not show an owner", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Bob checking in.", {
    pubkey: TEST_IDENTITIES.bob.pubkey,
  });
  await waitForTimelineSettled(page);

  const bobMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Bob checking in." })
    .first();
  await bobMessage.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expect(
    profilePopover.locator('[data-testid^="user-profile-popover-owner-"]'),
  ).toHaveCount(0);
});

test("owned bot profile only exposes message action", async ({ page }) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        name: "charlie",
        status: "online",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");

  const charlieMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Indexing the channel catalog now." })
    .first();
  await charlieMessage.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expectAgentProfileMessageOnly(
    profilePopover,
    TEST_IDENTITIES.charlie.pubkey,
  );
});

test("owned agent mention profile only exposes message action", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        name: "charlie",
        status: "online",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Can @charlie take this?", {
    mentionPubkeys: [TEST_IDENTITIES.charlie.pubkey],
  });
  await waitForTimelineSettled(page);

  const mentionChip = page
    .getByTestId("message-row")
    .filter({ hasText: "Can charlie take this?" })
    .locator("[data-mention].agent-mention-highlight", { hasText: "charlie" });
  await expect(mentionChip).toBeVisible();
  await mentionChip.hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expectAgentProfileMessageOnly(
    profilePopover,
    TEST_IDENTITIES.charlie.pubkey,
  );
});

test("profile popover wave sends a direct message for a human profile", async ({
  page,
}) => {
  await installMockBridge(page, { sendMessageDelayMs: 2_500 });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Bob says hello.", {
    pubkey: TEST_IDENTITIES.bob.pubkey,
  });
  await waitForTimelineSettled(page);

  const bobMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Bob says hello." })
    .first();
  await bobMessage.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-message-${TEST_IDENTITIES.bob.pubkey}`,
    ),
  ).toBeVisible();
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-huddle-${TEST_IDENTITIES.bob.pubkey}`,
    ),
  ).toBeVisible();
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-wave-${TEST_IDENTITIES.bob.pubkey}`,
    ),
  ).toBeVisible();
  await profilePopover
    .getByTestId(`user-profile-popover-wave-${TEST_IDENTITIES.bob.pubkey}`)
    .click();

  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");
  const waveAttachment = page.getByTestId("message-wave-attachment");
  await expect(waveAttachment).toBeVisible({ timeout: 1_500 });
  await expect(page.getByText("Sending")).toHaveCount(0, { timeout: 4_000 });
  await waitForTimelineSettled(page);
  await expect(waveAttachment).toContainText("👋");
  await expect(waveAttachment).toContainText("npub1mock... waved at you.");
  await expect(waveAttachment).toContainText("Start a huddle to talk to them.");
  await expect(
    waveAttachment.getByRole("button", { name: "Start huddle" }),
  ).toBeVisible();

  const commandLog = await readCommandPayloadLog(page);
  expect(commandLog).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        command: "send_channel_message",
        payload: expect.objectContaining({
          content: expect.stringContaining("npub1mock... waved at you."),
        }),
      }),
    ]),
  );
  expect(commandLog).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        command: "send_channel_message",
        payload: expect.objectContaining({
          content: expect.stringContaining("<!-- buzz:wave:v1 -->"),
        }),
      }),
    ]),
  );
});

test("delayed inaccessible agent profile keeps all actions hidden", async ({
  page,
}) => {
  await installMockBridge(page, {
    agentListDelayMs: 5_000,
    relayAgents: [
      {
        pubkey: DELAYED_RELAY_AGENT_PUBKEY,
        name: "orbit",
        channelNames: ["general"],
      },
    ],
    searchProfiles: [
      {
        pubkey: DELAYED_RELAY_AGENT_PUBKEY,
        displayName: "orbit",
      },
    ],
  });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Orbit checking in.", {
    pubkey: DELAYED_RELAY_AGENT_PUBKEY,
  });
  await waitForTimelineSettled(page);

  const orbitMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Orbit checking in." })
    .first();
  await orbitMessage.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-message-${DELAYED_RELAY_AGENT_PUBKEY}`,
    ),
  ).toHaveCount(0);
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-wave-${DELAYED_RELAY_AGENT_PUBKEY}`,
    ),
  ).toHaveCount(0);
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-huddle-${DELAYED_RELAY_AGENT_PUBKEY}`,
    ),
  ).toHaveCount(0);
});
