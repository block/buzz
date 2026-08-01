import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// The dev-mode agent status line: a quiet per-channel readout pinned between
// the transcript and the composer. It appears while an agent has an active
// turn in the channel, shows the newest headline from the agent's observer
// transcript ("working…" until one arrives), and clears when the turn ends.

// alice — agent member of #general in the mock bridge.
const ALICE_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";
const CHANNEL_GENERAL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

async function openDevModeGeneral(page: import("@playwright/test").Page) {
  await installMockBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("dev-mode-composer").waitFor();
  await page
    .getByTestId("dev-mode-channel-navigator")
    .getByText("# general", { exact: true })
    .click();
  await page.getByTestId("dev-mode-transcript").waitFor();
}

function seedTurnEvent(
  page: import("@playwright/test").Page,
  kind: "turn_started" | "turn_completed",
) {
  return page.evaluate(
    ({ pubkey, channelId, eventKind }) => {
      window.__BUZZ_E2E_SEED_ACTIVE_TURNS__?.({
        agentPubkey: pubkey,
        channelId,
        turnId: "turn-status-1",
        kind: eventKind,
      });
    },
    { pubkey: ALICE_PUBKEY, channelId: CHANNEL_GENERAL, eventKind: kind },
  );
}

test("status line tracks an agent turn from start to completion", async ({
  page,
}) => {
  await openDevModeGeneral(page);

  // Idle channel — no status line reserved.
  await expect(page.getByTestId("dev-mode-agent-status")).toHaveCount(0);
  await expect(page.getByTestId("dev-mode-working-channel-name")).toHaveCount(
    0,
  );
  const idleTopbarBounds = await page
    .getByTestId("dev-mode-topbar-channel")
    .boundingBox();
  expect(idleTopbarBounds).not.toBeNull();

  await seedTurnEvent(page, "turn_started");

  const status = page.getByTestId("dev-mode-agent-status");
  await expect(status).toBeVisible();
  const row = status.getByTestId("dev-mode-agent-status-row");
  await expect(row).toContainText("alice");
  // No observer transcript signal yet — quiet fallback.
  await expect(row).toContainText("working…");

  // Every visible developer-mode name for this channel gets the same moving
  // color spotlight: navigator row, main tab, and top bar. The text itself
  // remains one stationary run of glyphs.
  const workingNames = page.getByTestId("dev-mode-working-channel-name");
  await expect(workingNames).toHaveCount(3);
  await expect(workingNames.nth(0)).toHaveAttribute(
    "data-channel-name",
    "general",
  );
  await expect(workingNames.nth(0)).toHaveCSS(
    "animation-name",
    "dev-working-channel-spotlight",
  );
  await expect(workingNames.nth(0)).toHaveCSS("transform", "none");
  await expect(workingNames.nth(0).locator("span")).toHaveCount(0);
  const spotlightPositionBefore = await workingNames
    .nth(0)
    .evaluate((node) =>
      getComputedStyle(node).getPropertyValue("background-position"),
    );
  await page.waitForTimeout(120);
  const spotlightPositionAfter = await workingNames
    .nth(0)
    .evaluate((node) =>
      getComputedStyle(node).getPropertyValue("background-position"),
    );
  expect(spotlightPositionAfter).not.toBe(spotlightPositionBefore);
  const workingTopbarBounds = await page
    .getByTestId("dev-mode-topbar-channel")
    .boundingBox();
  expect(workingTopbarBounds).not.toBeNull();
  expect(workingTopbarBounds?.x).toBeCloseTo(idleTopbarBounds?.x ?? 0, 2);
  expect(workingTopbarBounds?.y).toBeCloseTo(idleTopbarBounds?.y ?? 0, 2);
  expect(workingTopbarBounds?.width).toBeCloseTo(
    idleTopbarBounds?.width ?? 0,
    2,
  );
  expect(workingTopbarBounds?.height).toBeCloseTo(
    idleTopbarBounds?.height ?? 0,
    2,
  );

  // A thought frame upgrades the fallback to a real headline.
  await page.evaluate(
    ({ pubkey, channelId }) => {
      window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
        agentPubkey: pubkey,
        events: [
          {
            seq: 5001,
            timestamp: new Date().toISOString(),
            kind: "acp_read",
            agentIndex: 0,
            channelId,
            sessionId: "session-status-1",
            turnId: "turn-status-1",
            payload: {
              method: "session/update",
              params: {
                sessionId: "session-status-1",
                update: {
                  sessionUpdate: "agent_thought_chunk",
                  content: { type: "text", text: "Scanning the repo" },
                },
              },
            },
          },
        ],
      });
    },
    { pubkey: ALICE_PUBKEY, channelId: CHANNEL_GENERAL },
  );

  await expect(row).toContainText("Thinking");

  await seedTurnEvent(page, "turn_completed");
  await expect(page.getByTestId("dev-mode-agent-status")).toHaveCount(0);
  await expect(workingNames).toHaveCount(0);
});
