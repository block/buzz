import { expect, test } from "@playwright/test";

import {
  installMockBridge,
  openNewMessagePage,
  TEST_IDENTITIES,
} from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/ambiguous-agent-owner-suffix";
const CURRENT_OWNER_PUBKEY = "deadbeef".repeat(8);
const FIRST_AGENT_PUBKEY = "1010".repeat(16);
const SECOND_AGENT_PUBKEY = "2020".repeat(16);

const duplicateAgentOptions = {
  relayAgents: [
    {
      pubkey: FIRST_AGENT_PUBKEY,
      name: "Echo",
      respondTo: "allowlist" as const,
      respondToAllowlist: [CURRENT_OWNER_PUBKEY],
    },
    {
      pubkey: SECOND_AGENT_PUBKEY,
      name: "eChO",
      respondTo: "allowlist" as const,
      respondToAllowlist: [CURRENT_OWNER_PUBKEY],
    },
  ],
  searchProfiles: [
    {
      pubkey: FIRST_AGENT_PUBKEY,
      displayName: "Echo",
      ownerPubkey: CURRENT_OWNER_PUBKEY,
      isAgent: true,
    },
    {
      pubkey: SECOND_AGENT_PUBKEY,
      displayName: "eChO",
      ownerPubkey: TEST_IDENTITIES.outsider.pubkey,
      isAgent: true,
    },
  ],
};

test("captures duplicate agent names in mention autocomplete", async ({
  page,
}) => {
  await installMockBridge(page, duplicateAgentOptions);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.getByTestId("message-input").fill("@echo");

  const autocomplete = page
    .getByTestId("message-composer")
    .getByTestId("mention-autocomplete");
  const firstRow = autocomplete.getByTestId(
    `mention-suggestion-${FIRST_AGENT_PUBKEY}`,
  );
  const secondRow = autocomplete.getByTestId(
    `mention-suggestion-${SECOND_AGENT_PUBKEY}`,
  );
  await expect(firstRow.getByTestId("mention-suggestion-name")).toHaveText(
    "Echo (you)",
  );
  await expect(secondRow.getByTestId("mention-suggestion-name")).toHaveText(
    "eChO (outsider)",
  );
  await expect(firstRow).toContainText("owned by you");
  await expect(secondRow).toContainText("owned by outsider");

  await waitForAnimations(page);
  await autocomplete.screenshot({ path: `${SHOTS}/after-mention.png` });
});

test("captures duplicate agent names in add-member results", async ({
  page,
}) => {
  await installMockBridge(page, duplicateAgentOptions);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.getByTestId("channel-members-trigger").click();
  await page.getByTestId("channel-management-search-users").fill("echo");

  const firstRow = page.getByTestId(
    `channel-user-search-result-${FIRST_AGENT_PUBKEY}`,
  );
  const secondRow = page.getByTestId(
    `channel-user-search-result-${SECOND_AGENT_PUBKEY}`,
  );
  await expect(
    firstRow.getByTestId("channel-user-search-result-name"),
  ).toHaveText("Echo (you)");
  await expect(
    secondRow.getByTestId("channel-user-search-result-name"),
  ).toHaveText("eChO (outsider)");
  await expect(
    firstRow.getByRole("button", { name: "Select Echo (you)", exact: true }),
  ).toBeVisible();
  await expect(
    secondRow.getByRole("button", {
      name: "Select eChO (outsider)",
      exact: true,
    }),
  ).toBeVisible();
  await expect(firstRow).toContainText("owned by you");
  await expect(secondRow).toContainText("owned by outsider");

  await waitForAnimations(page);
  await page
    .getByTestId("members-sidebar-people")
    .screenshot({ path: `${SHOTS}/after-add-member.png` });
});

test("captures duplicate agent names in new-message results", async ({
  page,
}) => {
  await installMockBridge(page, duplicateAgentOptions);
  await page.goto("/");
  await openNewMessagePage(page);
  await page.getByTestId("new-dm-search").fill("echo");

  const firstRow = page.getByTestId(`new-dm-result-${FIRST_AGENT_PUBKEY}`);
  const secondRow = page.getByTestId(`new-dm-result-${SECOND_AGENT_PUBKEY}`);
  await expect(
    firstRow
      .getByTestId(`new-dm-name-${FIRST_AGENT_PUBKEY}`)
      .getByText("Echo (you)", { exact: true }),
  ).toBeVisible();
  await expect(
    secondRow
      .getByTestId(`new-dm-name-${SECOND_AGENT_PUBKEY}`)
      .getByText("eChO (outsider)", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("option", { name: "Add Echo (you)", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("option", {
      name: "Add eChO (outsider)",
      exact: true,
    }),
  ).toBeVisible();
  await expect(firstRow).toContainText("owned by you");
  await expect(secondRow).toContainText("owned by outsider");

  await waitForAnimations(page);
  await page
    .getByTestId("new-message-recipient-popover")
    .screenshot({ path: `${SHOTS}/after-new-message.png` });
});
