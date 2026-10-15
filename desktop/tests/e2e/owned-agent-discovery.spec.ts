import { expect, test } from "@playwright/test";
import { installMockBridge, openNewMessagePage } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const AGENT = "a7".repeat(32);
const OWNER = "deadbeef".repeat(8);

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    managedAgents: [],
    searchProfiles: [],
    relayAgents: [
      {
        pubkey: AGENT,
        name: "Policy-only Scout",
        ownerPubkey: OWNER,
        status: "unknown",
        respondTo: "owner-only",
        channelNames: [],
        channelIds: [],
      },
    ],
  });
  await page.goto("/");
});

test("New Message keeps authenticated owner without a user-search duplicate", async ({
  page,
}) => {
  await openNewMessagePage(page);
  await page.getByTestId("new-dm-search").fill("Policy-only Scout");
  const row = page.getByTestId(`new-dm-result-${AGENT}`);
  await expect(row).toBeVisible();
  await expect(row).toContainText("managed by you");
  await waitForAnimations(page);
  await row.screenshot({
    path: "test-results/owned-agent-discovery/new-message-owner.png",
  });
});

test("member-add keeps authenticated owner without a user-search duplicate", async ({
  page,
}) => {
  await page.getByTestId("channel-general").click();
  await page.getByTestId("channel-members-trigger").click();
  await page
    .getByTestId("channel-management-search-users")
    .fill("Policy-only Scout");
  const row = page.getByTestId(`channel-user-search-result-${AGENT}`);
  await expect(row).toBeVisible();
  await expect(row).toContainText("managed by you");
  await waitForAnimations(page);
  await row.screenshot({
    path: "test-results/owned-agent-discovery/member-add-owner.png",
  });
});
