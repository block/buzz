import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, openCreateChannelDialog } from "../helpers/bridge";

const SHOTS = "test-results/welcome-agent-modal";
const FIZZ_PUBKEY = "f".repeat(64);
const SCOUT_PUBKEY = "a".repeat(64);
const EDITOR_PUBKEY = "b".repeat(64);

const scoutPersona = {
  id: "personal:scout",
  displayName: "Scout",
  systemPrompt: "Research a topic, verify sources, and return a concise brief.",
};
const editorPersona = {
  id: "personal:editor",
  displayName: "Editor",
  systemPrompt: "Turn rough writing into clear, confident prose.",
};

async function openChannel(page: Page, name: string) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId(`channel-${name}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(name);
}

async function readCommandLog(page: Page) {
  return page.evaluate(() => {
    const commands = (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
      .__BUZZ_E2E_COMMANDS__;
    if (!commands) {
      throw new Error("E2E bridge command log is not installed");
    }
    return commands;
  });
}

function commandCount(commands: string[], command: string) {
  return commands.filter((entry) => entry === command).length;
}

async function openAgentPicker(page: Page, channel = "random") {
  await openChannel(page, channel);
  await page.getByTestId("channel-intro-action-create-agent").click();
  const dialog = page.getByTestId("add-channel-bot-dialog");
  await expect(dialog).toBeVisible();
  await waitForAnimations(page);
  return dialog;
}

test.describe("welcome and channel agent entry points", () => {
  test("welcome offers chat-first creation", async ({ page }) => {
    await installMockBridge(page, {
      activePersonaIds: ["builtin:fizz"],
      managedAgents: [
        {
          pubkey: FIZZ_PUBKEY,
          name: "Fizz",
          personaId: "builtin:fizz",
          status: "running",
          channelNames: ["Welcome"],
        },
      ],
    });
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openCreateChannelDialog(page);
    await page.getByTestId("create-channel-name").fill("Welcome");
    await page
      .getByTestId("create-channel-description")
      .fill("A private channel for getting oriented in this workspace.");
    await page.getByTestId("create-channel-private-toggle").click();
    await page.getByTestId("create-channel-submit").click();
    await expect(page.getByTestId("chat-title")).toHaveText("Welcome");
    const agentCard = await page
      .getByTestId("welcome-intro-action-create-agent")
      .boundingBox();
    const channelCard = await page
      .getByTestId("welcome-intro-action-create-channel")
      .boundingBox();
    expect(agentCard).not.toBeNull();
    expect(channelCard).not.toBeNull();
    if (agentCard && channelCard) {
      const sameRow = Math.abs(agentCard.y - channelCard.y) < 1;
      expect(sameRow ? agentCard.x : agentCard.y).toBeLessThan(
        sameRow ? channelCard.x : channelCard.y,
      );
    }
    await page.getByTestId("welcome-intro-action-create-agent").click();
    const dialog = page.getByRole("dialog");
    await expect(
      dialog.getByRole("heading", { name: "Create an agent" }),
    ).toBeVisible();
    await waitForAnimations(page);
    await dialog.screenshot({ path: `${SHOTS}/01-welcome-create-choice.png` });

    await page.getByTestId("welcome-create-agent-in-chat").click();
    await expect(dialog).not.toBeVisible();
    await expect(page.getByTestId("message-timeline")).toContainText(
      "Fizz, help me create a new agent.",
    );
  });

  test("welcome manual creation opens the canonical agent form", async ({
    page,
  }) => {
    await installMockBridge(page, {
      activePersonaIds: ["builtin:fizz"],
      managedAgents: [
        {
          pubkey: FIZZ_PUBKEY,
          name: "Fizz",
          personaId: "builtin:fizz",
          status: "running",
          channelNames: ["Welcome"],
        },
      ],
    });
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openCreateChannelDialog(page);
    await page.getByTestId("create-channel-name").fill("Welcome");
    await page.getByTestId("create-channel-private-toggle").click();
    await page.getByTestId("create-channel-submit").click();
    await expect(page.getByTestId("chat-title")).toHaveText("Welcome");
    await page.getByTestId("welcome-intro-action-create-agent").click();
    await page.getByTestId("welcome-create-agent-manually").click();

    await expect(page).toHaveURL(/#\/channels\//);
    await expect(page.getByTestId("chat-title")).toHaveText("Welcome");
    await expect(
      page.getByRole("dialog").getByRole("heading", { name: "Create agent" }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Advanced", exact: true }),
    ).toHaveAttribute("aria-expanded", "false");
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByTestId("chat-title")).toHaveText("Welcome");
  });

  test("channel-targeted creation preserves the agent when attachment fails", async ({
    page,
  }) => {
    await installMockBridge(page, {
      addChannelMembersErrors: ["Relay unavailable.", null],
    });
    await openChannel(page, "random");
    await page.getByTestId("channel-intro-action-create-agent").click();
    await page.getByTestId("add-channel-create-agent").click();

    await expect(page).toHaveURL(/#\/channels\//);
    await expect(page.getByTestId("chat-title")).toHaveText("random");
    await page.locator("#persona-display-name").fill("Scout");
    await page
      .locator("#persona-system-prompt")
      .fill("Research a topic and return a concise brief.");
    await page.getByRole("tab", { name: "Customize for this agent" }).click();
    const provider = page.locator("#persona-llm-provider");
    await provider.press("Enter");
    await page
      .getByRole("menuitemradio", { exact: true, name: "Anthropic" })
      .click();
    await page.locator("#persona-model").click();
    await page
      .getByRole("button", { name: "Custom model...", exact: true })
      .click();
    await page.getByLabel("Custom model ID").fill("claude-opus-4-5");
    await page.getByLabel("Anthropic API Key").fill("sk-test-api-key-for-e2e");
    await expect(page.getByTestId("persona-dialog-submit")).toBeEnabled();
    await page.getByTestId("persona-dialog-submit").click();

    const createdDialog = page.getByRole("dialog");
    await expect(
      createdDialog.getByRole("heading", { name: "Agent created" }),
    ).toBeVisible({ timeout: 10_000 });
    await expect(createdDialog).toContainText(
      "Scout was created, but couldn’t be added to #random.",
    );
    await expect(createdDialog).toContainText("Relay unavailable.");
    await waitForAnimations(page);
    await createdDialog.screenshot({
      path: `${SHOTS}/05-agent-channel-attachment-failed.png`,
    });

    const commandsBeforeRetry = await readCommandLog(page);
    const createCount = commandCount(
      commandsBeforeRetry,
      "create_managed_agent",
    );
    const addCount = commandCount(commandsBeforeRetry, "add_channel_members");

    await createdDialog.getByRole("button", { name: "Try again" }).click();
    await expect(createdDialog).toContainText("Scout is ready and running.");
    await expect(
      createdDialog.getByRole("button", { name: "Try again" }),
    ).toHaveCount(0);

    const commandsAfterRetry = await readCommandLog(page);
    expect(commandCount(commandsAfterRetry, "create_managed_agent")).toEqual(
      createCount,
    );
    expect(commandCount(commandsAfterRetry, "add_channel_members")).toEqual(
      addCount + 1,
    );
    await createdDialog.getByRole("button", { name: "Done" }).click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");
  });

  test("only Fizz is already in the channel", async ({ page }) => {
    await installMockBridge(page, {
      activePersonaIds: ["builtin:fizz"],
      managedAgents: [
        {
          pubkey: FIZZ_PUBKEY,
          name: "Fizz",
          personaId: "builtin:fizz",
          status: "running",
          channelNames: ["random"],
        },
      ],
    });
    const dialog = await openAgentPicker(page);
    await expect(dialog).toContainText(
      "All of your agents are already in this channel.",
    );
    await dialog.screenshot({ path: `${SHOTS}/02-only-fizz-in-channel.png` });
  });

  test("some personal agents are available", async ({ page }) => {
    await installMockBridge(page, {
      activePersonaIds: ["builtin:fizz"],
      personas: [scoutPersona, editorPersona],
      managedAgents: [
        {
          pubkey: FIZZ_PUBKEY,
          name: "Fizz",
          personaId: "builtin:fizz",
          status: "running",
          channelNames: ["random"],
        },
        {
          pubkey: SCOUT_PUBKEY,
          name: "Scout",
          personaId: scoutPersona.id,
          status: "stopped",
        },
        {
          pubkey: EDITOR_PUBKEY,
          name: "Editor",
          personaId: editorPersona.id,
          status: "stopped",
        },
      ],
    });
    const dialog = await openAgentPicker(page);
    await expect(dialog).toContainText("Your agents");
    await expect(dialog).toContainText("Scout");
    await expect(dialog).toContainText("Editor");
    await dialog.screenshot({ path: `${SHOTS}/03-agents-available.png` });
  });

  test("all personal agents are already in the channel", async ({ page }) => {
    await installMockBridge(page, {
      activePersonaIds: ["builtin:fizz"],
      personas: [scoutPersona, editorPersona],
      managedAgents: [
        {
          pubkey: FIZZ_PUBKEY,
          name: "Fizz",
          personaId: "builtin:fizz",
          status: "running",
          channelNames: ["random"],
        },
        {
          pubkey: SCOUT_PUBKEY,
          name: "Scout",
          personaId: scoutPersona.id,
          status: "running",
          channelNames: ["random"],
        },
        {
          pubkey: EDITOR_PUBKEY,
          name: "Editor",
          personaId: editorPersona.id,
          status: "running",
          channelNames: ["random"],
        },
      ],
    });
    const dialog = await openAgentPicker(page);
    await expect(dialog).toContainText(
      "All of your agents are already in this channel.",
    );
    await dialog.screenshot({ path: `${SHOTS}/04-all-agents-in-channel.png` });
  });
});
