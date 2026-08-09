import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

test.use({ viewport: { width: 1510, height: 874 } });

const COMMAND_AGENTS = [
  [
    "1".repeat(64),
    "Chief of Staff",
    "builtin:command-chief-of-staff",
    "running",
  ],
  ["2".repeat(64), "Operations", "builtin:command-operations", "running"],
  ["3".repeat(64), "Maritime N2", "builtin:command-intelligence", "running"],
  ["4".repeat(64), "Logistics", "builtin:command-logistics", "running"],
  ["5".repeat(64), "Navigation", "builtin:command-navigation", "running"],
  ["6".repeat(64), "Daily Routine", "builtin:command-daily-routine", "running"],
  ["7".repeat(64), "Reporting", "builtin:command-reporting", "stopped"],
  ["8".repeat(64), "Plans", "builtin:command-plans", "stopped"],
] as const;

type LivingShipE2eWindow = Window & {
  __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
    command: string,
    payload?: unknown,
  ) => Promise<unknown>;
  __BUZZ_E2E_SEED_ACTIVE_TURNS__?: (input: {
    agentPubkey: string;
    channelId: string;
    turnId: string;
    kind?: string;
  }) => void;
  __BUZZ_E2E_SEED_OBSERVER_EVENTS__?: (input: {
    agentPubkey: string;
    events: Array<Record<string, unknown>>;
  }) => void;
};

test("Living Ship shows working, collaborating, idle and not-aboard advisers", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: COMMAND_AGENTS.map(([pubkey, name, personaId, status]) => ({
      pubkey,
      name,
      personaId,
      status,
      channelNames: ["general"],
    })),
  });
  await page.goto("/");

  const channelId = await page.evaluate(async () => {
    const invoke = (window as LivingShipE2eWindow)
      .__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
    if (!invoke) throw new Error("E2E mock command bridge is unavailable");
    const channels = (await invoke("get_channels")) as Array<{
      id: string;
      name: string;
    }>;
    const general = channels.find((channel) => channel.name === "general");
    if (!general) throw new Error("general channel fixture missing");
    return general.id;
  });

  await page.getByTestId("open-living-ship-view").click();
  await expect(page).toHaveURL(/#\/ship$/);
  const canvas = page.getByTestId("living-ship-canvas");
  await expect(canvas).toBeVisible();

  await page.evaluate(
    ({ agents, channel }) => {
      const target = window as LivingShipE2eWindow;
      const seedTurn = target.__BUZZ_E2E_SEED_ACTIVE_TURNS__;
      const seedEvents = target.__BUZZ_E2E_SEED_OBSERVER_EVENTS__;
      if (!seedTurn || !seedEvents) {
        throw new Error("Living Ship observer seed helpers are unavailable");
      }
      for (const [pubkey] of agents.slice(0, 4)) {
        seedTurn({
          agentPubkey: pubkey,
          channelId: channel,
          turnId: `turn-${pubkey[0]}`,
        });
      }
      const collaborationTimestamp = new Date(Date.now() + 1_000).toISOString();
      for (const pubkey of [agents[0][0], agents[1][0]]) {
        seedEvents({
          agentPubkey: pubkey,
          events: [
            {
              seq: Date.now() + Number(pubkey[0]),
              timestamp: collaborationTimestamp,
              kind: "acp_read",
              agentIndex: 0,
              channelId: channel,
              sessionId: null,
              turnId: `turn-${pubkey[0]}`,
              payload: {
                collaborationId: "daily-brief-1",
                workspace: "meeting-room",
                context: "command",
                participantPubkeys: [agents[0][0], agents[1][0]],
                summary: "Preparing the daily command brief",
              },
            },
          ],
        });
      }
    },
    { agents: COMMAND_AGENTS, channel: channelId },
  );
  await expect(
    canvas.getByRole("button", { name: /Select Operations/ }),
  ).toHaveAttribute("data-state", "collaborating");
  await expect(
    canvas.getByRole("button", { name: /Select Maritime N2/ }),
  ).toHaveAttribute("data-state", "working");
  await expect(
    canvas.getByRole("button", { name: /Select Reporting/ }),
  ).toHaveAttribute("data-state", "offline");

  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/living-ship/01-overview.png",
  });

  await canvas.getByRole("button", { name: /Select Operations/ }).click();
  const details = page.getByTestId("ship-agent-details");
  await expect(details).toContainText("Preparing the daily command brief");
  await expect(details).toContainText("With Chief of Staff");
  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/living-ship/02-collaboration.png",
  });

  const supplyOffice = canvas.locator('[data-room-id="supply-office"]');
  await supplyOffice.focus();
  await supplyOffice.press("Enter");
  await expect(page.getByTestId("ship-room-details")).toContainText(
    "Logistics",
  );
  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/living-ship/03-room.png",
  });

  await canvas.getByRole("button", { name: /Select Operations/ }).click();
  await details.getByRole("button", { name: "Open activity" }).click();
  await expect(page).toHaveURL(/agentSession=/);
});
