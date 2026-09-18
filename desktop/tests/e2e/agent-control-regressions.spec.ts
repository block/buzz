import { expect, test, type Page } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const AGENT_PUBKEY = TEST_IDENTITIES.charlie.pubkey;
const RESTORED_UNSCOPED_AGENT_PUBKEY = TEST_IDENTITIES.outsider.pubkey;
const CHANNEL_AGENTS = "94a444a4-c0a3-5966-ab05-530c6ddc2301";
const CHANNEL_GENERAL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const CHANNEL_FOREIGN = "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9";

type ControlRequest = {
  agentPubkey: string;
  payload: {
    type: "cancel_turn" | "switch_model";
    channelId?: string;
    requestId?: string;
    modelId?: string;
  };
};

type E2eControlWindow = Window & {
  __BUZZ_E2E_RUN_MODEL_SWITCH__?: (input: {
    agentPubkey: string;
    channelIds: string[];
    modelId: string;
    requestId: string;
    timeoutMs?: number;
  }) => Promise<string>;
  __BUZZ_E2E_MOUNT_AGENT_SESSION_PANEL__?: (input: {
    agentPubkey: string;
    channelId: string;
    canInterruptTurn?: boolean;
  }) => Promise<void>;
};

async function waitForActiveTurnSeed(page: Page) {
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_ACTIVE_TURNS__ === "function",
    null,
    { timeout: 10_000 },
  );
}

async function seedActiveTurn(page: Page, channelId: string) {
  await page.evaluate(
    ({ agentPubkey, channelId }) => {
      window.__BUZZ_E2E_SEED_ACTIVE_TURNS__?.({
        agentPubkey,
        channelId,
        turnId: `e2e-stop-${channelId}`,
      });
    },
    { agentPubkey: AGENT_PUBKEY, channelId },
  );
}

async function openAgentActivity(
  page: Page,
  activityChannelId: string,
  seedChannels: string[] = [activityChannelId],
): Promise<ReturnType<Page["getByTestId"]>> {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");
  await waitForActiveTurnSeed(page);
  for (const channelId of seedChannels) {
    await seedActiveTurn(page, channelId);
  }

  const messageRow = page
    .getByTestId("message-row")
    .filter({ hasText: "Indexing the channel catalog now." });
  await expect(messageRow.first()).toBeVisible({ timeout: 8_000 });
  await messageRow.first().getByRole("button").first().click();

  const profile = page.getByTestId("user-profile-panel");
  await expect(profile).toBeVisible({ timeout: 10_000 });
  if (activityChannelId !== CHANNEL_AGENTS) {
    const dot = page.getByRole("tab", {
      name: "Show #general activity",
    });
    await expect(dot).toBeVisible({ timeout: 5_000 });
    await dot.click();
  }
  const activity = page.getByRole("button", {
    name: /Open full activity\./,
  });
  await expect(activity).toBeVisible({ timeout: 5_000 });
  await activity.click();

  const panel = page.getByTestId("agent-session-thread-panel");
  await expect(panel).toBeVisible({ timeout: 10_000 });
  await expect(
    page.getByTestId("agent-session-settings-menu-trigger"),
  ).toBeVisible();
  return panel;
}

async function readControlRequests(page: Page): Promise<ControlRequest[]> {
  return page.evaluate(
    () => (window.__BUZZ_E2E_OBSERVER_CONTROLS__ ?? []) as ControlRequest[],
  );
}

async function clickStop(page: Page) {
  await page.getByTestId("agent-session-settings-menu-trigger").click();
  const stop = page.getByTestId("agent-session-stop-turn");
  await expect(stop).toBeVisible();
  await expect(stop).toBeEnabled();
  await stop.click();
}

