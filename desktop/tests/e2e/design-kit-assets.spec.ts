// Design-kit asset generator — NOT part of any test project on purpose.
// Run manually:
//   pnpm build:e2e && pnpm exec playwright test tests/e2e/design-kit-assets.spec.ts --config=playwright.design-kit.config.ts
// Outputs 2x screenshots + a video walkthrough to test-results/design-kit/.
// Untracked scratch tooling for the Agent Activity Map design package.

import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { FEATURE_OVERRIDES_STORAGE_KEY } from "../helpers/features";

const SHOTS = "test-results/design-kit";

const AGENT_A = TEST_IDENTITIES.tyler.pubkey; // "Scout"
const AGENT_B = TEST_IDENTITIES.charlie.pubkey; // "Mason"
const CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301"; // #agents

const THREAD_EVICTION = "a".repeat(64);
const THREAD_README = "b".repeat(64);

const MANAGED_AGENTS = [
  {
    pubkey: AGENT_A,
    name: "Scout",
    status: "running" as const,
    channelNames: ["agents"],
  },
  {
    pubkey: AGENT_B,
    name: "Mason",
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

function frameFactory(sessionId: string, t0: number, seqStart = 0) {
  let seq = seqStart;
  return (
    offsetMs: number,
    kind: string,
    turnId: string | null,
    payload: unknown,
  ): SeededObserverEvent => {
    seq += 1;
    return {
      seq,
      timestamp: new Date(t0 + offsetMs).toISOString(),
      kind,
      agentIndex: 0,
      channelId: CHANNEL_ID,
      sessionId,
      turnId,
      payload,
    };
  };
}

function sessionUpdate(sessionId: string, update: Record<string, unknown>) {
  return {
    jsonrpc: "2.0",
    method: "session/update",
    params: { sessionId, update },
  };
}

function toolCall(
  sessionId: string,
  toolCallId: string,
  fields: Record<string, unknown>,
) {
  return sessionUpdate(sessionId, {
    sessionUpdate: "tool_call",
    toolCallId,
    status: "pending",
    ...fields,
  });
}

function narration(sessionId: string, text: string) {
  return sessionUpdate(sessionId, {
    sessionUpdate: "agent_message_chunk",
    content: { type: "text", text },
  });
}

const REPO = "/Users/demo/checkout";

/** Agent A: eviction-thread turn (3 files) + README-thread turn. */
function scoutFrames(t0: number): SeededObserverEvent[] {
  const f = frameFactory("scout-session", t0);
  const s = "scout-session";
  return [
    f(0, "turn_started", "scout-t1", {
      source: "channel",
      triggeringEventIds: [THREAD_EVICTION],
    }),
    f(800, "acp_read", "scout-t1", narration(s, "Reading the store first.")),
    f(
      1_500,
      "acp_read",
      "scout-t1",
      toolCall(s, "sc-1", {
        title: "read_file",
        kind: "read",
        locations: [{ path: `${REPO}/src/store.rs` }],
      }),
    ),
    f(
      4_000,
      "acp_read",
      "scout-t1",
      toolCall(s, "sc-2", {
        title: "read_file",
        kind: "read",
        locations: [{ path: `${REPO}/src/lib.rs` }],
      }),
    ),
    f(
      7_500,
      "acp_read",
      "scout-t1",
      narration(s, "Patching the eviction bug now."),
    ),
    f(
      8_200,
      "acp_read",
      "scout-t1",
      toolCall(s, "sc-3", {
        title: "apply_patch",
        kind: "edit",
        locations: [{ path: `${REPO}/src/store.rs` }],
      }),
    ),
    f(
      12_000,
      "acp_read",
      "scout-t1",
      toolCall(s, "sc-4", {
        title: "write",
        kind: "other",
        rawInput: { path: `${REPO}/docs/notes.md`, content: "notes" },
      }),
    ),
    f(15_000, "turn_completed", "scout-t1", {}),

    f(120_000, "turn_started", "scout-t2", {
      source: "channel",
      triggeringEventIds: [THREAD_README],
    }),
    f(
      121_000,
      "acp_read",
      "scout-t2",
      narration(s, "Updating the README with the new behavior."),
    ),
    f(
      122_000,
      "acp_read",
      "scout-t2",
      toolCall(s, "sc-5", {
        title: "read_file",
        kind: "read",
        locations: [{ path: `${REPO}/README.md` }],
      }),
    ),
    f(
      126_000,
      "acp_read",
      "scout-t2",
      toolCall(s, "sc-6", {
        title: "apply_patch",
        kind: "edit",
        locations: [{ path: `${REPO}/README.md` }],
      }),
    ),
    f(129_000, "turn_completed", "scout-t2", {}),
  ];
}

/** Agent B: same eviction thread, same repo — near-simultaneous write to
 * store.rs (drives the cross-agent contention flare). */
function masonFrames(t0: number): SeededObserverEvent[] {
  const f = frameFactory("mason-session", t0);
  const s = "mason-session";
  return [
    f(2_000, "turn_started", "mason-t1", {
      source: "channel",
      triggeringEventIds: [THREAD_EVICTION],
    }),
    f(
      2_800,
      "acp_read",
      "mason-t1",
      narration(s, "Adding a regression test for the eviction path."),
    ),
    f(
      3_500,
      "acp_read",
      "mason-t1",
      toolCall(s, "ms-1", {
        title: "read_file",
        kind: "read",
        locations: [{ path: `${REPO}/tests/eviction.rs` }],
      }),
    ),
    f(
      6_500,
      "acp_read",
      "mason-t1",
      toolCall(s, "ms-2", {
        title: "apply_patch",
        kind: "edit",
        locations: [{ path: `${REPO}/tests/eviction.rs` }],
      }),
    ),
    f(
      9_000,
      "acp_read",
      "mason-t1",
      narration(s, "Store API changed — fixing the callsite too."),
    ),
    f(
      9_600,
      "acp_read",
      "mason-t1",
      toolCall(s, "ms-3", {
        title: "apply_patch",
        kind: "edit",
        locations: [{ path: `${REPO}/src/store.rs` }],
      }),
    ),
    f(
      13_000,
      "acp_read",
      "mason-t1",
      toolCall(s, "ms-4", {
        title: "developer__shell",
        kind: "execute",
        rawInput: { command: "cargo test eviction" },
      }),
    ),
    f(16_000, "turn_completed", "mason-t1", {}),
  ];
}

/** Live tail for Scout: an OPEN turn (no turn_completed) in the README
 * thread with wall-clock-recent timestamps, so the pill dot + LIVE badge
 * light up when paired with a seeded active turn. */
function scoutLiveFrames(nowMs: number): SeededObserverEvent[] {
  // seqStart continues after scoutFrames' 13 frames — the store orders by
  // (timestamp, seq) and the live turn must sort after the historical ones.
  const f = frameFactory("scout-session", nowMs - 20_000, 100);
  const s = "scout-session";
  return [
    f(0, "turn_started", "scout-t3", {
      source: "channel",
      triggeringEventIds: [THREAD_README],
    }),
    f(
      1_000,
      "acp_read",
      "scout-t3",
      narration(s, "Follow-up: expanding the usage section."),
    ),
    f(
      2_500,
      "acp_read",
      "scout-t3",
      toolCall(s, "sc-7", {
        title: "read_file",
        kind: "read",
        locations: [{ path: `${REPO}/README.md` }],
      }),
    ),
    f(
      9_000,
      "acp_read",
      "scout-t3",
      toolCall(s, "sc-8", {
        title: "apply_patch",
        kind: "edit",
        locations: [{ path: `${REPO}/README.md` }],
      }),
    ),
    f(
      16_000,
      "acp_read",
      "scout-t3",
      toolCall(s, "sc-9", {
        title: "apply_patch",
        kind: "edit",
        locations: [{ path: `${REPO}/docs/usage.md` }],
      }),
    ),
  ];
}

async function waitForSeedHook(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__ === "function",
    null,
    { timeout: 10_000 },
  );
}

async function seedObserver(
  page: import("@playwright/test").Page,
  agentPubkey: string,
  events: SeededObserverEvent[],
) {
  await page.evaluate(
    ({ pubkey, evts }) => {
      window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
        agentPubkey: pubkey,
        events: evts,
      });
    },
    { pubkey: agentPubkey, evts: events },
  );
  await page.waitForTimeout(400);
}

