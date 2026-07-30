/**
 * Screenshot + behavior spec for the Agent Fleet view (/fleet).
 *
 * Seeds a mixed fleet — a working agent (live turn in #agents), an idle agent
 * with a recent transcript headline, a crashed agent with a recorded error,
 * and a stopped one — plus NIP-AM turn metrics, then captures the board and
 * per-card close-ups. The default mock bridge also contributes `nadia`, the
 * declared-owned relay agent fixture, exercising the cross-machine roster
 * path (owned but not locally managed).
 */

import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/fleet-screenshots";

// #agents mock channel — the working chip resolves this id to its name.
const AGENTS_CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301";

const WORKING_AGENT = {
  pubkey: TEST_IDENTITIES.tyler.pubkey,
  name: "Scout",
  status: "running" as const,
  channelNames: ["agents"],
};

const IDLE_AGENT = {
  pubkey: TEST_IDENTITIES.alice.pubkey,
  name: "Archivist",
  status: "running" as const,
  channelNames: ["agents"],
};

const ERROR_AGENT = {
  pubkey: TEST_IDENTITIES.bob.pubkey,
  name: "Databricks Agent",
  status: "stopped" as const,
  lastError:
    "Agent reported error (code -32002): llm model not found: (goose-databricks-llama-3-3-70b) 404 Not Found: model not found",
  lastErrorCode: -32002,
};

const OFFLINE_AGENT = {
  pubkey: TEST_IDENTITIES.charlie.pubkey,
  name: "Weekend Bot",
  status: "stopped" as const,
};

const MANAGED_AGENTS = [WORKING_AGENT, IDLE_AGENT, ERROR_AGENT, OFFLINE_AGENT];

// A declared-owned relay agent running on another machine: present in the
// relay roster, owned by the mock viewer via its profile's NIP-OA
// owner_pubkey, but NOT locally managed — the cross-machine fleet case.
const MOCK_VIEWER_PUBKEY = "deadbeef".repeat(8);
const REMOTE_AGENT = {
  pubkey: TEST_IDENTITIES.outsider.pubkey,
  name: "Laptop Runner",
};

async function openFleetView(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  // The sidebar entry is behind the "fleet" preview gate, which
  // installMockBridge seeds as enabled by default.
  const fleetButton = page.getByTestId("open-fleet-view");
  await expect(fleetButton).toBeVisible({ timeout: 10_000 });
  await fleetButton.click();
  await expect(page.getByTestId("fleet-view")).toBeVisible({
    timeout: 10_000,
  });
}

async function waitForSeedHooks(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_SEED_ACTIVE_TURNS__ === "function" &&
      typeof window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__ === "function" &&
      typeof window.__BUZZ_E2E_SEED_TURN_METRICS__ === "function",
    null,
    { timeout: 10_000 },
  );
}

async function seedFleetActivity(page: import("@playwright/test").Page) {
  await page.evaluate(
    ({ workingPubkey, idlePubkey, channelId }) => {
      // Live turn: Scout is working in #agents, anchored ~90s ago.
      window.__BUZZ_E2E_SEED_ACTIVE_TURNS__?.({
        agentPubkey: workingPubkey,
        channelId,
        turnId: "turn-fleet-1",
      });

      // Recent transcript for the idle agent → activity headline.
      window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
        agentPubkey: idlePubkey,
        events: [
          {
            seq: 1,
            timestamp: new Date(Date.now() - 5 * 60_000).toISOString(),
            kind: "acp_read",
            agentIndex: 0,
            channelId,
            sessionId: "session-fleet-1",
            // No turnId: a turn-scoped acp frame would register live-turn
            // activity in activeAgentTurnsStore and flip this agent to
            // "working" — this seed is history for the headline only.
            turnId: null,
            payload: {
              jsonrpc: "2.0",
              method: "session/update",
              params: {
                update: {
                  sessionUpdate: "agent_message_chunk",
                  messageId: "msg-fleet-1",
                  content: [
                    {
                      type: "text",
                      text: "Indexed 42 documents and refreshed the search cache.",
                    },
                  ],
                },
              },
            },
          },
        ],
      });

      // NIP-AM turn metrics: per-turn deltas for Scout, a session-cumulative
      // series for Archivist (the aggregator must not double-count it).
      window.__BUZZ_E2E_SEED_TURN_METRICS__?.({
        agentPubkey: workingPubkey,
        payloads: [
          {
            harness: "goose",
            timestamp: new Date(Date.now() - 40 * 60_000).toISOString(),
            sessionId: "sess-a",
            turnSeq: 1,
            turn: { inputTokens: 48_200, outputTokens: 3_400, costUsd: 0.31 },
          },
          {
            harness: "goose",
            timestamp: new Date(Date.now() - 12 * 60_000).toISOString(),
            sessionId: "sess-a",
            turnSeq: 2,
            turn: { inputTokens: 61_800, outputTokens: 5_100, costUsd: 0.42 },
          },
        ],
      });
      window.__BUZZ_E2E_SEED_TURN_METRICS__?.({
        agentPubkey: idlePubkey,
        payloads: [
          {
            harness: "goose",
            timestamp: new Date(Date.now() - 2 * 3_600_000).toISOString(),
            sessionId: "sess-b",
            turnSeq: 1,
            cumulative: { totalTokens: 220_000, costUsd: 0.9 },
          },
          {
            harness: "goose",
            timestamp: new Date(Date.now() - 5 * 60_000).toISOString(),
            sessionId: "sess-b",
            turnSeq: 2,
            cumulative: { totalTokens: 402_000, costUsd: 1.65 },
          },
        ],
      });
    },
    {
      workingPubkey: WORKING_AGENT.pubkey,
      idlePubkey: IDLE_AGENT.pubkey,
      channelId: AGENTS_CHANNEL_ID,
    },
  );
}

