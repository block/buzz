import { expect, test } from "@playwright/test";

import {
  installMockBridge,
  openChannelBrowser,
  TEST_IDENTITIES,
} from "../helpers/bridge";

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
const CASEY_PROFILE_PUBKEY =
  "1111111111111111111111111111111111111111111111111111111111111111";
const CASEY_PILOT_PROFILE_PUBKEY =
  "2222222222222222222222222222222222222222222222222222222222222222";
const SYSTEM_MESSAGE_KIND = 40099;

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

async function readCommandPayloads(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    return (
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{
            command: string;
            payload: unknown;
          }>;
        }
      ).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []
    );
  });
}

function commandCount(commands: string[], command: string) {
  return commands.filter((entry) => entry === command).length;
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

test("@ trigger shows unified autocomplete with agents first", async ({
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
  await expect(dropdown.getByText("outsider")).toBeVisible();
  const charlieRow = dropdown.locator("button", { hasText: "charlie" });
  const outsiderRow = dropdown.locator("button", { hasText: "outsider" });
  await expect(charlieRow.getByTestId("mention-agent-icon")).toBeVisible();
  await expect(charlieRow.getByText("not in channel")).toBeVisible();
  await expect(outsiderRow.getByText("not in channel")).toBeVisible();
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
  expect(outsiderIndex).toBeGreaterThanOrEqual(0);
  expect(fizzIndex).toBeLessThan(aliceIndex);
  expect(fizzIndex).toBeLessThan(bobIndex);
  expect(aliceIndex).toBeLessThan(charlieIndex);
  expect(bobIndex).toBeLessThan(charlieIndex);
  expect(aliceIndex).toBeLessThan(outsiderIndex);
  expect(bobIndex).toBeLessThan(outsiderIndex);
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

test("autocomplete stays open while expanded search results load", async ({
  page,
}) => {
  await installMockBridge(page, { userSearchDelayMs: 1_000 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@");

  const dropdown = autocomplete(page);
  await expect(dropdown).toBeVisible();
  await expect(dropdown.getByText("alice")).toBeVisible();

  await input.fill("@zzzz");
  await page.waitForTimeout(250);

  await expect(dropdown).toBeVisible();
  await expect(dropdown.getByText("alice")).toBeVisible();
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

test("profile-only agents without public respond-to are hidden from mentions", async ({
  page,
}) => {
  await installMockBridge(page, { userSearchDelayMs: 1_000 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@mira");

  const dropdown = autocomplete(page);
  await expect(dropdown).not.toBeVisible();
  await expect(input.locator(".mention-chip")).toHaveCount(0);
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

test("system add and remove rows use agent mention styling for managed agents", async ({
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
    .filter({ hasText: "added portal to the channel" });
  const removedRow = page
    .getByTestId("system-message-row")
    .filter({ hasText: "removed portal from the channel" });

  await expect(
    addedRow.locator("[data-mention].agent-mention-highlight", {
      hasText: "portal",
    }),
  ).toHaveText("portal");
  await expect(
    removedRow.locator("[data-mention].agent-mention-highlight", {
      hasText: "portal",
    }),
  ).toHaveText("portal");
});

test("system member-joined rows render the joined person as a mention chip", async ({
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
  const joinedPersonChip = joinedRow.locator("[data-mention].mention-chip", {
    hasText: "bob",
  });

  await expect(joinedPersonChip).toBeVisible();
  await expect(joinedPersonChip).toHaveCSS("display", /^(inline-)?flex$/);
  await expect(joinedPersonChip).not.toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await expect(joinedPersonChip.locator(".mention-chip-prefix")).toHaveText(
    "@",
  );
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

test("do nothing sends a non-member mention without inviting", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Loop in @out");
  const initialMessageCount = await page.getByTestId("message-row").count();
  const initialAddMemberCount = commandCount(
    await readCommandLog(page),
    "add_channel_members",
  );

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("outsider")).toBeVisible();
  await input.press("Enter");
  await page.getByTestId("send-message").click();

  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("outsider");
  await expect(
    dialog.getByRole("button", { name: "Do nothing" }),
  ).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Invite" })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Cancel" })).toHaveCount(0);
  await expect(
    dialog.getByRole("button", { name: "Reference only" }),
  ).toHaveCount(0);
  await expect(dialog.getByRole("button", { name: "Notify" })).toHaveCount(0);

  await dialog.getByRole("button", { name: "Do nothing" }).click();
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  await expect(page.getByTestId("message-row")).toHaveCount(
    initialMessageCount + 1,
  );
  await expect(input).toHaveText("");

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention]", { hasText: "@outsider" });
  await expect(mentionChip).toBeVisible();
  await expect(mentionChip.locator("svg")).toHaveCount(0);
  expect(
    commandCount(await readCommandLog(page), "add_channel_members"),
  ).toEqual(initialAddMemberCount);

  await mentionChip.click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
  await expect(page.getByTestId("user-profile-panel")).toContainText(
    "outsider",
  );
});

test("invite action adds non-member before sending mention", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Loop in @out");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("outsider")).toBeVisible();
  await input.press("Enter");
  await page.getByTestId("send-message").click();

  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Invite" }).click();

  await expect
    .poll(async () => {
      const commands = await readCommandLog(page);
      return commands.filter((command) => command === "add_channel_members")
        .length;
    })
    .toBeGreaterThan(0);

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention]", { hasText: "@outsider" });
  await expect(mentionChip).toBeVisible();
  await expect(mentionChip.locator("svg")).toHaveCount(0);
});

test("invite action only adds the selected non-member profile", async ({
  page,
}) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: CASEY_PROFILE_PUBKEY,
        displayName: "casey",
      },
      {
        pubkey: CASEY_PILOT_PROFILE_PUBKEY,
        displayName: "casey pilot",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  const dropdown = autocomplete(page);
  await input.fill("Loop in @");
  await expect(dropdown.getByText("casey pilot")).toBeVisible();
  await input.fill("Loop in @casey p");
  await expect(dropdown.getByText("casey pilot")).toBeVisible();
  await dropdown.getByText("casey pilot").click();
  await page.getByTestId("send-message").click();

  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("casey pilot");
  await dialog.getByRole("button", { name: "Invite" }).click();

  await expect
    .poll(async () => {
      return readCommandPayloads(page);
    })
    .toContainEqual(
      expect.objectContaining({
        command: "add_channel_members",
        payload: expect.objectContaining({
          pubkeys: [CASEY_PILOT_PROFILE_PUBKEY],
        }),
      }),
    );
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
