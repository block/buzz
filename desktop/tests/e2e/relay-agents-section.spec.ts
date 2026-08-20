import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const MOCK_IDENTITY_PUBKEY = "deadbeef".repeat(8);
const OWNED_REMOTE_AGENT_PUBKEY = "a7".repeat(32);
const DM_ONLY_REMOTE_AGENT_PUBKEY = "b7".repeat(32);

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: OWNED_REMOTE_AGENT_PUBKEY,
        name: "AM Window",
        channelNames: ["general", "random"],
        respondTo: "anyone",
      },
      {
        pubkey: DM_ONLY_REMOTE_AGENT_PUBKEY,
        name: "Quinn",
        respondTo: "owner-only",
      },
    ],
    searchProfiles: [
      {
        pubkey: OWNED_REMOTE_AGENT_PUBKEY,
        displayName: "AM Window",
        ownerPubkey: MOCK_IDENTITY_PUBKEY,
        isAgent: true,
      },
    ],
  });
});

async function openAgentsView(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("open-agents-view")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-page-content")).toBeVisible();
}

test("relay-hosted agents render in a read-only directory section", async ({
  page,
}) => {
  await openAgentsView(page);

  const section = page.getByTestId("agents-on-this-relay");
  await section.scrollIntoViewIfNeeded();
  await expect(section).toBeVisible();
  await expect(section.getByText("On this relay")).toBeVisible();

  // Channel-scoped agent: membership summary, audience, and the owner badge
  // sourced from its kind:0 NIP-OA ownership (profile batch).
  const ownedCard = section.getByTestId(
    `relay-agent-card-${OWNED_REMOTE_AGENT_PUBKEY}`,
  );
  await expect(ownedCard).toBeVisible();
  await expect(ownedCard).toContainText("AM Window");
  await expect(ownedCard).toContainText("2 channels · anyone");
  await expect(ownedCard).toContainText("Owned by you");

  // DM-only agent: no channels in its directory record, no ownership proof
  // seeded, so no badge.
  const dmOnlyCard = section.getByTestId(
    `relay-agent-card-${DM_ONLY_REMOTE_AGENT_PUBKEY}`,
  );
  await expect(dmOnlyCard).toBeVisible();
  await expect(dmOnlyCard).toContainText("Quinn");
  await expect(dmOnlyCard).toContainText("No channels · owner only");
  await expect(dmOnlyCard).not.toContainText("Owned by you");

  // The cards are read-only: no start button and no actions menu anywhere in
  // the section.
  await expect(section.getByRole("button", { name: /start/i })).toHaveCount(0);
  await expect(section.getByRole("button", { name: /actions/i })).toHaveCount(
    0,
  );

  await waitForAnimations(page);
  await section.screenshot({
    path: "test-results/relay-agents-section/section.png",
  });
});

test("relay records mirroring local managed agents stay in the managed section", async ({
  page,
}) => {
  await openAgentsView(page);

  const section = page.getByTestId("agents-on-this-relay");
  await section.scrollIntoViewIfNeeded();
  await expect(section).toBeVisible();

  // The mock bridge mirrors every managed agent into the relay directory
  // (syncMockRelayAgentsFromManagedAgents). None of those pubkeys may render
  // here — the managed section above already shows them.
  const managedPubkeys = await page.evaluate(async () => {
    const internals = (
      window as unknown as {
        __TAURI_INTERNALS__: {
          invoke: (cmd: string) => Promise<{ pubkey: string }[]>;
        };
      }
    ).__TAURI_INTERNALS__;
    const agents = await internals.invoke("list_managed_agents");
    return agents.map((agent) => agent.pubkey);
  });
  for (const pubkey of managedPubkeys) {
    await expect(section.getByTestId(`relay-agent-card-${pubkey}`)).toHaveCount(
      0,
    );
  }
});
