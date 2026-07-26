import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/richer-agent-cards";
const WORKING_AGENT = "aa".repeat(32);
const COMPLETED_AGENT = "bb".repeat(32);
const CHANNEL_GENERAL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const CHANNEL_ENGINEERING = "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9";

async function waitForTurnBridge(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () =>
      typeof (window as Window & { __BUZZ_E2E_SEED_ACTIVE_TURNS__?: unknown })
        .__BUZZ_E2E_SEED_ACTIVE_TURNS__ === "function",
    null,
    { timeout: 10_000 },
  );
}

async function seedTurn(
  page: import("@playwright/test").Page,
  input: {
    agentPubkey: string;
    channelId: string;
    turnId: string;
    kind?: "turn_started" | "turn_completed";
  },
) {
  await page.evaluate((turn) => {
    (
      window as Window & {
        __BUZZ_E2E_SEED_ACTIVE_TURNS__?: (seed: typeof turn) => void;
      }
    ).__BUZZ_E2E_SEED_ACTIVE_TURNS__?.(turn);
  }, input);
}

test("agent cards show current and last completed activity", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: WORKING_AGENT,
        name: "Onderzoeker",
        status: "running",
        channelNames: ["engineering"],
      },
      {
        pubkey: COMPLETED_AGENT,
        name: "Redactie",
        status: "running",
        channelNames: ["general"],
      },
    ],
  });

  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForTurnBridge(page);
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("unified-agents-groups")).toBeVisible({
    timeout: 10_000,
  });

  await seedTurn(page, {
    agentPubkey: WORKING_AGENT,
    channelId: CHANNEL_ENGINEERING,
    turnId: "working-turn",
  });
  await seedTurn(page, {
    agentPubkey: COMPLETED_AGENT,
    channelId: CHANNEL_GENERAL,
    turnId: "completed-turn",
  });
  await seedTurn(page, {
    agentPubkey: COMPLETED_AGENT,
    channelId: CHANNEL_GENERAL,
    turnId: "completed-turn",
    kind: "turn_completed",
  });

  const workingCard = page.getByTestId(`managed-agent-${WORKING_AGENT}`);
  const completedCard = page.getByTestId(`managed-agent-${COMPLETED_AGENT}`);

  await expect(workingCard.getByText("Bezig", { exact: true })).toBeVisible();
  await expect(
    workingCard.getByTestId("agent-card-current-activity"),
  ).toContainText("Nu: #engineering");
  await expect(
    completedCard.getByText("Beschikbaar", { exact: true }),
  ).toBeVisible();
  await expect(
    completedCard.getByTestId("agent-card-last-activity"),
  ).toContainText("Laatst: #general");

  await waitForAnimations(page);
  await workingCard.locator("..").screenshot({
    path: `${SHOTS}/agent-cards-working-last.png`,
  });

  await completedCard.getByTestId("agent-card-last-activity").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});
