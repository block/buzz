/**
 * E2E regression for the wrong-channel send bug.
 *
 * Repro: compose a message in channel A that tags a non-member managed agent
 * (which forces the slow `add_channel_members` path), submit, then immediately
 * switch to channel B before the agent-attach await resolves. Without the fix,
 * the message lands in B's timeline; with the fix it must land in A's.
 *
 * The `addChannelMembersDelayMs` bridge knob holds the `add_channel_members`
 * handler open long enough for the channel click to race the in-flight send.
 */

import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// A managed agent that is NOT a member of any channel in the seed data.
const OUT_OF_CHANNEL_BOT_PUBKEY =
  "ee00000000000000000000000000000000000000000000000000000000000001";

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

function commandCount(commands: string[], command: string) {
  return commands.filter((c) => c === command).length;
}

// The channel timeline renders off a `useDeferredValue` snapshot; poll for the
// pending marker to clear before asserting on freshly-sent content.
async function waitForTimelineSettled(page: import("@playwright/test").Page) {
  await expect(page.locator("[data-render-pending]")).toHaveCount(0);
}

// ---------------------------------------------------------------------------
// Main regression: message always lands in the compose-time channel
// ---------------------------------------------------------------------------

test("message with agent mention lands in compose-time channel despite mid-send navigation", async ({
  page,
}) => {
  const MESSAGE_TEXT = `send-binding-repro-${Date.now()}`;

  // Install bridge with:
  //   - a managed agent that is NOT in general (forces add_channel_members path)
  //   - a 500ms delay on add_channel_members to open the race window
  await installMockBridge(page, {
    addChannelMembersDelayMs: 500,
    managedAgents: [
      {
        pubkey: OUT_OF_CHANNEL_BOT_PUBKEY,
        name: "BotA",
        status: "running",
        // No channelNames → agent is not a member of any channel
      },
    ],
  });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Type a message that mentions the out-of-channel agent
  const input = page.getByTestId("message-input");
  await input.fill("@BotA");

  const dropdown = autocomplete(page);
  const botRow = dropdown.locator("button", { hasText: "BotA" });
  await expect(botRow).toBeVisible();
  await expect(botRow.getByText("not in channel")).toBeVisible();
  // Select BotA from the autocomplete
  await input.press("Enter");
  await page.keyboard.type(` ${MESSAGE_TEXT}`);

  // Verify the mention chip is present before submitting
  const composerChip = input.locator(".agent-mention-highlight", {
    hasText: "BotA",
  });
  await expect(composerChip).toBeVisible();

  // Snapshot the baseline command count before sending
  const baselineCommands = await readCommandLog(page);
  const baselineAddCount = commandCount(
    baselineCommands,
    "add_channel_members",
  );

  // Submit the message — this triggers the async add_channel_members path
  await page.getByTestId("send-message").click();

  // Immediately switch to channel-agents BEFORE the 500ms delay resolves.
  // This is the race the fix closes.
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");

  // Wait for add_channel_members to fire (confirms the race window opened and
  // the fix's captured channel id was used for the agent-attach call).
  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "add_channel_members"),
    )
    .toBeGreaterThan(baselineAddCount);

  // Let the in-flight send finish (500ms delay + buffer).
  await page.waitForTimeout(800);

  // --- Assert message landed in general (compose-time channel) ---
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForTimelineSettled(page);

  // The message must appear in general's timeline (not the switched-to channel).
  await expect(page.getByTestId("message-timeline")).toContainText(
    MESSAGE_TEXT,
  );

  // --- Assert message did NOT land in agents (switched-to channel) ---
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");
  await waitForTimelineSettled(page);

  await expect(page.getByTestId("message-timeline")).not.toContainText(
    MESSAGE_TEXT,
  );
});

// ---------------------------------------------------------------------------
// Invariant: without mid-send navigation, normal agent-mention send still works
// ---------------------------------------------------------------------------

test("message with agent mention delivers correctly when no channel switch occurs", async ({
  page,
}) => {
  const MESSAGE_TEXT = `no-switch-verify-${Date.now()}`;

  await installMockBridge(page, {
    addChannelMembersDelayMs: 0,
    managedAgents: [
      {
        pubkey: OUT_OF_CHANNEL_BOT_PUBKEY,
        name: "BotA",
        status: "running",
      },
    ],
  });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@BotA");
  const dropdown = autocomplete(page);
  await expect(dropdown.locator("button", { hasText: "BotA" })).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(` ${MESSAGE_TEXT}`);

  const baselineCommands = await readCommandLog(page);
  const baselineAddCount = commandCount(
    baselineCommands,
    "add_channel_members",
  );

  await page.getByTestId("send-message").click();

  // Wait for the agent-attach step to complete before asserting the timeline.
  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "add_channel_members"),
    )
    .toBeGreaterThan(baselineAddCount);

  await waitForTimelineSettled(page);
  await expect(page.getByTestId("message-timeline")).toContainText(
    MESSAGE_TEXT,
  );
});
