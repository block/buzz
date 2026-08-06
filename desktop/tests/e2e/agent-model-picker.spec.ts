import { expect, test, type Page } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

// A standalone agent: no persona, not running. That combination forces the
// ModelPicker down its non-live branch, where a pick persists the default via
// `update_managed_agent` instead of publishing a kind-24200 `switch_model`
// control frame (which the browser harness has no relay to carry).
const AGENT = TEST_IDENTITIES.tyler;
const AGENT_NAME = "Standalone Helper";

const CATALOG = {
  models: [
    { id: "openrouter/auto", name: "Auto (OpenRouter)" },
    { id: "anthropic/claude-sonnet-4.5", name: "Claude Sonnet 4.5" },
    // A nameless entry proves the item falls back to the raw model id.
    { id: "openai/gpt-5", name: null },
  ],
  supportsSwitching: true,
  agentDefaultModel: "openrouter/auto",
};

async function openAgentsView(page: Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-library-personas")).toBeVisible({
    timeout: 10_000,
  });
}

function commandCount(page: Page, command: string) {
  return page.evaluate(
    (name) =>
      (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
        (entry) => entry.command === name,
      ).length,
    command,
  );
}

test("the picker loads its catalog on first open and persists the pick", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: AGENT.pubkey,
        name: AGENT_NAME,
        personaId: null,
        status: "stopped",
      },
    ],
    agentModels: CATALOG,
  });

  await openAgentsView(page);

  const card = page.getByTestId(`managed-agent-${AGENT.pubkey}`);
  await expect(card).toBeVisible();

  // With no persisted model and no catalog yet, the trigger reads "Auto" and
  // nothing has been fetched — the request is deferred to the first open.
  const trigger = card.getByRole("button", { name: "Auto", exact: true });
  await expect(trigger).toBeVisible();
  expect(await commandCount(page, "get_agent_models")).toBe(0);

  await trigger.click();

  await expect
    .poll(() => commandCount(page, "get_agent_models"))
    .toBeGreaterThan(0);

  // The seeded catalog renders, including the id fallback for a nameless model.
  await expect(
    page.getByRole("menuitemradio", { name: "Auto (OpenRouter)" }),
  ).toBeVisible();
  await expect(
    page.getByRole("menuitemradio", { name: "openai/gpt-5" }),
  ).toBeVisible();
  const sonnet = page.getByRole("menuitemradio", { name: "Claude Sonnet 4.5" });
  await expect(sonnet).toBeVisible();

  const commandsBeforePick = await page.evaluate(
    () => window.__BUZZ_E2E_COMMAND_LOG__?.length ?? 0,
  );
  await sonnet.click();

  // The non-live path persists the chosen model as the agent's default.
  await expect
    .poll(async () =>
      page.evaluate((start) => {
        const commands = window.__BUZZ_E2E_COMMAND_LOG__ ?? [];
        return commands
          .slice(start)
          .some(
            (entry) =>
              entry.command === "update_managed_agent" &&
              (entry.payload as { input?: { model?: string | null } })?.input
                ?.model === "anthropic/claude-sonnet-4.5",
          );
      }, commandsBeforePick),
    )
    .toBe(true);

  // ...and the refetched agent drives the trigger label.
  await expect(
    card.getByRole("button", {
      name: "anthropic/claude-sonnet-4.5",
      exact: true,
    }),
  ).toBeVisible();
});

test("a runtime that cannot switch models explains itself instead of listing", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: AGENT.pubkey,
        name: AGENT_NAME,
        personaId: null,
        status: "stopped",
      },
    ],
    agentModels: { ...CATALOG, supportsSwitching: false },
  });

  await openAgentsView(page);

  const card = page.getByTestId(`managed-agent-${AGENT.pubkey}`);
  await card.getByRole("button", { name: "Auto", exact: true }).click();

  await expect(
    page.getByText("This agent uses the runtime's default model."),
  ).toBeVisible();
  await expect(page.getByRole("menuitemradio")).toHaveCount(0);
});
