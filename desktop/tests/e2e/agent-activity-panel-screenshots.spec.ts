import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { FEATURE_OVERRIDES_STORAGE_KEY } from "../helpers/features";

const SHOTS = "test-results/agent-activity-panel";

// A running managed agent whose pubkey maps to the "agents" channel.
// Tyler's pubkey so the mock bridge already knows this identity.
const AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301"; // #agents
const T0 = Date.parse("2025-06-15T12:00:00Z");

const MANAGED_AGENTS = [
  {
    pubkey: AGENT_PUBKEY,
    name: "Observer Agent",
    status: "running" as const,
    channelNames: ["agents"],
  },
];

type SeededObserverEvent = {
  seq: number;
  timestamp: string;
  kind: string;
  agentIndex: number | null;
  channelId: string | null;
  sessionId: string | null;
  turnId: string | null;
  payload: unknown;
};

function frame(
  seq: number,
  offsetMs: number,
  kind: string,
  turnId: string | null,
  payload: unknown,
): SeededObserverEvent {
  return {
    seq,
    timestamp: new Date(T0 + offsetMs).toISOString(),
    kind,
    agentIndex: 0,
    channelId: CHANNEL_ID,
    sessionId: "session-001",
    turnId,
    payload,
  };
}

function sessionUpdate(update: Record<string, unknown>): unknown {
  return {
    jsonrpc: "2.0",
    method: "session/update",
    params: { sessionId: "session-001", update },
  };
}

function toolCall(
  toolCallId: string,
  fields: Record<string, unknown>,
): unknown {
  return sessionUpdate({
    sessionUpdate: "tool_call",
    toolCallId,
    status: "pending",
    ...fields,
  });
}

function narration(text: string): unknown {
  return sessionUpdate({
    sessionUpdate: "agent_message_chunk",
    content: { type: "text", text },
  });
}

/**
 * Two completed turns of realistic file activity in one repo, separated by a
 * two-minute idle gap (exercises gap compression). Turn 1 reads and patches
 * the store, creates a notes file, and runs a shell command; turn 2 touches
 * the README.
 */
function activityFrames(): SeededObserverEvent[] {
  const repo = "/Users/tyler/repo";
  let seq = 0;
  const next = () => {
    seq += 1;
    return seq;
  };

  return [
    frame(next(), 0, "turn_started", "turn-001", {
      source: "channel",
      triggeringEventIds: ["a".repeat(64)],
    }),
    frame(next(), 500, "session_resolved", "turn-001", {
      sessionId: "session-001",
      isNewSession: true,
    }),
    frame(
      next(),
      1_000,
      "acp_read",
      "turn-001",
      narration("Let me look at the store implementation first."),
    ),
    frame(
      next(),
      2_000,
      "acp_read",
      "turn-001",
      toolCall("call-1", {
        title: "read_file",
        kind: "read",
        locations: [{ path: `${repo}/src/store.rs` }],
      }),
    ),
    frame(
      next(),
      5_000,
      "acp_read",
      "turn-001",
      toolCall("call-2", {
        title: "read_file",
        kind: "read",
        locations: [{ path: `${repo}/src/lib.rs` }],
      }),
    ),
    frame(
      next(),
      9_000,
      "acp_read",
      "turn-001",
      narration("Now patching the eviction bug in the store."),
    ),
    frame(
      next(),
      10_000,
      "acp_read",
      "turn-001",
      toolCall("call-3", {
        title: "apply_patch",
        kind: "edit",
        locations: [{ path: `${repo}/src/store.rs` }],
      }),
    ),
    frame(
      next(),
      14_000,
      "acp_read",
      "turn-001",
      toolCall("call-4", {
        title: "write",
        kind: "other",
        rawInput: { path: `${repo}/docs/notes.md`, content: "notes" },
      }),
    ),
    frame(
      next(),
      17_000,
      "acp_read",
      "turn-001",
      toolCall("call-5", {
        title: "developer__shell",
        kind: "execute",
        rawInput: { command: "cargo test" },
      }),
    ),
    frame(next(), 21_000, "turn_completed", "turn-001", {}),

    // Two minutes idle — compressed to a short gap band in display time.
    frame(next(), 141_000, "turn_started", "turn-002", {
      source: "channel",
      triggeringEventIds: ["b".repeat(64)],
    }),
    frame(
      next(),
      142_000,
      "acp_read",
      "turn-002",
      narration("Updating the README with the new behavior."),
    ),
    frame(
      next(),
      143_000,
      "acp_read",
      "turn-002",
      toolCall("call-6", {
        title: "read_file",
        kind: "read",
        locations: [{ path: `${repo}/README.md` }],
      }),
    ),
    frame(
      next(),
      147_000,
      "acp_read",
      "turn-002",
      toolCall("call-7", {
        title: "apply_patch",
        kind: "edit",
        locations: [{ path: `${repo}/README.md` }],
      }),
    ),
    frame(next(), 150_000, "turn_completed", "turn-002", {}),
  ];
}

