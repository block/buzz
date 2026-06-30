import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Fixed pubkey for the owned managed agent seeded in these tests.
// Must not collide with any existing e2eBridge constant.
const OWNED_AGENT_PUBKEY =
  "a0b1c2d3e4f5061728394a5b6c7d8e9f0a1b2c3d4e5f6071829304a5b6c7d8e";

// #random is owned by alice; the mock identity is a plain member.
// This is the isolation fixture for the canManageOwnedAgentChannel path —
// selfMember.role is "member" not "owner", so without the new gate the
// Edit button would not appear.
const RANDOM_CHANNEL_ID = "9dae0116-799b-5071-a0a8-fdd30a91a35d";

// Mock-bridge helper: call a bridge command from within the live page context.
async function invoke(
  page: import("@playwright/test").Page,
  command: string,
  payload?: Record<string, unknown>,
): Promise<unknown> {
  return page.evaluate(
    async ({ cmd, p }) => {
      const win = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
      const fn = win.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      if (!fn) throw new Error("Mock bridge unavailable");
      return fn(cmd, p);
    },
    { cmd: command, p: payload },
  );
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        // OwnedBot: a managed agent owned by the mock identity.
        // The bridge automatically sets owner_pubkey = MOCK_IDENTITY_PUBKEY
        // in mockProfiles when a managed agent is seeded.
        pubkey: OWNED_AGENT_PUBKEY,
        name: "OwnedBot",
        personaId: "builtin:fizz",
        status: "running",
        // Seed into #agents so the bridge seeds a message from this agent.
        channelNames: ["agents"],
      },
    ],
  });
});

// ─── Message gate ─────────────────────────────────────────────────────────────

test("owner sees Edit and Delete on their owned agent's message", async ({
  page,
}) => {
  // The bridge seeds a message from each managed agent in its channels:
  //   id: `mock-agents-managed-${pubkey.slice(0, 8)}`
  const messageId = `mock-agents-managed-${OWNED_AGENT_PUBKEY.slice(0, 8)}`;

  await page.goto("/");
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");

  // Wait for the agent's seeded message to appear.
  const agentRow = page.locator(`[data-message-id="${messageId}"]`);
  await expect(agentRow).toBeVisible({ timeout: 10_000 });

  // Hover to surface the action bar.
  await agentRow.hover();

  await expect(page.getByTestId(`edit-message-${messageId}`)).toBeVisible({
    timeout: 5_000,
  });
  await expect(page.getByTestId(`delete-message-${messageId}`)).toBeVisible({
    timeout: 5_000,
  });
});

test("owner does NOT see Edit or Delete on an unowned agent's message", async ({
  page,
}) => {
  // "mock-agents-charlie" is seeded in #agents for CHARLIE_PUBKEY.
  // Charlie is in mockAgentPubkeys but ownerPubkey is NOT the mock identity.
  const charlieMessageId = "mock-agents-charlie";

  await page.goto("/");
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");

  const charlieRow = page.locator(`[data-message-id="${charlieMessageId}"]`);
  await expect(charlieRow).toBeVisible({ timeout: 10_000 });

  // Hover the row — the action bar will render but edit/delete must be absent.
  await charlieRow.hover();

  // Allow the action bar opacity transition to settle before asserting.
  await page.waitForTimeout(300);

  await expect(
    page.getByTestId(`edit-message-${charlieMessageId}`),
  ).toBeHidden();
  await expect(
    page.getByTestId(`delete-message-${charlieMessageId}`),
  ).toBeHidden();
});

// ─── Channel management gate ──────────────────────────────────────────────────

test("owner sees channel Edit button when their agent is a channel owner", async ({
  page,
}) => {
  await page.goto("/");

  // Add OwnedBot as an owner-role member of #random.
  // In #random the mock identity is only a plain member — selfMember.role !== "owner".
  // This isolates canManageOwnedAgentChannel as the sole reason Edit appears.
  await invoke(page, "add_channel_members", {
    channelId: RANDOM_CHANNEL_ID,
    pubkeys: [OWNED_AGENT_PUBKEY],
    role: "owner",
  });

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-management-sheet")).toBeVisible();

  // Edit quick-action must be visible: canManageOwnedAgentChannel is true.
  await expect(page.getByTestId("channel-management-edit")).toBeVisible({
    timeout: 5_000,
  });
});

test("owner does NOT see channel Edit button when no owned agent is a channel owner", async ({
  page,
}) => {
  await page.goto("/");

  // #random has alice as owner; mock identity is a plain member.
  // OwnedBot is NOT added as owner in this test — Edit must not appear.
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-management-sheet")).toBeVisible();

  await expect(page.getByTestId("channel-management-edit")).toBeHidden();
});
