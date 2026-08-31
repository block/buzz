import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { FEATURE_OVERRIDES_STORAGE_KEY } from "../helpers/features";

const BESTIE_PUBKEY =
  "be571e0000000000000000000000000000000000000000000000000000000000";
const OWNER_PUBKEY = "deadbeef".repeat(8);
const RELAY_A = "ws://localhost:3000";
const COMMUNITY_A = {
  addedAt: "2026-01-01T00:00:00.000Z",
  id: "bestie-community-a",
  name: "Alpha",
  relayUrl: RELAY_A,
};
const COMMUNITY_B = {
  addedAt: "2026-01-02T00:00:00.000Z",
  id: "bestie-community-b",
  name: "Bravo",
  relayUrl: "ws://localhost:3001",
};

const bestie = {
  avatarUrl: null,
  name: "Bestie",
  personaId: "builtin:bestie",
  pubkey: BESTIE_PUBKEY,
  relayUrl: RELAY_A,
  status: "running" as const,
};

async function seedCommunities(
  page: import("@playwright/test").Page,
  activeId = COMMUNITY_A.id,
) {
  await page.addInitScript(
    ({ active, communities }) => {
      window.localStorage.setItem(
        "buzz-communities",
        JSON.stringify(communities),
      );
      window.localStorage.setItem("buzz-active-community-id", active);
    },
    { active: activeId, communities: [COMMUNITY_A, COMMUNITY_B] },
  );
}

test("the enabled Bestie experiment adds a direct-message entry below Agents", async ({
  page,
}) => {
  await installMockBridge(page, { managedAgents: [bestie] });
  await page.goto("/");

  const agentsEntry = page.getByTestId("open-agents-view");
  const bestieEntry = page.getByTestId("open-bestie-dm");
  await expect(bestieEntry).toBeVisible();
  await expect(bestieEntry).toContainText("Bestie");

  const [agentsBox, bestieBox] = await Promise.all([
    agentsEntry.boundingBox(),
    bestieEntry.boundingBox(),
  ]);
  expect(agentsBox).not.toBeNull();
  expect(bestieBox).not.toBeNull();
  expect(bestieBox?.y).toBeGreaterThan(agentsBox?.y ?? 0);

  await bestieEntry.click();
  await expect(page.getByTestId("chat-title")).toHaveText("Bestie");
});

test("the disabled Bestie experiment does not mount the sidebar entry", async ({
  page,
}) => {
  await installMockBridge(page, { managedAgents: [bestie] });
  await page.addInitScript((key) => {
    const overrides = JSON.parse(
      window.localStorage.getItem(key) ?? "{}",
    ) as Record<string, boolean>;
    overrides.bestie = false;
    window.localStorage.setItem(key, JSON.stringify(overrides));
  }, FEATURE_OVERRIDES_STORAGE_KEY);
  await page.goto("/");

  await expect(page.getByTestId("open-bestie-dm")).toHaveCount(0);
});

test("a delayed Bestie open is scoped to its rendered community and signer", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { managedAgents: [bestie], openDmDelayMs: 1_000 },
    { skipCommunitySeed: true },
  );
  await seedCommunities(page);
  await page.goto("/");

  await page.getByTestId("open-bestie-dm").click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_COMMAND_LOG__?.findLast(
            (entry) => entry.command === "open_dm",
          )?.payload,
      ),
    )
    .toBeTruthy();
  const openDmPayload = await page.evaluate(
    () =>
      window.__BUZZ_E2E_COMMAND_LOG__?.findLast(
        (entry) => entry.command === "open_dm",
      )?.payload,
  );
  expect(openDmPayload).toMatchObject({
    expectedRelayUrl: RELAY_A,
    expectedSignerPubkey: OWNER_PUBKEY,
    pubkeys: [BESTIE_PUBKEY],
  });

  await page.getByTestId(`community-rail-button-${COMMUNITY_B.id}`).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.localStorage.getItem("buzz-active-community-id"),
      ),
    )
    .toBe(COMMUNITY_B.id);

  await page.waitForTimeout(1_150);
  await expect(page.getByTestId("chat-title")).toHaveCount(0);
  await expect(page.getByTestId("open-bestie-dm")).toHaveCount(0);
});
