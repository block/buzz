import { expect, test } from "@playwright/test";

import {
  installMockBridge,
  openNewMessagePage,
  TEST_IDENTITIES,
} from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/pubkey-display";

const AGENT_PUBKEY = "cafef00d".repeat(8);

// Screenshot evidence for the pubkey-display work: the canonical profile
// surfaces, plus the new-DM recipient identity-hover states retained by it.

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

test("new-DM agent name swaps to its public key on name hover", async ({
  page,
}) => {
  // Agent rows only surface when the agent is mentionable, so seed a managed
  // agent alongside its search profile.
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
        ownerPubkey: "deadbeef".repeat(8),
        pubkey: AGENT_PUBKEY,
      },
    ],
  });
  await page.goto("/");

  await openNewMessagePage(page);
  await expect(page.getByTestId("new-message-page")).toBeVisible();

  const agentResult = page.getByTestId(`new-dm-result-${AGENT_PUBKEY}`);
  await expect(agentResult).toBeVisible();
  await expect(page.getByTestId("new-dm-loading")).toBeHidden();
  await expect
    .poll(async () => {
      const marker = crypto.randomUUID();
      await agentResult.evaluate((element, value) => {
        element.dataset.e2eSettleMarker = value;
      }, marker);
      await page.waitForTimeout(250);
      return agentResult
        .evaluate(
          (element, value) => element.dataset.e2eSettleMarker === value,
          marker,
        )
        .catch(() => false);
    })
    .toBe(true);

  const agentName = agentResult.getByTestId(`new-dm-name-${AGENT_PUBKEY}`);
  const agentNpub = agentResult.getByTestId(`new-dm-npub-${AGENT_PUBKEY}`);
  await expect(agentResult).toContainText("managed by you");
  await expect(agentName).toContainText("Pinky");
  await expect(agentNpub).toHaveCSS("opacity", "0");

  const agentNameBox = await agentName.boundingBox();
  const agentResultBox = await agentResult.boundingBox();
  expect(agentNameBox).not.toBeNull();
  expect(agentResultBox).not.toBeNull();
  if (!agentNameBox || !agentResultBox) return;
  expect(agentNameBox.width).toBeLessThan(agentResultBox.width / 2);
  await page.mouse.move(
    agentResultBox.x + agentResultBox.width - 12,
    agentNameBox.y + agentNameBox.height / 2,
  );
  await expect(agentNpub).toHaveCSS("opacity", "0");

  // Acquire fresh locators after the directory queries settle: the result row
  // can be replaced while the initial loading skeleton is transitioning out.
  const settledAgentResult = page.getByTestId(`new-dm-result-${AGENT_PUBKEY}`);
  const settledAgentName = settledAgentResult.getByTestId(
    `new-dm-name-${AGENT_PUBKEY}`,
  );
  const settledAgentNpub = settledAgentResult.getByTestId(
    `new-dm-npub-${AGENT_PUBKEY}`,
  );
  await settledAgentName.hover();
  await expect
    .poll(async () =>
      settledAgentName.evaluate((element) => element.matches(":hover")),
    )
    .toBe(true);
  await expect(settledAgentNpub).not.toHaveCSS("opacity", "0");
  await expect(settledAgentNpub).toHaveText("cafef00d…f00d");
  await expect(
    settledAgentName.getByText("Pinky", { exact: true }),
  ).not.toHaveCSS("opacity", "1");
  await expect(settledAgentResult).toContainText("managed by you");
  await waitForAnimations(page);
  await page.getByTestId("new-message-page").screenshot({
    path: `${SHOTS}/new-dm-agent-name-hover.png`,
  });
});

test("selected new-DM recipient can be verified again through search", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");

  await openNewMessagePage(page);
  await expect(page.getByTestId("new-message-page")).toBeVisible();

  const search = page.getByTestId("new-dm-search");
  const charlieResult = page.getByTestId(
    `new-dm-result-${TEST_IDENTITIES.charlie.pubkey}`,
  );
  const charlieName = page.getByTestId(
    `new-dm-name-${TEST_IDENTITIES.charlie.pubkey}`,
  );
  const charlieNpub = page.getByTestId(
    `new-dm-npub-${TEST_IDENTITIES.charlie.pubkey}`,
  );

  // The same name-to-key swap is available in the unfiltered directory,
  // before the user has typed anything.
  await expect(charlieResult).toBeVisible();
  await expect(charlieNpub).toHaveCSS("opacity", "0");
  await charlieName.hover();
  await expect(charlieNpub).toHaveCSS("opacity", "1");
  await expect(charlieNpub).toHaveText(
    `${TEST_IDENTITIES.charlie.pubkey.slice(0, 8)}…${TEST_IDENTITIES.charlie.pubkey.slice(-4)}`,
  );
  await page.mouse.move(1_100, 500);
  await expect(charlieNpub).toHaveCSS("opacity", "0");

  await search.fill("charlie");
  await expect(charlieResult).toBeVisible();
  await page.keyboard.press("Enter");

  const charlieChip = page.getByTestId(
    `new-dm-selected-${TEST_IDENTITIES.charlie.pubkey}`,
  );
  const charlieNameTrigger = page.getByTestId(
    `new-dm-recipient-name-${TEST_IDENTITIES.charlie.pubkey}`,
  );
  const charlieKeyPopover = page.getByTestId(
    `new-dm-selected-key-popover-${TEST_IDENTITIES.charlie.pubkey}`,
  );
  const charliePubkey = page.getByTestId(
    `new-dm-selected-pubkey-${TEST_IDENTITIES.charlie.pubkey}`,
  );
  await expect(charlieChip).toBeVisible();
  await expect(page.getByTestId("new-message-recipient-popover")).toBeVisible();
  await expect(search).toHaveValue("");
  await expect(charlieResult).toHaveCount(0);
  await expect(charlieKeyPopover).toHaveCount(0);
  await charlieChip.hover();
  await expect(charlieKeyPopover).toHaveCount(0);
  await charlieNameTrigger.click();
  await expect(charlieKeyPopover).toBeVisible();
  await expect(charliePubkey).toContainText("npub1");
  await expect(charlieKeyPopover).toContainText(TEST_IDENTITIES.charlie.pubkey);
  await waitForAnimations(page);
  await page.getByTestId("new-message-page").screenshot({
    path: `${SHOTS}/new-dm-selected-recipient-key.png`,
  });
  await page.keyboard.press("Escape");
  await expect(charlieKeyPopover).toHaveCount(0);

  await search.fill("charlie");
  await expect(charlieResult).toBeVisible();
  await expect(charlieResult).toHaveAttribute(
    "aria-label",
    "Already added charlie",
  );
  await charlieName.hover();
  await expect(charlieNpub).toHaveCSS("opacity", "1");
  await expect(charlieNpub).toHaveText(
    `${TEST_IDENTITIES.charlie.pubkey.slice(0, 8)}…${TEST_IDENTITIES.charlie.pubkey.slice(-4)}`,
  );
  await expect(charlieName.getByText("charlie", { exact: true })).toHaveCSS(
    "opacity",
    "0",
  );
  await page.keyboard.press("Enter");

  await expect(search).toHaveValue("");
  await expect(
    page.locator("button[data-testid^='new-dm-selected-']"),
  ).toHaveCount(1);
  await page.mouse.move(1_100, 500);
  await expect(charlieNpub).toHaveCount(0);
  await expect(charlieKeyPopover).toHaveCount(0);

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