test.describe("fleet view screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (err) => {
      console.error(
        "PAGE ERROR:",
        err.message,
        err.stack?.split("\n").slice(0, 5).join("\n"),
      );
    });
  });

  test("fleet board with mixed statuses and usage", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: MANAGED_AGENTS,
      relayAgents: [
        {
          pubkey: REMOTE_AGENT.pubkey,
          name: REMOTE_AGENT.name,
          status: "online",
          channelNames: ["agents"],
        },
      ],
      searchProfiles: [
        {
          pubkey: REMOTE_AGENT.pubkey,
          displayName: REMOTE_AGENT.name,
          ownerPubkey: MOCK_VIEWER_PUBKEY,
          isAgent: true,
        },
      ],
    });
    // Open the view first, then seed — the board updates reactively, and a
    // later goto would reload the page and wipe the seeded module stores.
    await openFleetView(page);
    await waitForSeedHooks(page);
    await seedFleetActivity(page);

    // Summary strip: 1 working, 1 error; idle covers Archivist plus the
    // default owned-relay fixture (nadia).
    const summary = page.getByTestId("fleet-summary");
    await expect(summary).toBeVisible({ timeout: 10_000 });
    await expect(page.getByTestId("fleet-summary-working")).toHaveText(
      /1 working/,
    );
    await expect(page.getByTestId("fleet-summary-error")).toHaveText(
      /1 need attention/,
    );

    // Working card: channel chip with a live elapsed counter.
    const workingCard = page.getByTestId(
      `fleet-agent-card-${WORKING_AGENT.pubkey}`,
    );
    await expect(workingCard).toHaveAttribute("data-status", "working");
    await expect(
      workingCard.getByTestId("fleet-card-working-channel"),
    ).toContainText("#agents");

    // Idle card: headline from the seeded transcript, cumulative usage.
    const idleCard = page.getByTestId(`fleet-agent-card-${IDLE_AGENT.pubkey}`);
    await expect(idleCard.getByTestId("fleet-card-headline")).toContainText(
      "Indexed 42 documents",
    );
    await expect(idleCard).toContainText("402k");
    await expect(idleCard).toContainText("$1.65");

    // Error card: friendly structured copy, not the raw JSON error string.
    const errorCard = page.getByTestId(
      `fleet-agent-card-${ERROR_AGENT.pubkey}`,
    );
    await expect(errorCard).toHaveAttribute("data-status", "error");
    await expect(errorCard.getByTestId("fleet-card-headline")).toContainText(
      "model is not available",
    );

    // Remote owned agent: on the board via profile ownership, labeled as
    // running elsewhere.
    const remoteCard = page.getByTestId(
      `fleet-agent-card-${REMOTE_AGENT.pubkey}`,
    );
    await expect(remoteCard).toContainText("Laptop Runner");
    await expect(remoteCard).toContainText("Runs on another device");

    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/01-fleet-board.png` });
    await workingCard.screenshot({
      path: `${SHOTS}/02-fleet-card-working.png`,
    });
    await errorCard.screenshot({ path: `${SHOTS}/03-fleet-card-error.png` });
  });

  test("clicking a card routes to the agent's session panel", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: MANAGED_AGENTS,
      relayAgents: [
        {
          pubkey: REMOTE_AGENT.pubkey,
          name: REMOTE_AGENT.name,
          status: "online",
          channelNames: ["agents"],
        },
      ],
      searchProfiles: [
        {
          pubkey: REMOTE_AGENT.pubkey,
          displayName: REMOTE_AGENT.name,
          ownerPubkey: MOCK_VIEWER_PUBKEY,
          isAgent: true,
        },
      ],
    });
    await openFleetView(page);
    await waitForSeedHooks(page);
    await seedFleetActivity(page);

    // The working agent card deep-links into its working channel with the
    // existing agent session pane open (useOpenAgentActivity ingress).
    await page.getByTestId(`fleet-agent-card-${WORKING_AGENT.pubkey}`).click();
    await expect(page.getByTestId("agent-session-thread-panel")).toBeVisible({
      timeout: 10_000,
    });
    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/04-fleet-open-session.png` });
  });
});
