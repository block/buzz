import { expect, test, bootstrapE2ePage } from "../helpers/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/agent-access-warning";

async function choosePersonaAccess(
  page: import("@playwright/test").Page,
  optionName: string,
) {
  await page.locator("#agent-respond-to").click();
  await page.getByRole("menuitemradio", { name: optionName }).click();
}

async function openAgentAccessDialog(
  page: import("@playwright/test").Page,
  agentPubkey: string,
) {
  if (!(await page.getByTestId("members-sidebar").isVisible())) {
    await page.getByTestId("channel-general").click();
    await page.getByTestId("channel-members-trigger").click();
    await expect(page.getByTestId("members-sidebar")).toBeVisible();
  }

  const row = page.getByTestId(`sidebar-member-${agentPubkey}`);
  const menu = page.getByTestId(`sidebar-member-menu-${agentPubkey}`);
  await row.hover();
  await menu.focus();
  await menu.press("Enter");
  await page.getByTestId(`sidebar-edit-respond-to-${agentPubkey}`).click();

  await expect(
    page.getByRole("dialog", { name: "Manage agent access" }),
  ).toBeVisible();
}

test("open agent access explains the available access before save", async ({
  page,
}) => {
  const agent = TEST_IDENTITIES.charlie;
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: agent.pubkey,
        name: "Hack Day Helper",
        status: "running",
        channelNames: ["general"],
        respondTo: "anyone",
      },
    ],
  });
  await bootstrapE2ePage(page, "/");
  await page.getByTestId("channel-general").click();
  await page.getByTestId("channel-members-trigger").click();
  const accessBadge = page.getByTestId(
    `sidebar-managed-agent-respond-to-${agent.pubkey}`,
  );
  await expect(accessBadge).toBeVisible();
  await expect(accessBadge).toHaveText("Anyone");
  await openAgentAccessDialog(page, agent.pubkey);

  const accessSelect = page.getByTestId("agent-respond-to-select");
  await expect(accessSelect).toHaveValue("anyone");
  const saveAccess = page.getByRole("button", { name: "Save access" });
  await expect(saveAccess).toBeVisible();

  const commandsBeforeSave = await page.evaluate(
    () => window.__BUZZ_E2E_COMMAND_LOG__?.length ?? 0,
  );
  await accessSelect.selectOption("owner-only");
  await expect(page.getByTestId("agent-access-warning")).toHaveCount(0);

  await waitForAnimations(page);
  await page
    .getByRole("dialog", { name: "Manage agent access" })
    .screenshot({ path: `${SHOTS}/open-access-warning.png` });

  await saveAccess.click();
  await expect(
    page.getByRole("dialog", { name: "Manage agent access" }),
  ).not.toBeVisible();
  await expect
    .poll(async () =>
      page.evaluate((start) => {
        const commands = window.__BUZZ_E2E_COMMAND_LOG__ ?? [];
        return commands
          .slice(start)
          .some(
            (entry) =>
              entry.command === "update_managed_agent" &&
              (entry.payload as { input?: { respondTo?: string } })?.input
                ?.respondTo === "owner-only",
          );
      }, commandsBeforeSave),
    )
    .toBe(true);
  await expect(accessBadge).toHaveText("Only me");

  await openAgentAccessDialog(page, agent.pubkey);
  await expect(accessSelect).toHaveValue("owner-only");
  // Selected people narrows the audience but not the access, so the warning
  // persists with its own audience phrase.
  await accessSelect.selectOption("allowlist");
  const warning = page.getByTestId("agent-access-warning");
  await expect(warning).toBeVisible();
  await expect(warning).toContainText(
    "Selected people can use this agent to access your computer, including files, accounts, and connected tools.",
  );
  const picker = page.getByTestId("agent-respond-to-allowlist");
  await expect(
    picker.getByText("Selected people", { exact: true }),
  ).toBeVisible();

  // The warning sits below the picker so it never blocks the selection the
  // user came here to make.
  await waitForAnimations(page);
  const pickerBox = await picker.boundingBox();
  const warningBox = await warning.boundingBox();
  expect(pickerBox?.y).toBeDefined();
  expect(warningBox?.y).toBeGreaterThan(pickerBox?.y ?? 0);
  await page
    .getByRole("dialog", { name: "Manage agent access" })
    .screenshot({ path: `${SHOTS}/selected-people-warning.png` });

  // Only me shares nothing, so the warning goes away entirely.
  await accessSelect.selectOption("owner-only");
  await expect(warning).toHaveCount(0);
});

