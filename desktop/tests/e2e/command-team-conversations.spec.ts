import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const OPERATIONS_PERSONA_ID = "builtin:command-operations";
const OPERATIONS_PUBKEY = "f".repeat(64);
const COMMAND_TEAM_PERSONA_IDS = [
  "builtin:command-chief-of-staff",
  OPERATIONS_PERSONA_ID,
  "builtin:command-navigation",
  "builtin:command-daily-routine",
  "builtin:command-reporting",
  "builtin:command-plans",
] as const;

type ManagedAgent = {
  pubkey: string;
  persona_id: string | null;
  status: string;
};

async function invokeMock<T>(
  page: Page,
  command: string,
  payload?: Record<string, unknown>,
): Promise<T> {
  return page.evaluate(
    async ({ command: targetCommand, payload: targetPayload }) => {
      const invoke = (
        window as Window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<unknown>;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      if (!invoke) {
        throw new Error("Mock bridge is unavailable.");
      }
      return (await invoke(targetCommand, targetPayload)) as T;
    },
    { command, payload },
  );
}

async function managedOperationsAgents(page: Page): Promise<ManagedAgent[]> {
  const agents = await invokeMock<ManagedAgent[]>(page, "list_managed_agents");
  return agents.filter((agent) => agent.persona_id === OPERATIONS_PERSONA_ID);
}

async function commandCount(page: Page, command: string): Promise<number> {
  return page.evaluate((target) => {
    const commands =
      (
        window as Window & {
          __BUZZ_E2E_COMMANDS__?: string[];
        }
      ).__BUZZ_E2E_COMMANDS__ ?? [];
    return commands.filter((candidate) => candidate === target).length;
  }, command);
}

test("Command Team conversations create once and reuse the same agent in DMs and channels", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();

  const group = page.getByTestId("command-team-agent-group");
  await expect(group).toBeVisible();
  for (const personaId of COMMAND_TEAM_PERSONA_IDS) {
    await expect(
      group.getByTestId(`persona-agent-row-${personaId}`),
    ).toHaveCount(1);
  }

  const operationsCard = group.getByTestId(
    `persona-agent-row-${OPERATIONS_PERSONA_ID}`,
  );
  await operationsCard.getByRole("button", { name: "Message" }).click();

  await expect(page.getByTestId("chat-title")).toContainText(
    "Operations Adviser",
  );
  const createdDmId = await page
    .locator("[data-active='true'][data-channel-id]")
    .getAttribute("data-channel-id");
  expect(createdDmId).toBeTruthy();
  const afterFirstMessage = await managedOperationsAgents(page);
  expect(afterFirstMessage).toHaveLength(1);
  expect(afterFirstMessage[0]?.status).toBe("running");
  const operationsPubkey = afterFirstMessage[0]?.pubkey;
  expect(operationsPubkey).toBeTruthy();
  expect(await commandCount(page, "create_managed_agent")).toBe(1);

  await page.getByTestId("open-command-console-view").click();
  const consoleCard = page.locator(
    `[data-persona-id="${OPERATIONS_PERSONA_ID}"]`,
  );
  await consoleCard.getByRole("button", { name: "Message" }).click();

  await expect(
    page.locator("[data-active='true'][data-channel-id]"),
  ).toHaveAttribute("data-channel-id", createdDmId ?? "");
  const afterConsoleMessage = await managedOperationsAgents(page);
  expect(afterConsoleMessage).toHaveLength(1);
  expect(afterConsoleMessage[0]?.pubkey).toBe(operationsPubkey);
  expect(await commandCount(page, "create_managed_agent")).toBe(1);

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const input = page.getByTestId("message-input");
  await input.fill("Ask @oper");
  const suggestion = page
    .getByTestId("mention-autocomplete")
    .locator("button", { hasText: "Operations Adviser" });
  await expect(suggestion).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" to review readiness");
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("message-timeline")).toContainText(
    "review readiness",
  );
  const afterMention = await managedOperationsAgents(page);
  expect(afterMention).toHaveLength(1);
  expect(afterMention[0]?.pubkey).toBe(operationsPubkey);
  expect(await commandCount(page, "create_managed_agent")).toBe(1);
});

test("a Command Team start error stays visible and does not navigate", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: OPERATIONS_PUBKEY,
        name: "Operations Adviser",
        personaId: OPERATIONS_PERSONA_ID,
        status: "stopped",
      },
    ],
    startManagedAgentErrors: ["Operations adviser could not start."],
  });
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();

  const agentsUrl = page.url();
  const card = page.getByTestId(`persona-agent-row-${OPERATIONS_PERSONA_ID}`);
  await card.getByRole("button", { name: "Message" }).click();

  await expect(
    page.getByText("Operations adviser could not start."),
  ).toBeVisible();
  await expect(page).toHaveURL(agentsUrl);
  expect(await managedOperationsAgents(page)).toHaveLength(1);
  expect(await commandCount(page, "open_dm")).toBe(0);
});
