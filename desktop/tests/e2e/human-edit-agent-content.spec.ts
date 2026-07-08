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

// Mock-bridge helper: wait for the bridge to initialise, then invoke a command.
async function invoke(
  page: import("@playwright/test").Page,
  command: string,
  payload?: Record<string, unknown>,
): Promise<unknown> {
  await page.waitForFunction(() =>
    Boolean(
      (
        window as Window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__,
    ),
  );
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

// Open the more-actions menu for a message row and wait for the menu to mount.
async function openMoreActionsMenu(
  page: import("@playwright/test").Page,
  messageId: string,
) {
  const row = page.locator(`[data-message-id="${messageId}"]`);
  await row.hover();
  await page.getByTestId(`more-actions-${messageId}`).click();
  // Wait for dropdown content to mount — any menu item signals it's open.
  await expect(page.locator('[role="menuitem"]').first()).toBeVisible({
    timeout: 5_000,
  });
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

test("owner can edit their owned agent's message", async ({ page }) => {
  // The bridge seeds a message from each managed agent in its channels:
  //   id: `mock-agents-managed-${pubkey.slice(0, 8)}`
  const messageId = `mock-agents-managed-${OWNED_AGENT_PUBKEY.slice(0, 8)}`;
  const editedContent = `Edited by owner ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");

  // Wait for the agent's seeded message to appear.
  const agentRow = page.locator(`[data-message-id="${messageId}"]`);
  await expect(agentRow).toBeVisible({ timeout: 10_000 });

  // Open the more-actions menu and click Edit message.
  await openMoreActionsMenu(page, messageId);
  await page.getByTestId(`edit-message-${messageId}`).click();

  // Edit banner must appear confirming edit mode is active.
  await expect(page.getByTestId("edit-target")).toBeVisible({ timeout: 5_000 });

  // Wait for the editor to be populated with the original message content
  // (edit mode calls richText.setContent which is async in Tiptap's
  // transaction pipeline). Then select-all and type the replacement.
  const input = page.getByTestId("message-input");
  await expect(input).not.toBeEmpty({ timeout: 5_000 });
  await input.click();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.type(editedContent);
  await page.keyboard.press("Enter");

  // Edit mode must exit (banner gone) and the updated content must render.
  await expect(page.getByTestId("edit-target")).toBeHidden();
  await expect(page.getByTestId("message-timeline")).toContainText(
    editedContent,
  );
});

test("owner can delete their owned agent's message", async ({ page }) => {
  const messageId = `mock-agents-managed-${OWNED_AGENT_PUBKEY.slice(0, 8)}`;

  await page.goto("/");
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");

  const agentRow = page.locator(`[data-message-id="${messageId}"]`);
  await expect(agentRow).toBeVisible({ timeout: 10_000 });

  // Open the more-actions menu and click Delete message.
  await openMoreActionsMenu(page, messageId);
  await page.getByTestId(`delete-message-${messageId}`).click();

  // Confirm the deletion in the AlertDialog.
  await expect(page.getByRole("alertdialog")).toBeVisible({ timeout: 5_000 });
  // The destructive confirm button inside the dialog.
  await page
    .getByRole("alertdialog")
    .getByRole("button", { name: "Delete" })
    .click();

  // The message row must be removed from the timeline.
  await expect(agentRow).toBeHidden({ timeout: 5_000 });
});

test("owner does NOT see Edit or Delete for an unowned agent's message", async ({
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

  // Open the more-actions menu — it must open without Edit or Delete items.
  await charlieRow.hover();
  await page.getByTestId(`more-actions-${charlieMessageId}`).click();
  await expect(page.locator('[role="menuitem"]').first()).toBeVisible({
    timeout: 5_000,
  });

  await expect(
    page.getByTestId(`edit-message-${charlieMessageId}`),
  ).toHaveCount(0);
  await expect(
    page.getByTestId(`delete-message-${charlieMessageId}`),
  ).toHaveCount(0);
});

// ─── Thread-panel gate ────────────────────────────────────────────────────────

test("owner can delete their owned agent's message from the thread panel", async ({
  page,
}) => {
  // The bridge seeds a message from each managed agent in its channels:
  //   id: `mock-agents-managed-${pubkey.slice(0, 8)}`
  const messageId = `mock-agents-managed-${OWNED_AGENT_PUBKEY.slice(0, 8)}`;

  await page.goto("/");
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");

  // Wait for the agent's seeded message to appear in the main timeline.
  const agentRow = page.locator(`[data-message-id="${messageId}"]`).first();
  await expect(agentRow).toBeVisible({ timeout: 10_000 });

  // Open the thread panel by hovering the message and clicking Reply.
  await agentRow.hover();
  await agentRow.getByRole("button", { name: "Reply" }).click();

  // Wait for the thread panel and confirm the thread head contains the agent message.
  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible({ timeout: 5_000 });
  const threadHead = threadPanel.getByTestId("message-thread-head");
  await expect(
    threadHead.locator(`[data-message-id="${messageId}"]`),
  ).toBeVisible({
    timeout: 5_000,
  });

  // Open the more-actions menu for the thread head from inside the thread panel.
  const headRow = threadHead.locator(`[data-message-id="${messageId}"]`);
  await headRow.hover();
  await threadHead.getByTestId(`more-actions-${messageId}`).click();
  // Wait for the dropdown to mount.
  await expect(page.locator('[role="menuitem"]').first()).toBeVisible({
    timeout: 5_000,
  });

  // Click Delete.
  // Radix dropdown portals render outside the thread-panel DOM boundary, so we
  // query from the full page. Only one menu is open, making the testId unique.
  await page.getByTestId(`delete-message-${messageId}`).click();

  // The AlertDialog must appear. This proves canManageMessageForCurrentUser is
  // wired into MessageThreadPanel.tsx — the delete handler is reachable from
  // the thread-panel path (i.e. the thread-panel permission gate is not
  // accidentally more restrictive than the main-timeline gate). The full
  // delete-and-cache-invalidation path is already covered by the main-timeline
  // delete test above; asserting the dialog here is sufficient as a thread-panel
  // regression guard.
  await expect(page.getByRole("alertdialog")).toBeVisible({ timeout: 5_000 });

  // Dismiss without confirming — leaves the fixture clean for any subsequent steps.
  await page.keyboard.press("Escape");
  await expect(page.getByRole("alertdialog")).not.toBeVisible({
    timeout: 3_000,
  });
});

// ─── Channel management gate ──────────────────────────────────────────────────

test("owner can edit channel name via owned-agent-owner path", async ({
  page,
}) => {
  const newChannelName = `renamed-${Date.now()}`;

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
  const editButton = page.getByTestId("channel-management-edit");
  await expect(editButton).toBeVisible({ timeout: 5_000 });
  await editButton.click();

  // Change the channel name and save.
  const nameInput = page.getByTestId("channel-management-name");
  await expect(nameInput).toBeVisible({ timeout: 5_000 });
  await nameInput.clear();
  await nameInput.fill(newChannelName);
  await page.getByTestId("channel-management-save-changes").click();

  // The edit dialog must close and the updated name must appear in the header.
  await expect(page.getByTestId("channel-management-name")).toBeHidden();
  await expect(page.getByTestId("chat-title")).toHaveText(newChannelName);
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
