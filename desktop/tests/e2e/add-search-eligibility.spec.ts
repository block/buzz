import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// External (non-managed) relay agents in the add-member search are gated on
// their kind:10100 `channel_add_policy` — the declaration the relay enforces
// on third-party adds. Paired specs: the deny case proves an opted-out agent
// never reaches the picker; the allow control proves the deny result comes
// from the policy gate, not from a broken search or member-exclusion.

const NOBODY_AGENT_PUBKEY = "77".repeat(32);
const ANYONE_AGENT_PUBKEY = "99".repeat(32);

async function openAddMemberSearch(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("channel-agents").click();
  await page.getByTestId("channel-members-trigger").click();
  await expect(page.getByTestId("members-sidebar")).toBeVisible();
}

test("add search: agent declaring channel_add_policy nobody is never offered", async ({
  page,
}) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: NOBODY_AGENT_PUBKEY,
        name: "vega",
        channelAddPolicy: "nobody",
        channelNames: ["general"],
        respondTo: "anyone",
      },
    ],
  });
  await openAddMemberSearch(page);
  await page.getByTestId("channel-management-search-users").fill("vega");
  await expect(
    page.getByTestId(`channel-user-search-result-${NOBODY_AGENT_PUBKEY}`),
  ).toHaveCount(0);
});

test("add search: control — agent declaring channel_add_policy anyone IS offered", async ({
  page,
}) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: ANYONE_AGENT_PUBKEY,
        name: "lyra",
        channelAddPolicy: "anyone",
        channelNames: ["general"],
        respondTo: "anyone",
      },
    ],
  });
  await openAddMemberSearch(page);
  await page.getByTestId("channel-management-search-users").fill("lyra");
  await expect(
    page.getByTestId(`channel-user-search-result-${ANYONE_AGENT_PUBKEY}`),
  ).toBeVisible();
});