async function waitForSeedHook(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__ === "function",
    null,
    { timeout: 10_000 },
  );
}

async function openObserverFeedPanel(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForSeedHook(page);

  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");

  const messageRow = page
    .getByTestId("message-row")
    .filter({ has: page.getByText("Observer Agent", { exact: false }) });
  await expect(messageRow.first()).toBeVisible({ timeout: 8_000 });
  await messageRow.first().getByRole("button").first().click();

  const profilePanel = page.getByTestId("user-profile-panel");
  await expect(profilePanel).toBeVisible({ timeout: 10_000 });

  const activityBtn = page.getByTestId(
    `user-profile-view-activity-${AGENT_PUBKEY}`,
  );
  await expect(activityBtn).toBeVisible({ timeout: 5_000 });
  await activityBtn.click();

  const feedPanel = page.getByTestId("agent-session-thread-panel");
  await expect(feedPanel).toBeVisible({ timeout: 10_000 });
  return feedPanel;
}

async function seedObserverEvents(
  page: import("@playwright/test").Page,
  events: SeededObserverEvent[],
) {
  await page.evaluate(
    ({ pubkey, evts }) => {
      window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
        agentPubkey: pubkey,
        events: evts,
      });
    },
    { pubkey: AGENT_PUBKEY, evts: events },
  );
  // Let React re-render and the rAF loop paint a frame.
  await page.waitForTimeout(400);
}