async function seedThreads(page: import("@playwright/test").Page) {
  await page.evaluate(
    ({ agentA, agentB, evictionId, readmeId }) => {
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
      const nowS = Math.floor(Date.now() / 1_000);
      emit({
        channelName: "agents",
        content:
          "Fix the eviction bug in the cache store — repro in #4821, evictions fire twice under load",
        createdAt: nowS - 900,
        id: evictionId,
      });
      emit({
        channelName: "agents",
        content: "On it — patching the store and adding a regression test.",
        createdAt: nowS - 880,
        parentEventId: evictionId,
        pubkey: agentA,
      });
      emit({
        channelName: "agents",
        content: "Taking the test half.",
        createdAt: nowS - 870,
        parentEventId: evictionId,
        pubkey: agentB,
      });
      emit({
        channelName: "agents",
        content: "Update the README for the new eviction behavior",
        createdAt: nowS - 400,
        id: readmeId,
      });
      emit({
        channelName: "agents",
        content: "Updating the docs now.",
        createdAt: nowS - 390,
        parentEventId: readmeId,
        pubkey: agentA,
      });
    },
    {
      agentA: AGENT_A,
      agentB: AGENT_B,
      evictionId: THREAD_EVICTION,
      readmeId: THREAD_README,
    },
  );
}

