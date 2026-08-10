import { expect, test } from "@playwright/test";

import type { RelayEvent } from "@/shared/api/types";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const LIVE_PUBKEY = "f".repeat(64);
const STALE_PUBKEY = "e".repeat(64);

function teamCatalogEvent(input: {
  dTag: string;
  eventId: string;
  memberCount: number;
  name: string;
}): RelayEvent {
  return {
    id: input.eventId,
    pubkey: TEST_IDENTITIES.alice.pubkey,
    created_at: 1_721_750_400,
    kind: 30178,
    tags: [
      ["d", input.dTag],
      ["shared", "true"],
    ],
    content: JSON.stringify({
      v: 1,
      name: input.name,
      members: Array.from({ length: input.memberCount }, (_, index) => ({
        member_key: `member-${index + 1}`,
        display_name: `Member ${index + 1}`,
      })),
    }),
    sig: "f".repeat(128),
  };
}

test("Agents shows an independently confirmed read-only shared fleet", async ({
  page,
}) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: LIVE_PUBKEY,
        name: "Clyde",
        model: "Model Alpha",
        channelNames: ["Agent Testing", "x-articles-ozark", "Agent Testing"],
        // Directory status is deliberately stale. Presence below is current.
        status: "offline",
      },
      {
        pubkey: STALE_PUBKEY,
        name: "Stale directory profile",
        channelNames: ["general"],
        status: "online",
      },
    ],
    presenceStatuses: {
      [LIVE_PUBKEY]: "online",
      [STALE_PUBKEY]: "offline",
    },
    teamCatalogEvents: [
      teamCatalogEvent({
        dTag: "deepseek-crew",
        eventId: "1".repeat(64),
        memberCount: 5,
        name: "DeepSeek Crew",
      }),
      teamCatalogEvent({
        dTag: "hermes-canary-team",
        eventId: "2".repeat(64),
        memberCount: 13,
        name: "Hermes Canaries",
      }),
    ],
  });

  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("open-agents-view")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("open-agents-view").click();

  const fleet = page.getByTestId("shared-fleet");
  const liveWorker = fleet.getByTestId(`shared-fleet-worker-${LIVE_PUBKEY}`);
  await expect(liveWorker).toBeVisible();
  await expect(liveWorker).toContainText("Clyde");
  await expect(liveWorker).toContainText("Remote worker");
  await expect(liveWorker).toContainText("Model Alpha");
  await expect(liveWorker).toContainText("Online");
  await expect(liveWorker).toContainText("Agent Testing");
  await expect(liveWorker).toContainText("x-articles-ozark");
  await expect(liveWorker).toContainText(
    "Mention only in its 2 assigned channels",
  );
  await expect(
    fleet.getByTestId(`shared-fleet-worker-${STALE_PUBKEY}`),
  ).toHaveCount(0);
  await expect(fleet.getByRole("button")).toHaveCount(0);

  const teams = page.getByTestId("shared-team-catalog");
  await expect(teams).toContainText("DeepSeek Crew");
  await expect(teams).toContainText("5 members");
  await expect(teams).toContainText("Hermes Canaries");
  await expect(teams).toContainText("13 members");
  await expect(teams.getByRole("button")).toHaveCount(0);
});