test.describe("agent activity panel screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (err) => {
      console.error(
        "PAGE ERROR:",
        err.message,
        err.stack?.split("\n").slice(0, 5).join("\n"),
      );
    });
    page.on("console", (msg) => {
      if (msg.type() === "error") {
        console.error("CONSOLE ERROR:", msg.text().slice(0, 500));
      }
    });

    // Enable the preview feature before the app boots.
    await page.addInitScript((overridesKey) => {
      window.localStorage.setItem(
        overridesKey,
        JSON.stringify({ agentActivityPanel: true }),
      );
    }, FEATURE_OVERRIDES_STORAGE_KEY);
  });

  test("01 — session activity map with gap compression", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page);

    const panel = feedPanel.locator(
      'section[aria-label="Agent file activity"]',
    );

    // Before any frames: placeholder copy renders instead of the canvas.
    await expect(
      feedPanel.getByText("No file activity in this window yet.", {
        exact: false,
      }),
    ).toBeVisible({ timeout: 5_000 });

    await seedObserverEvents(page, activityFrames());

    await expect(panel).toBeVisible({ timeout: 5_000 });
    // Both turns completed and the agent is idle → parked at the end.
    await expect(panel.getByText("Idle")).toBeVisible({ timeout: 5_000 });

    // Turn picker offers the whole session plus both turns.
    const scopePicker = panel.getByLabel("Activity scope");
    await expect(scopePicker).toBeVisible();
    const optionLabels = await scopePicker.locator("option").allTextContents();
    expect(optionLabels.some((label) => label.includes("Whole session"))).toBe(
      true,
    );
    expect(optionLabels.some((label) => label.includes("Turn 1"))).toBe(true);
    expect(optionLabels.some((label) => label.includes("Turn 2"))).toBe(true);

    await panel.screenshot({ path: `${SHOTS}/01-session-activity-map.png` });
  });

  /**
   * Seed two threads whose root ids match the seeded turns'
   * triggeringEventIds (turn→thread resolution finds real roots with
   * labels), each with an agent reply so the timeline renders a thread
   * summary row — the surface the activity pill mounts on.
   */
  async function seedThreadedChannel(page: import("@playwright/test").Page) {
    await page.evaluate((agentPubkey) => {
      const emit = (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            createdAt?: number;
            id?: string;
            parentEventId?: string;
            pubkey?: string;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) throw new Error("Mock message emitter is unavailable");
      const nowS = Math.floor(Date.now() / 1000);
      emit({
        channelName: "agents",
        content: "Fix the eviction bug in the store",
        createdAt: nowS - 600,
        id: "a".repeat(64),
      });
      emit({
        channelName: "agents",
        content: "On it — patching the store now.",
        createdAt: nowS - 590,
        parentEventId: "a".repeat(64),
        pubkey: agentPubkey,
      });
      emit({
        channelName: "agents",
        content: "Update the README for the new behavior",
        createdAt: nowS - 300,
        id: "b".repeat(64),
      });
      emit({
        channelName: "agents",
        content: "Updating the README.",
        createdAt: nowS - 290,
        parentEventId: "b".repeat(64),
        pubkey: agentPubkey,
      });
    }, AGENT_PUBKEY);
  }

  test("03 — thread activity pill opens the map scoped to its thread", async ({
    page,
  }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await waitForSeedHook(page);

    await page.getByTestId("channel-agents").click();
    await expect(page.getByTestId("chat-title")).toHaveText("agents");

    await seedThreadedChannel(page);

    // No pills until an agent has observer activity resolved to the threads.
    const pills = page.getByTestId("thread-activity-pill");
    await expect(pills).toHaveCount(0);

    await seedObserverEvents(page, activityFrames());

    // One pill per thread with agent file activity, on the summary rows.
    await expect(pills).toHaveCount(2, { timeout: 5_000 });
    await expect(pills.first()).toContainText("Observer Agent");

    const summaryRow = page
      .getByTestId("message-thread-summary")
      .filter({ hasText: "1 reply" })
      .first();
    await expect(summaryRow).toBeVisible();

    await page.screenshot({
      path: `${SHOTS}/03-thread-activity-pills.png`,
      fullPage: false,
    });

    // The pill for the eviction thread (older, first in the timeline) opens
    // the map locked to THAT thread: only turn-001's store/lib/notes rows.
    // The pill IS the thread selection — no thread picker inside. (File
    // rows are canvas-drawn, not DOM text; scope proof is per-pill
    // screenshot diffing in spec 04.)
    await pills.first().click();
    const popover = page.getByTestId("thread-activity-popover");
    await expect(popover).toBeVisible({ timeout: 5_000 });
    const panel = popover.locator('section[aria-label="Agent file activity"]');
    await expect(panel).toBeVisible({ timeout: 5_000 });

    await expect(panel.getByTestId("activity-thread-chips")).toHaveCount(0);

    await popover.screenshot({
      path: `${SHOTS}/03-thread-pill-popover.png`,
    });

    // Agent chips still work inside the popover.
    await expect(
      panel.getByRole("button", { name: "All agents" }),
    ).toBeVisible();
  });

  test("04 — each pill opens the map locked to its own thread", async ({
    page,
  }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await waitForSeedHook(page);

    await page.getByTestId("channel-agents").click();
    await expect(page.getByTestId("chat-title")).toHaveText("agents");

    await seedThreadedChannel(page);
    await seedObserverEvents(page, activityFrames());

    const pills = page.getByTestId("thread-activity-pill");
    await expect(pills).toHaveCount(2, { timeout: 5_000 });
    const popover = page.getByTestId("thread-activity-popover");
    const panel = popover.locator('section[aria-label="Agent file activity"]');

    // Eviction pill (older thread, first in the timeline) → turn-001 scope:
    // three file rows (store.rs, lib.rs, notes.md), so the canvas is taller.
    await pills.first().click();
    await expect(panel).toBeVisible({ timeout: 5_000 });
    const evictionBox = await panel.boundingBox();
    await panel.screenshot({ path: `${SHOTS}/04-scope-eviction-thread.png` });
    await page.keyboard.press("Escape");
    await expect(popover).toHaveCount(0);

    // README pill (newest thread) → turn-002 scope: one file row.
    await pills.nth(1).click();
    await expect(panel).toBeVisible({ timeout: 5_000 });
    const readmeBox = await panel.boundingBox();
    await panel.screenshot({ path: `${SHOTS}/04-scope-readme-thread.png` });

    // Different thread → different scope: the one-file map is strictly
    // shorter than the three-file map (rows drive canvas height).
    if (!evictionBox || !readmeBox) throw new Error("panel box unavailable");
    expect(readmeBox.height).toBeLessThan(evictionBox.height);
  });

  test("02 — turn-scoped replay", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page);
    await seedObserverEvents(page, activityFrames());

    const panel = feedPanel.locator(
      'section[aria-label="Agent file activity"]',
    );
    await expect(panel).toBeVisible({ timeout: 5_000 });

    // Scope to turn 2: only the README events remain.
    const scopePicker = panel.getByLabel("Activity scope");
    await scopePicker.selectOption("turn:turn-002");
    await page.waitForTimeout(400);

    await expect(panel.getByText("Idle")).toBeVisible({ timeout: 5_000 });
    await panel.screenshot({ path: `${SHOTS}/02-turn-scoped-replay.png` });
  });
});
