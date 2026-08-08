import { expect, type Page, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

/**
 * Hosted-community per-owner limit (#4160).
 *
 * The relay derives the limit from `BUZZ_MAX_COMMUNITIES_PER_OWNER` and reports
 * it as `max_communities_per_owner`; the desktop used to hardcode 5, so any
 * deployment that overrode the default gated and worded its copy wrong in both
 * directions. These specs drive the settings card through the mock bridge with
 * the relay reporting different limits and capture what the owner actually
 * sees.
 */
const OUTDIR = "test-results/hosted-community-limit";
const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const RECIPIENT_NPUB = `npub1${"q".repeat(58)}`;

// The card scrolls inside the settings pane: at the stock 720px viewport an
// element screenshot clips before the create form, which is where the limit
// gate and its copy live. Give it room so one frame shows the count, the rows,
// and the create form together.
test.use({ viewport: { width: 1280, height: 1200 } });

const CONNECTED_COMMUNITIES = [
  {
    id: "ws-a",
    name: "Alpha",
    relayUrl: "ws://localhost:3000",
    addedAt: "2026-01-01T00:00:00.000Z",
  },
  {
    id: "ws-b",
    name: "Bravo",
    relayUrl: "ws://localhost:3001",
    addedAt: "2026-01-02T00:00:00.000Z",
  },
];

function communities(count: number) {
  return Array.from({ length: count }, (_, index) => ({
    id: `community-${index + 1}`,
    name: `Team ${index + 1}`,
    normalized_host: `team-${index + 1}.communities.buzz.xyz`,
  }));
}

async function openHostedCommunities(page: Page) {
  await page.goto("/");
  await openSettings(page, "hosted-communities");
  await expect(page.getByTestId("hosted-communities-settings")).toBeVisible();
}

async function capture(page: Page, name: string) {
  await waitForAnimations(page);
  await page
    .getByTestId("hosted-communities-settings")
    .screenshot({ path: `${OUTDIR}/${name}.png` });
}

test("capture: a relay-reported limit above the default ungates a sixth community", async ({
  page,
}) => {
  await installMockBridge(page, {
    builderlabAuth: {
      email: "owner@example.com",
      expiresAt: "2099-01-01T00:00:00Z",
    },
    builderlabIdentity: { pubkey_hex: DEFAULT_MOCK_PUBKEY },
    builderlabCommunities: communities(5),
    builderlabCommunityLimit: 7,
  });
  await openHostedCommunities(page);

  // The owner has exactly as many communities as the old hardcoded limit, so
  // before the fix this page read "5 of 5 used" and refused to create another.
  await expect(page.getByText("5 of 7 used")).toBeVisible();
  await expect(page.getByText(/reached the limit of/)).toHaveCount(0);
  await expect(page.getByLabel("Community address")).toBeEnabled();

  await capture(page, "01-limit-above-default-ungated");
});

test("capture: a relay-reported limit below the default gates early with its own number", async ({
  page,
}) => {
  await installMockBridge(page, {
    builderlabAuth: {
      email: "owner@example.com",
      expiresAt: "2099-01-01T00:00:00Z",
    },
    builderlabIdentity: { pubkey_hex: DEFAULT_MOCK_PUBKEY },
    builderlabCommunities: communities(3),
    builderlabCommunityLimit: 3,
  });
  await openHostedCommunities(page);

  await expect(page.getByText("3 of 3 used")).toBeVisible();
  await expect(
    page.getByText(
      "You’ve reached the limit of 3 hosted communities. Transfer one to free up a slot before creating another.",
    ),
  ).toBeVisible();
  await expect(page.getByLabel("Community address")).toBeDisabled();

  await capture(page, "02-limit-below-default-gated");
});

test("capture: a relay that reports no limit keeps the stock default", async ({
  page,
}) => {
  await installMockBridge(page, {
    builderlabAuth: {
      email: "owner@example.com",
      expiresAt: "2099-01-01T00:00:00Z",
    },
    builderlabIdentity: { pubkey_hex: DEFAULT_MOCK_PUBKEY },
    builderlabCommunities: communities(5),
  });
  await openHostedCommunities(page);

  // No `max_communities_per_owner` on the wire (older relay, or a Builderlab
  // hop that doesn't forward it yet) must behave exactly as it did before.
  await expect(page.getByText("5 of 5 used")).toBeVisible();
  await expect(
    page.getByText(
      "You’ve reached the limit of 5 hosted communities. Transfer one to free up a slot before creating another.",
    ),
  ).toBeVisible();

  await capture(page, "03-no-reported-limit-falls-back");
});

test("capture: a limit_reached create rejection adopts the relay's number", async ({
  page,
}) => {
  await installMockBridge(page, {
    builderlabAuth: {
      email: "owner@example.com",
      expiresAt: "2099-01-01T00:00:00Z",
    },
    builderlabIdentity: { pubkey_hex: DEFAULT_MOCK_PUBKEY },
    builderlabCommunities: communities(3),
    // The list response carried no limit, so the page opens on the fallback.
    builderlabCreateError: { code: "limit_reached", limit: 3 },
  });
  await openHostedCommunities(page);
  await expect(page.getByText("3 of 5 used")).toBeVisible();

  await page.getByLabel("Community address").fill("north-star");
  await expect(page.getByText("That address is available.")).toBeVisible();
  await page.getByRole("button", { name: "Create and connect" }).click();

  // The 409 reports the deployment's real limit: the copy names it and the
  // card re-gates against it instead of the hardcoded 5.
  await expect(
    page.getByText("You’ve reached the limit of 3 hosted communities.").first(),
  ).toBeVisible();
  await expect(page.getByText("3 of 3 used")).toBeVisible();
  await expect(page.getByLabel("Community address")).toBeDisabled();

  await capture(page, "04-create-rejection-adopts-limit");
});

test("capture: the add-community create flow gates on the relay's limit", async ({
  page,
}) => {
  // The community rail (and its add button) only renders with more than one
  // connected community, so seed them the way add-community-screenshots does.
  await page.addInitScript((connected) => {
    window.localStorage.setItem("buzz-communities", JSON.stringify(connected));
    window.localStorage.setItem("buzz-active-community-id", connected[0].id);
  }, CONNECTED_COMMUNITIES);
  await installMockBridge(
    page,
    {
      builderlabAuth: {
        email: "owner@example.com",
        expiresAt: "2099-01-01T00:00:00Z",
      },
      builderlabIdentity: { pubkey_hex: DEFAULT_MOCK_PUBKEY },
      builderlabCommunities: communities(4),
      builderlabCommunityLimit: 4,
    },
    { skipCommunitySeed: true },
  );
  await page.goto("/");
  await page.getByTestId("community-rail-add").click();
  await page.getByTestId("add-community-create").click();

  const dialog = page.getByTestId("add-community-dialog");
  await expect(page.getByTestId("hosted-community-create-name")).toBeDisabled();
  await expect(
    dialog.getByText("You’ve reached the limit of 4 hosted communities."),
  ).toBeVisible();

  await waitForAnimations(page);
  await dialog.screenshot({ path: `${OUTDIR}/06-create-flow-gated.png` });
});

test("capture: a limit_reached transfer rejection names the recipient's limit", async ({
  page,
}) => {
  await installMockBridge(page, {
    builderlabAuth: {
      email: "owner@example.com",
      expiresAt: "2099-01-01T00:00:00Z",
    },
    builderlabIdentity: { pubkey_hex: DEFAULT_MOCK_PUBKEY },
    builderlabCommunities: communities(2),
    builderlabCommunityLimit: 7,
    builderlabTransferError: { code: "limit_reached", limit: 9 },
  });
  await openHostedCommunities(page);
  await expect(page.getByText("2 of 7 used")).toBeVisible();

  const row = page
    .getByTestId("hosted-community-row")
    .filter({ hasText: "Team 1" });
  await row.getByRole("button", { name: "Transfer" }).click();
  await page.getByLabel("Recipient npub").fill(RECIPIENT_NPUB);
  await page.getByRole("button", { name: "Transfer ownership" }).click();

  // The relay rejects a transfer on the *transferee's* quota, so the copy must
  // be about them — not the owner who is giving the community away.
  const rejection = page.getByText(
    "That person already owns the limit of 9 hosted communities, so they can’t receive another.",
  );
  await expect(rejection).toBeVisible();
  await expect(page.getByText(/You’ve reached the limit/)).toHaveCount(0);

  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByLabel("Recipient npub")).toHaveCount(0);
  await expect(page.getByText("2 of 9 used")).toBeVisible();

  await capture(page, "05-transfer-rejection-names-recipient");
});
