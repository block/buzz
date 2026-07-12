import { expect, test } from "@playwright/test";

import {
  installMockBridge,
  openNewMessagePage,
  TEST_IDENTITIES,
} from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/pubkey-display";

const MOCK_IDENTITY_PUBKEY = "deadbeef".repeat(8);
const AGENT_PUBKEY = "cafef00d".repeat(8);

// Screenshot evidence for the pubkey-display work: inline full-npub decision
// surfaces, plus the new-DM recipient picker states retained by that work.

test("profile panel Public key row opens the PubKey popover on hover", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const messageRow = page.getByTestId("message-row").first();
  await expect(messageRow).toBeVisible();
  await messageRow.locator("button").first().click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();

  const pubkeyTrigger = page.getByTestId("user-profile-copy-pubkey");
  await expect(pubkeyTrigger).toBeVisible();
  await pubkeyTrigger.hover();

  // Hover-open fires after a 500ms intent delay.
  await expect(page.getByText("hex", { exact: true })).toBeVisible({
    timeout: 3_000,
  });
  await expect(page.getByText("npub", { exact: true })).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/profile-panel-pubkey-hover-popover.png`,
  });
});

test("new-DM agent row keeps the name on hover and shows 'owned by you'", async ({
  page,
}) => {
  // Agent rows only surface when the agent is mentionable (managed or in a
  // shared channel), so seed managedAgents alongside the search profile.
  await installMockBridge(page, {
    managedAgents: [
      {
        name: "Pinky",
        pubkey: AGENT_PUBKEY,
        status: "running",
      },
    ],
    searchProfiles: [
      {
        displayName: "Pinky",
        isAgent: true,
        ownerPubkey: MOCK_IDENTITY_PUBKEY,
        pubkey: AGENT_PUBKEY,
      },
    ],
  });
  await page.goto("/");

  await openNewMessagePage(page);
  await expect(page.getByTestId("new-message-page")).toBeVisible();
  await page.getByTestId("new-dm-search").fill("pinky");

  // The result testid sits on an empty inset overlay button; the visible
  // text lives on the parent row.
  const agentRow = page
    .getByTestId(`new-dm-result-${AGENT_PUBKEY}`)
    .locator("..");
  await expect(agentRow).toBeVisible();
  await expect(agentRow).toContainText("owned by you");

  await agentRow.hover();
  // Hover must ADD the full npub, not swap the name away.
  await expect(agentRow).toContainText("Pinky");
  await expect(page.getByTestId(`new-dm-npub-${AGENT_PUBKEY}`)).toContainText(
    "npub1",
  );
  await waitForAnimations(page);
  await page.getByTestId("new-message-page").screenshot({
    path: `${SHOTS}/new-dm-agent-row-hover-owned-by-you.png`,
  });
});

test("selected new-DM recipient stays a chip while the picker remains open", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");

  await openNewMessagePage(page);
  await expect(page.getByTestId("new-message-page")).toBeVisible();

  await page.getByTestId("new-dm-search").fill("charlie");
  await expect(
    page.getByTestId(`new-dm-result-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toBeVisible();
  await page.keyboard.press("Enter");

  await expect(
    page.getByTestId(`new-dm-selected-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toBeVisible();
  await expect(page.getByTestId("new-message-recipient-popover")).toBeVisible();
  await expect(page.getByTestId("new-dm-search")).toHaveValue("");
  await expect(page.locator("[data-testid^='new-dm-pubkey-']")).toHaveCount(0);

  await waitForAnimations(page);
  await page.getByTestId("new-message-page").screenshot({
    path: `${SHOTS}/new-dm-selected-recipient.png`,
  });
});

test("member removal confirm shows the full npub inline", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.getByTestId("channel-members-trigger").click();
  await expect(page.getByTestId("members-sidebar")).toBeVisible();
  await waitForAnimations(page);
  await page.getByTestId("members-sidebar").screenshot({
    path: `${SHOTS}/members-sidebar.png`,
  });
});