test("full agent editor tightens the exact sidebar agent instance", async ({
  page,
}) => {
  const agent = TEST_IDENTITIES.tyler;
  const preferredSiblingPubkey = "d".repeat(64);
  const personaId = "shared-sidebar-agent";
  await installMockBridge(page, {
    acpRuntimesCatalog: [
      {
        availability: "available",
        command: "goose",
        default_args: [],
        id: "goose",
        install_hint: "",
        label: "Goose",
        mcp_command: "",
      },
    ],
    globalAgentConfig: {
      env_vars: { ANTHROPIC_API_KEY: "sk-ant-test-key" },
      model: "claude-opus-4-5",
      preferred_runtime: "goose",
      provider: "anthropic",
    },
    managedAgents: [
      {
        pubkey: agent.pubkey,
        name: "Tyler Agent",
        personaId,
        status: "running",
        channelNames: ["general"],
        respondTo: "anyone",
      },
      {
        pubkey: preferredSiblingPubkey,
        name: "Preferred Sibling",
        personaId,
        status: "running",
        channelNames: [],
        respondTo: "anyone",
      },
    ],
    personas: [
      {
        displayName: "Shared Sidebar Agent",
        id: personaId,
        isActive: true,
        respondTo: "anyone",
        runtime: "goose",
        systemPrompt: "Test exact instance editing.",
      },
    ],
  });
  await bootstrapE2ePage(page, "/");
  await page.getByTestId("channel-general").click();
  await page.getByTestId("channel-members-trigger").click();
  const accessBadge = page.getByTestId(
    `sidebar-managed-agent-respond-to-${agent.pubkey}`,
  );
  await expect(accessBadge).toHaveText("Anyone");

  await page.getByTestId(`sidebar-member-${agent.pubkey}`).click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
  await page.getByTestId("user-profile-edit-agent").click();
  const dialog = page.getByRole("dialog", { name: "Edit agent" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Advanced" }).click();
  await choosePersonaAccess(page, "Only me (default)");
  await dialog.getByRole("tab", { name: "Customize for this agent" }).click();
  const saveChanges = dialog.getByRole("button", { name: "Save changes" });
  await expect(saveChanges).toBeEnabled();
  await saveChanges.click();
  await expect(dialog).not.toBeVisible();

  const updateCommand = await page.evaluate(
    (pubkey) =>
      window.__BUZZ_E2E_COMMAND_LOG__?.findLast(
        (entry) =>
          entry.command === "update_managed_agent" &&
          (entry.payload as { input?: { pubkey?: string } })?.input?.pubkey ===
            pubkey,
      ),
    agent.pubkey,
  );
  expect(updateCommand?.payload).toMatchObject({
    input: { pubkey: agent.pubkey, respondTo: "owner-only" },
  });
  expect(updateCommand?.payload).not.toMatchObject({
    input: { pubkey: preferredSiblingPubkey },
  });

  await page.getByTestId("channel-members-trigger").click();
  await expect(accessBadge).toHaveText("Only me");

  // The definition still says "anyone" after the instance-only save above.
  // Reopening the same linked agent for an unrelated prompt edit must seed
  // access from the exact instance, or the submit silently widens it again.
  await page.getByTestId(`sidebar-member-${agent.pubkey}`).click();
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Advanced" }).click();
  await expect(page.locator("#agent-respond-to")).toHaveText(
    "Only me (default)",
  );
  await page
    .locator("#persona-system-prompt")
    .fill("Test unrelated prompt editing after tightening access.");
  await dialog.getByRole("button", { name: "Save changes" }).click();
  await expect(dialog).not.toBeVisible();

  const unrelatedEditCommand = await page.evaluate(
    (pubkey) =>
      window.__BUZZ_E2E_COMMAND_LOG__?.findLast(
        (entry) =>
          entry.command === "update_managed_agent" &&
          (entry.payload as { input?: { pubkey?: string } })?.input?.pubkey ===
            pubkey,
      ),
    agent.pubkey,
  );
  expect(unrelatedEditCommand?.payload).toMatchObject({
    input: { pubkey: agent.pubkey },
  });
  expect(unrelatedEditCommand?.payload).not.toMatchObject({
    input: { respondTo: "anyone" },
  });
});

test("a provider-backed agent's warning names the server, not this computer", async ({
  page,
}) => {
  const agent = TEST_IDENTITIES.charlie;
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: agent.pubkey,
        name: "Remote Helper",
        status: "running",
        channelNames: ["general"],
        respondTo: "owner-only",
        backend: { type: "provider", id: "blox", config: {} },
      },
    ],
  });
  await bootstrapE2ePage(page, "/");
  await openAgentAccessDialog(page, agent.pubkey);

  await page.getByTestId("agent-respond-to-select").selectOption("anyone");
  const warning = page.getByTestId("agent-access-warning");
  await expect(warning).toContainText(
    "Anyone can use this agent to access the server it runs on, including any accounts and tools available there.",
  );
  // The local wording must not leak into a remote-backed agent.
  await expect(warning).not.toContainText("your computer");
});

test("persona-backed edit warns before saving open access", async ({
  page,
}) => {
  const agent = TEST_IDENTITIES.tyler;
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: agent.pubkey,
        name: "Tyler Agent",
        status: "stopped",
        channelNames: ["agents"],
        respondTo: "owner-only",
      },
    ],
  });
  await bootstrapE2ePage(page, "/");
  await page.getByTestId("open-agents-view").click();
  await page.getByRole("button", { name: "Tyler Agent agent profile" }).click();
  await page.getByTestId("user-profile-edit-agent").click();

  const dialog = page.getByTestId("edit-agent-dialog");
  await expect(dialog).toBeVisible();
  await expect(page.locator("#agent-respond-to")).toHaveText(
    "Only me (default)",
  );
  await choosePersonaAccess(page, "Anyone");
  await expect(dialog.getByTestId("agent-access-warning")).toContainText(
    "Anyone can use this agent to access your computer, including files, accounts, and connected tools.",
  );

  const commandsBeforeSave = await page.evaluate(
    () => window.__BUZZ_E2E_COMMAND_LOG__?.length ?? 0,
  );
  await page.getByTestId("edit-agent-dialog-submit").click();
  await expect(dialog).not.toBeVisible();
  await expect
    .poll(async () =>
      page.evaluate((start) => {
        const commands = window.__BUZZ_E2E_COMMAND_LOG__ ?? [];
        return commands
          .slice(start)
          .some(
            (entry) =>
              entry.command === "update_managed_agent" &&
              (entry.payload as { input?: { respondTo?: string } })?.input
                ?.respondTo === "anyone",
          );
      }, commandsBeforeSave),
    )
    .toBe(true);
});
