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

test("Agents lists an owned agent this device does not run, read-only", async ({
  page,
}) => {
  await openAgentsView(page);
  const group = page.getByTestId("running-elsewhere-agents");
  await expect(group).toContainText("Running elsewhere");
  const card = page.getByTestId(`running-elsewhere-agent-${AGENT}`);
  await expect(card).toContainText("Policy-only Scout");
  await expect(
    card.getByRole("img", { name: "Not managed on this device" }),
  ).toBeVisible();
  await expect(card.getByTestId(`agent-runtime-start-${AGENT}`)).toHaveCount(0);
  await waitForAnimations(page);
  await group.screenshot({
    path: "test-results/owned-agent-discovery/agents-running-elsewhere.png",
  });

  await card
    .getByRole("button", { name: "Policy-only Scout agent profile" })
    .click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
});

// The first load occasionally misses the mock bridge; retry once, as
// agents.spec.ts's gotoApp does.
async function openAgentsView(page: import("@playwright/test").Page) {
  const nav = page.getByTestId("open-agents-view");
  try {
    await expect(nav).toBeVisible({ timeout: 10_000 });
  } catch {
    await page.reload({ waitUntil: "domcontentloaded" });
    await expect(nav).toBeVisible({ timeout: 10_000 });
  }
  await nav.click();
}
