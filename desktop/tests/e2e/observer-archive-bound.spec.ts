import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

// The archive window keeps channels a panel is showing and only the most
// recently used of the rest. This drives the real panel: its archive survives
// other channels loading while it is open, and after it closes and other
// channels push it out, reopening the panel re-hydrates it from the archive.

const AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301"; // #agents
const READ_COMMAND = "read_archived_observer_events_for_channel";

function systemPromptFrame(channelId: string, seq: number) {
  return {
    seq,
    timestamp: new Date(Date.UTC(2025, 5, 15, 11, 0, seq)).toISOString(),
    kind: "acp_write",
    agentIndex: 0,
    channelId,
    sessionId: `session-${channelId}`,
    turnId: null,
    payload: {
      jsonrpc: "2.0",
      id: 1,
      method: "session/new",
      params: { systemPrompt: `[System]\nArchived prompt for ${channelId}.` },
    },
  };
}

async function seedArchive(
  page: import("@playwright/test").Page,
  channelIds: string[],
  ingest: boolean,
) {
  await page.evaluate(
    ({ agentPubkey, frames, ingest }) =>
      window.__BUZZ_E2E_SEED_ARCHIVED_OBSERVER_FRAMES__?.({
        agentPubkey,
        frames,
        ingest,
      }),
    {
      agentPubkey: AGENT_PUBKEY,
      frames: channelIds.map((channelId, i) => systemPromptFrame(channelId, i)),
      ingest,
    },
  );
}

async function archiveReads(page: import("@playwright/test").Page) {
  return page.evaluate(
    (command) =>
      (
        (window as unknown as Record<string, unknown>)
          .__BUZZ_E2E_IPC_COUNTERS__ as Record<string, number> | undefined
      )?.[command] ?? 0,
    READ_COMMAND,
  );
}

async function openActivityPanel(page: import("@playwright/test").Page) {
  const messageRow = page
    .getByTestId("message-row")
    .filter({ has: page.getByText("Observer Agent", { exact: false }) });
  await expect(messageRow.first()).toBeVisible({ timeout: 8_000 });
  await messageRow.first().getByRole("button").first().click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId(`user-profile-view-activity-${AGENT_PUBKEY}`).click();
  const panel = page.getByTestId("agent-session-thread-panel");
  await expect(panel).toBeVisible({ timeout: 10_000 });
  return panel;
}

const otherChannels = (from: number, count: number) =>
  Array.from({ length: count }, (_, i) => `other-channel-${from + i}`);

test("viewed archive survives other channels loading and re-hydrates after eviction", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: AGENT_PUBKEY,
        name: "Observer Agent",
        status: "running",
        channelNames: ["agents"],
      },
    ],
    saveSubscriptions: [
      {
        scope_type: "owner_p",
        scope_value: "deadbeef".repeat(8),
        kinds: "[24200]",
      },
    ],
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_SEED_ARCHIVED_OBSERVER_FRAMES__ === "function",
  );
  await seedArchive(page, [CHANNEL_ID], false);
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");

  const panel = await openActivityPanel(page);
  const archived = panel.getByText("System prompt");
  await expect(archived).toBeVisible({ timeout: 10_000 });
  const readsWhileOpen = await archiveReads(page);
  expect(readsWhileOpen).toBeGreaterThan(0);

  // Another panel loads ten other channels' archives while this one is open.
  await seedArchive(page, otherChannels(0, 10), true);
  await expect(archived).toBeVisible();

  // Close the panel, then load enough channels to push #agents out.
  await panel.getByRole("button", { name: "Close panel" }).click();
  await expect(panel).toBeHidden();
  await seedArchive(page, otherChannels(10, 4), true);

  const reopened = await openActivityPanel(page);
  await expect(reopened.getByText("System prompt")).toBeVisible({
    timeout: 10_000,
  });
  expect(await archiveReads(page)).toBeGreaterThan(readsWhileOpen);
});
