import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/active-turn-resilience";

// Mock agent pubkeys (distinct from the relay agents seeded by default).
const AGENT_PAUL = "aa".repeat(32);
const AGENT_DUNCAN = "bb".repeat(32);

// Mock channel IDs from the e2e bridge.
const CHANNEL_GENERAL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const CHANNEL_ENGINEERING = "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9";

// A fixed epoch so the mocked clock is deterministic across runs.
const T0 = new Date("2026-06-18T12:00:00.000Z");

// Past both thresholds: FRAME_GAP_PAUSE_MS (20s) and REMOVE_AFTER_MS (25s).
// Several 5s prune ticks fire across this span, so shouldPausePrune is what
// keeps the badges alive — not the absence of a prune tick.
const FRAME_GAP_MS = 30_000;

type SeedInput = {
  agentPubkey: string;
  channelId: string;
  turnId: string;
};

async function waitForBridge(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () =>
      typeof (window as Window & { __BUZZ_E2E_SEED_ACTIVE_TURNS__?: unknown })
        .__BUZZ_E2E_SEED_ACTIVE_TURNS__ === "function",
    null,
    { timeout: 10_000 },
  );
}

async function openAgentsView(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForBridge(page);
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("unified-agents-groups")).toBeVisible({
    timeout: 10_000,
  });
}

async function seedTurns(
  page: import("@playwright/test").Page,
  turns: SeedInput[],
) {
  await page.evaluate((seeds) => {
    const win = window as Window & {
      __BUZZ_E2E_SEED_ACTIVE_TURNS__?: (input: {
        agentPubkey: string;
        channelId: string;
        turnId: string;
      }) => void;
    };
    for (const seed of seeds) win.__BUZZ_E2E_SEED_ACTIVE_TURNS__?.(seed);
  }, turns);
}

test.describe("active turn badge resilience screenshots", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("badges persist through an all-at-once liveness gap", async ({
    page,
  }) => {
    // Install the mocked clock BEFORE navigation so the store's Date.now() /
    // setInterval and the badge's useNow(1000) all run on the mocked clock from
    // module init. Seeded turns then stamp lastActivityAt at T0.
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PAUL,
          name: "Paul",
          status: "running",
          channelNames: ["general", "engineering"],
        },
        {
          pubkey: AGENT_DUNCAN,
          name: "Duncan",
          status: "running",
          channelNames: ["general"],
        },
      ],
    });
    await page.clock.install({ time: T0 });

    await openAgentsView(page);

    // Both agents working across channels — the healthy multi-agent state.
    await seedTurns(page, [
      {
        agentPubkey: AGENT_PAUL,
        channelId: CHANNEL_GENERAL,
        turnId: "t-paul-g",
      },
      {
        agentPubkey: AGENT_PAUL,
        channelId: CHANNEL_ENGINEERING,
        turnId: "t-paul-e",
      },
      {
        agentPubkey: AGENT_DUNCAN,
        channelId: CHANNEL_GENERAL,
        turnId: "t-duncan-g",
      },
    ]);

    const paulRow = page.getByTestId(`managed-agent-${AGENT_PAUL}`);
    const duncanRow = page.getByTestId(`managed-agent-${AGENT_DUNCAN}`);
    await expect(paulRow).toContainText("Working", { timeout: 5_000 });
    await expect(duncanRow).toContainText("Working", { timeout: 5_000 });

    const agentsSection = page.getByTestId("unified-agents-groups");
    await agentsSection.screenshot({
      path: `${SHOTS}/01-badges-before-gap.png`,
    });

    // Simulate the all-at-once relay drop: no further frames, advance the clock
    // past both thresholds. This fires several real prune ticks; shouldPausePrune
    // sees every turn's lastActivityAt stuck at T0 (gap > 20s) and pauses the
    // prune, so the badges survive. Under the pre-fix code every badge would be
    // gone after the first tick past 25s.
    await page.clock.fastForward(FRAME_GAP_MS);

    await expect(paulRow).toContainText("Working");
    await expect(duncanRow).toContainText("Working");

    await agentsSection.screenshot({
      path: `${SHOTS}/02-badges-survive-gap.png`,
    });
  });
});