async function seedActiveTurn(
  page: import("@playwright/test").Page,
  agentPubkey: string,
  turnId: string,
) {
  await page.evaluate(
    ({ pubkey, channelId, turn }) => {
      (
        window as Window & {
          __BUZZ_E2E_SEED_ACTIVE_TURNS__?: (input: {
            agentPubkey: string;
            channelId: string;
            turnId: string;
          }) => void;
        }
      ).__BUZZ_E2E_SEED_ACTIVE_TURNS__?.({
        agentPubkey: pubkey,
        channelId,
        turnId: turn,
      });
    },
    { pubkey: agentPubkey, channelId: CHANNEL_ID, turn: turnId },
  );
}

async function openChannel(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForSeedHook(page);
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");
}

test.describe("design kit assets", () => {
  test.use({
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 2,
  });

  test.beforeEach(async ({ page }) => {
    await page.addInitScript((overridesKey) => {
      window.localStorage.setItem(
        overridesKey,
        JSON.stringify({ agentActivityPanel: true }),
      );
    }, FEATURE_OVERRIDES_STORAGE_KEY);
  });

  test("A — pills in timeline (idle)", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openChannel(page);
    await seedThreads(page);
    const t0 = Date.now() - 15 * 60_000;
    await seedObserver(page, AGENT_A, scoutFrames(t0));
    await seedObserver(page, AGENT_B, masonFrames(t0));

    const pills = page.getByTestId("thread-activity-pill");
    await expect(pills).toHaveCount(2, { timeout: 8_000 });
    await expect(pills.first()).toContainText("2 agents");
    await expect(pills.nth(1)).toContainText("Scout");

    await page.screenshot({ path: `${SHOTS}/A-pills-in-timeline.png` });
  });

  test("B — popover idle, multi-agent with contention", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openChannel(page);
    await seedThreads(page);
    const t0 = Date.now() - 15 * 60_000;
    await seedObserver(page, AGENT_A, scoutFrames(t0));
    await seedObserver(page, AGENT_B, masonFrames(t0));

    const pills = page.getByTestId("thread-activity-pill");
    await expect(pills).toHaveCount(2, { timeout: 8_000 });
    await pills.first().click();

    const popover = page.getByTestId("thread-activity-popover");
    const panel = popover.locator('section[aria-label="Agent file activity"]');
    await expect(panel).toBeVisible({ timeout: 5_000 });
    await expect(panel.getByText("Idle")).toBeVisible({ timeout: 5_000 });
    await page.waitForTimeout(600);

    await popover.screenshot({
      path: `${SHOTS}/B-popover-idle-multiagent.png`,
    });
    await page.screenshot({ path: `${SHOTS}/B-popover-in-context.png` });
  });

  test("C — agent filter chip active", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openChannel(page);
    await seedThreads(page);
    const t0 = Date.now() - 15 * 60_000;
    await seedObserver(page, AGENT_A, scoutFrames(t0));
    await seedObserver(page, AGENT_B, masonFrames(t0));

    const pills = page.getByTestId("thread-activity-pill");
    await expect(pills).toHaveCount(2, { timeout: 8_000 });
    await pills.first().click();

    const popover = page.getByTestId("thread-activity-popover");
    const panel = popover.locator('section[aria-label="Agent file activity"]');
    await expect(panel).toBeVisible({ timeout: 5_000 });

    await panel.getByRole("button", { name: /Mason/ }).click();
    await page.waitForTimeout(600);
    await popover.screenshot({ path: `${SHOTS}/C-agent-filter-mason.png` });
  });

  test("D — replay scrubbed mid-turn", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openChannel(page);
    await seedThreads(page);
    const t0 = Date.now() - 15 * 60_000;
    await seedObserver(page, AGENT_A, scoutFrames(t0));
    await seedObserver(page, AGENT_B, masonFrames(t0));

    const pills = page.getByTestId("thread-activity-pill");
    await expect(pills).toHaveCount(2, { timeout: 8_000 });
    await pills.first().click();

    const popover = page.getByTestId("thread-activity-popover");
    const panel = popover.locator('section[aria-label="Agent file activity"]');
    await expect(panel).toBeVisible({ timeout: 5_000 });

    // Restart playback from the beginning, let the playhead run into the
    // turn, then pause mid-flight for the shot.
    await panel.getByRole("button", { name: "Restart" }).click();
    await page.waitForTimeout(1_800);
    await panel.getByRole("button", { name: "Play" }).click();
    await page.waitForTimeout(300);
    await expect(panel.getByText("Replay")).toBeVisible();
    await popover.screenshot({ path: `${SHOTS}/D-replay-mid-scrub.png` });
  });

  test("E — live: working dot + LIVE badge", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openChannel(page);
    await seedThreads(page);
    const t0 = Date.now() - 15 * 60_000;
    await seedObserver(page, AGENT_A, scoutFrames(t0));
    await seedObserver(page, AGENT_B, masonFrames(t0));
    await seedObserver(page, AGENT_A, scoutLiveFrames(Date.now()));
    await seedActiveTurn(page, AGENT_A, "scout-t3");

    const pills = page.getByTestId("thread-activity-pill");
    await expect(pills).toHaveCount(2, { timeout: 8_000 });
    // README thread pill (second) should now show the working dot.
    await page.waitForTimeout(800);
    await page.screenshot({ path: `${SHOTS}/E-pill-working-dot.png` });

    await pills.nth(1).click();
    const popover = page.getByTestId("thread-activity-popover");
    const panel = popover.locator('section[aria-label="Agent file activity"]');
    await expect(panel).toBeVisible({ timeout: 5_000 });
    await expect(panel.getByText("Live", { exact: true })).toBeVisible({
      timeout: 5_000,
    });
    await page.waitForTimeout(800);
    await popover.screenshot({ path: `${SHOTS}/E-popover-live.png` });
  });

  test("F — video walkthrough", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openChannel(page);
    await seedThreads(page);
    const t0 = Date.now() - 15 * 60_000;
    await seedObserver(page, AGENT_A, scoutFrames(t0));
    await seedObserver(page, AGENT_B, masonFrames(t0));

    const pills = page.getByTestId("thread-activity-pill");
    await expect(pills).toHaveCount(2, { timeout: 8_000 });
    await page.waitForTimeout(1_500);

    // Open the multi-agent thread, let the DVR replay run.
    await pills.first().click();
    const popover = page.getByTestId("thread-activity-popover");
    const panel = popover.locator('section[aria-label="Agent file activity"]');
    await expect(panel).toBeVisible({ timeout: 5_000 });
    await page.waitForTimeout(1_000);
    await panel.getByRole("button", { name: "Restart" }).click();
    await page.waitForTimeout(4_000);

    // Filter to one agent, then back to all.
    await panel.getByRole("button", { name: /Mason/ }).click();
    await page.waitForTimeout(1_500);
    await panel.getByRole("button", { name: "All agents" }).click();
    await page.waitForTimeout(1_500);

    // Close, open the other thread's pill.
    await page.keyboard.press("Escape");
    await page.waitForTimeout(600);
    await pills.nth(1).click();
    await expect(panel).toBeVisible({ timeout: 5_000 });
    await page.waitForTimeout(1_000);
    await panel.getByRole("button", { name: "Restart" }).click();
    await page.waitForTimeout(3_500);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(800);
  });
});
