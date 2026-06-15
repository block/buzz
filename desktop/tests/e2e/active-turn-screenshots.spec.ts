import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/active-turns";

// Mock agent pubkeys (distinct from the relay agents seeded by default)
const AGENT_PAUL = "aa".repeat(32);
const AGENT_DUNCAN = "bb".repeat(32);
const AGENT_THUFIR = "cc".repeat(32);

// Mock channel IDs from the e2e bridge
const CHANNEL_GENERAL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const CHANNEL_ENGINEERING = "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9";

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

test.describe("active turn indicator screenshots", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("01 — baseline: agents running but idle", async ({ page }) => {
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
          channelNames: ["general", "design"],
        },
        {
          pubkey: AGENT_THUFIR,
          name: "Thufir",
          status: "stopped",
          channelNames: [],
        },
      ],
    });

    await openAgentsView(page);

    const agentsSection = page.getByTestId("unified-agents-groups");
    await expect(agentsSection).toContainText("Paul");
    await expect(agentsSection).toContainText("Duncan");
    await expect(agentsSection).toContainText("Thufir");

    await agentsSection.screenshot({
      path: `${SHOTS}/01-baseline-idle.png`,
    });
  });

  test("02 — single agent working in one channel", async ({ page }) => {
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
          channelNames: ["general", "design"],
        },
        {
          pubkey: AGENT_THUFIR,
          name: "Thufir",
          status: "stopped",
          channelNames: [],
        },
      ],
    });

    await openAgentsView(page);
    await waitForBridge(page);

    // Seed Paul as actively working in #general
    await page.evaluate(
      ({ pubkey, channelId }) => {
        const win = window as Window & {
          __BUZZ_E2E_SEED_ACTIVE_TURNS__?: (input: {
            agentPubkey: string;
            channelId: string;
            turnId: string;
          }) => void;
        };
        win.__BUZZ_E2E_SEED_ACTIVE_TURNS__?.({
          agentPubkey: pubkey,
          channelId,
          turnId: "turn-001",
        });
      },
      { pubkey: AGENT_PAUL, channelId: CHANNEL_GENERAL },
    );

    // Wait for the "Working" badge to appear
    await expect(page.getByTestId(`managed-agent-${AGENT_PAUL}`)).toContainText(
      "Working",
      { timeout: 5_000 },
    );

    const agentsSection = page.getByTestId("unified-agents-groups");
    await agentsSection.screenshot({
      path: `${SHOTS}/02-single-agent-working.png`,
    });
  });

  test("03 — mixed states: one working in 2 channels, one idle, one stopped", async ({
    page,
  }) => {
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
          channelNames: ["general", "design"],
        },
        {
          pubkey: AGENT_THUFIR,
          name: "Thufir",
          status: "stopped",
          channelNames: [],
        },
      ],
    });

    await openAgentsView(page);
    await waitForBridge(page);

    // Seed Paul as working in both #general and #engineering
    await page.evaluate(
      ({ pubkey, channels }) => {
        const win = window as Window & {
          __BUZZ_E2E_SEED_ACTIVE_TURNS__?: (input: {
            agentPubkey: string;
            channelId: string;
            turnId: string;
          }) => void;
        };
        for (const { channelId, turnId } of channels) {
          win.__BUZZ_E2E_SEED_ACTIVE_TURNS__?.({
            agentPubkey: pubkey,
            channelId,
            turnId,
          });
        }
      },
      {
        pubkey: AGENT_PAUL,
        channels: [
          { channelId: CHANNEL_GENERAL, turnId: "turn-002" },
          { channelId: CHANNEL_ENGINEERING, turnId: "turn-003" },
        ],
      },
    );

    // Wait for the "Working" indicators to appear
    await expect(page.getByTestId(`managed-agent-${AGENT_PAUL}`)).toContainText(
      "Working",
      { timeout: 5_000 },
    );

    const agentsSection = page.getByTestId("unified-agents-groups");
    await agentsSection.screenshot({
      path: `${SHOTS}/03-mixed-states.png`,
    });
  });
});