test.describe("agent control browser regressions", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("Stop uses the channelId-only activity scope and carries a requestId", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          name: "Charlie",
          personaId: "control-persona",
          pubkey: AGENT_PUBKEY,
          status: "running",
          channelNames: ["agents"],
        },
      ],
      observerControlResults: [{ type: "cancel_turn", status: "sent" }],
    });

    // The visible route normalizes a requested #general activity scope back
    // to the active #agents channel. This covers route normalization and
    // effective active-channel targeting, not the channelId-only component
    // props contract (covered by the direct harness regression below).
    const panel = await openAgentActivity(page, CHANNEL_GENERAL, [
      CHANNEL_AGENTS,
      CHANNEL_GENERAL,
    ]);
    // ChannelPane intentionally resolves a requested scope that differs from
    // the visible route back to the active channel. Stop must target that
    // effective channel scope, never the stale profile selection.
    await expect(page.getByTestId("agent-session-scope-label")).toHaveText(
      "Activity · #agents",
    );

    await clickStop(page);
    await expect
      .poll(() => readControlRequests(page))
      .toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            agentPubkey: AGENT_PUBKEY,
            payload: expect.objectContaining({
              type: "cancel_turn",
              channelId: CHANNEL_AGENTS,
              requestId: expect.any(String),
            }),
          }),
        ]),
      );
    const request = (await readControlRequests(page)).find(
      (entry) => entry.payload.type === "cancel_turn",
    );
    expect(request?.payload.requestId).toBeTruthy();
    await expect(page.getByText(/Stop signal sent to Charlie/)).toBeVisible();
    await expect(panel).toBeVisible();
  });

  test("Stop publishes from a channelId-only panel with no Channel object", async ({
    page,
  }) => {
    await installMockBridge(page, {
      observerControlResults: [{ type: "cancel_turn", status: "sent" }],
    });
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await page.waitForFunction(
      () =>
        typeof (window as E2eControlWindow)
          .__BUZZ_E2E_MOUNT_AGENT_SESSION_PANEL__ === "function",
    );
    await waitForActiveTurnSeed(page);
    await seedActiveTurn(page, CHANNEL_AGENTS);
    await page.evaluate(
      ({ agentPubkey, channelId }) =>
        (window as E2eControlWindow).__BUZZ_E2E_MOUNT_AGENT_SESSION_PANEL__?.({
          agentPubkey,
          channelId,
        }),
      { agentPubkey: AGENT_PUBKEY, channelId: CHANNEL_AGENTS },
    );

    const panel = page.getByTestId("agent-session-thread-panel");
    await expect(panel).toBeVisible({ timeout: 10_000 });
    await page.getByTestId("agent-session-settings-menu-trigger").click();
    const stop = page.getByTestId("agent-session-stop-turn");
    await expect(stop).toBeEnabled();
    await stop.click();
    await expect
      .poll(() => readControlRequests(page))
      .toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            agentPubkey: AGENT_PUBKEY,
            payload: expect.objectContaining({
              type: "cancel_turn",
              channelId: CHANNEL_AGENTS,
              requestId: expect.any(String),
            }),
          }),
        ]),
      );
    await expect(page.getByText(/Stop signal sent to charlie/)).toBeVisible();
  });

  test("Stop reports ambiguous_target without claiming success", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          name: "Charlie",
          personaId: "control-persona",
          pubkey: AGENT_PUBKEY,
          status: "running",
          channelNames: ["agents"],
        },
      ],
      observerControlResults: [
        { type: "cancel_turn", status: "ambiguous_target" },
      ],
    });
    await openAgentActivity(page, CHANNEL_AGENTS);

    await clickStop(page);
    await expect(page.getByText(/multiple agent sessions/)).toBeVisible();
    await expect(page.getByText(/Stop signal sent to Charlie/)).toHaveCount(0);
  });

  test("Stop does not accept an unconfirmed or foreign-channel result", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          name: "Charlie",
          personaId: "control-persona",
          pubkey: AGENT_PUBKEY,
          status: "running",
          channelNames: ["agents"],
        },
      ],
      observerControlResults: [
        {
          type: "cancel_turn",
          status: "sent",
          channelId: CHANNEL_FOREIGN,
        },
      ],
    });
    await openAgentActivity(page, CHANNEL_AGENTS);

    // Open the settings menu on real time so the DropdownMenuContent's 150ms
    // CSS enter-animation (zoom-in-95) can complete before the fake clock is
    // installed. Once the clock is active, every setTimeout — including those
    // used by the correlation timeout the test exercises — is fake-controlled.
    await page.getByTestId("agent-session-settings-menu-trigger").click();
    const stop = page.getByTestId("agent-session-stop-turn");
    await expect(stop).toBeVisible();
    await expect(stop).toBeEnabled();
    // Settle the enter-animation before installing the fake clock. Real
    // setTimeout here; waitForAnimations works normally with no fake clock.
    await waitForAnimations(page);

    // Install the fake clock NOW — after the menu is open and stable. Any
    // setTimeout scheduled from this point forward (e.g. the 8-second
    // correlation timeout) will be fake-clock-controlled.
    await page.clock.install({ time: new Date("2026-08-30T17:00:00.000Z") });

    await stop.click();
    await expect
      .poll(() => readControlRequests(page))
      .toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            payload: expect.objectContaining({
              channelId: CHANNEL_AGENTS,
              requestId: expect.any(String),
            }),
          }),
        ]),
      );

    // The configured result is emitted with CHANNEL_FOREIGN. The correlator
    // must ignore it, then report the bounded timeout honestly.
    await page.clock.fastForward(8_001);
    await expect(page.getByText(/hasn't confirmed it/)).toBeVisible();
    await expect(page.getByText(/Stop signal sent to Charlie/)).toHaveCount(0);
  });

  test("model switch sends requestId and reports ambiguous_target", async ({
    page,
  }) => {
    const requestId = "e2e-model-request";
    await installMockBridge(page, {
      managedAgents: [
        {
          name: "Charlie",
          pubkey: AGENT_PUBKEY,
          status: "running",
          channelNames: ["agents"],
        },
      ],
      observerControlResults: [
        {
          type: "switch_model",
          status: "ambiguous_target",
        },
      ],
    });
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await page.waitForFunction(
      () => typeof window.__BUZZ_E2E_RUN_MODEL_SWITCH__ === "function",
    );
    const result = await page.evaluate(
      ({ agentPubkey, channelId, modelId, requestId }) =>
        (window as E2eControlWindow).__BUZZ_E2E_RUN_MODEL_SWITCH__?.({
          agentPubkey,
          channelIds: [channelId],
          modelId,
          requestId,
        }),
      {
        agentPubkey: AGENT_PUBKEY,
        channelId: CHANNEL_AGENTS,
        modelId: "test-model",
        requestId,
      },
    );
    expect(result).toBe("ambiguous");
    await expect
      .poll(() => readControlRequests(page))
      .toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            agentPubkey: AGENT_PUBKEY,
            payload: expect.objectContaining({
              type: "switch_model",
              channelId: CHANNEL_AGENTS,
              modelId: "test-model",
              requestId,
            }),
          }),
        ]),
      );
  });

  test("model switch reports pending when no correlated terminal arrives", async ({
    page,
  }) => {
    const requestId = "e2e-pending-request";
    await installMockBridge(page, {
      observerControlResults: [
        {
          type: "switch_model",
          status: "sent",
          channelId: CHANNEL_FOREIGN,
        },
      ],
    });
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await page.waitForFunction(
      () => typeof window.__BUZZ_E2E_RUN_MODEL_SWITCH__ === "function",
    );
    const resultPromise = page.evaluate(
      ({ agentPubkey, channelId, modelId, requestId }) =>
        (window as E2eControlWindow).__BUZZ_E2E_RUN_MODEL_SWITCH__?.({
          agentPubkey,
          channelIds: [channelId],
          modelId,
          requestId,
          timeoutMs: 50,
        }),
      {
        agentPubkey: AGENT_PUBKEY,
        channelId: CHANNEL_AGENTS,
        modelId: "test-model",
        requestId,
      },
    );
    expect(await resultPromise).toBe("pending");
  });

  test("Stop is disabled for an unscoped restored activity URL", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          name: "Outsider",
          pubkey: RESTORED_UNSCOPED_AGENT_PUBKEY,
          status: "running",
          channelNames: ["random"],
        },
      ],
    });
    await page.goto(
      `/#/channels/${CHANNEL_AGENTS}?agentSession=${RESTORED_UNSCOPED_AGENT_PUBKEY}`,
      { waitUntil: "domcontentloaded" },
    );
    await waitForActiveTurnSeed(page);
    // Seed real work for an agent that is not in the visible channel activity
    // list. The restored URL therefore mounts an unscoped panel (no
    // agentSessionChannel and no matching activity-list channel), so the
    // disabled state proves the missing scope guard rather than inactivity.
    await seedActiveTurn(page, CHANNEL_AGENTS);
    const panel = page.getByTestId("agent-session-thread-panel");
    await expect(panel).toBeVisible({ timeout: 10_000 });
    await expect(page.getByTestId("agent-session-scope-label")).toHaveText(
      "Activity · All channels",
    );
    await page.getByTestId("agent-session-settings-menu-trigger").click();
    await expect(page.getByTestId("agent-session-stop-turn")).toBeDisabled();
    await expect(page.getByTestId("agent-session-stop-turn")).toHaveAttribute(
      "aria-disabled",
      "true",
    );
  });
});
