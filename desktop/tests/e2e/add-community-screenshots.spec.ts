import { expect, test } from "@playwright/test";
import { npubEncode } from "nostr-tools/nip19";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const OUTDIR = "test-results/add-community";
const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const COMMUNITIES = [
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

test.beforeEach(async ({ page }) => {
  await page.addInitScript((communities) => {
    window.localStorage.setItem(
      "buzz-communities",
      JSON.stringify(communities),
    );
    window.localStorage.setItem("buzz-active-community-id", communities[0].id);
  }, COMMUNITIES);
  await installMockBridge(
    page,
    {
      builderlabAuth: {
        email: "owner@example.com",
        expiresAt: "2099-01-01T00:00:00Z",
      },
      builderlabIdentity: { pubkey_hex: DEFAULT_MOCK_PUBKEY },
    },
    {
      skipCommunitySeed: true,
    },
  );
  await page.goto("/");
  await page.getByTestId("community-rail-add").click();
});

test("capture: add-community choices", async ({ page }) => {
  const dialog = page.getByTestId("add-community-dialog");
  await dialog.waitFor();
  await waitForAnimations(page);
  await dialog.screenshot({ path: `${OUTDIR}/01-choices.png` });
});

test("capture: join an existing community", async ({ page }) => {
  await page.getByTestId("add-community-join").click();
  const dialog = page.getByTestId("add-community-dialog");
  const communityUrl = page.getByLabel("Community URL or invite link");
  await communityUrl.fill("community.example.com");
  await page.getByTestId("community-api-token-reveal").waitFor({
    state: "detached",
  });
  await waitForAnimations(page);
  await dialog.screenshot({ path: `${OUTDIR}/02-join.png` });
});

test("capture: create a new community", async ({ page }) => {
  await page.getByTestId("add-community-create").click();
  const dialog = page.getByTestId("add-community-dialog");
  await page.getByLabel("Community address").waitFor();
  await waitForAnimations(page);
  await dialog.screenshot({ path: `${OUTDIR}/03-create.png` });
});

test("identity: create owner shows the bound key's npub, never the hosted npub or raw hex", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      builderlabAuth: {
        email: "old-owner@example.com",
        expiresAt: "2099-01-01T00:00:00Z",
      },
      builderlabIdentity: {
        pubkey_hex: "f".repeat(64),
        npub: npubEncode("b".repeat(64)),
      },
    },
    { skipCommunitySeed: true },
  );
  await page.reload();
  await page.getByTestId("community-rail-add").click();
  await page.getByTestId("add-community-create").click();
  await expect(
    page.getByText("This Builderlab account uses a different Buzz identity."),
  ).toBeVisible();
  await expect(
    page.getByText(`Account: ${npubEncode("f".repeat(64))}`),
  ).toBeVisible();
  await expect(
    page.getByText(`This device: ${npubEncode(DEFAULT_MOCK_PUBKEY)}`),
  ).toBeVisible();
  await expect(page.getByText(npubEncode("b".repeat(64)))).toHaveCount(0);
  await expect(page.getByText("f".repeat(64))).toHaveCount(0);
});

test("identity: create owner renders the neutral label when the bound hex is unusable", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      builderlabAuth: {
        email: "old-owner@example.com",
        expiresAt: "2099-01-01T00:00:00Z",
      },
      builderlabIdentity: {
        // Valid hex alphabet, wrong length — unusable as an identity key.
        pubkey_hex: "f".repeat(63),
        npub: npubEncode("b".repeat(64)),
      },
    },
    { skipCommunitySeed: true },
  );
  await page.reload();
  await page.getByTestId("community-rail-add").click();
  await page.getByTestId("add-community-create").click();
  await expect(
    page.getByText("This Builderlab account uses a different Buzz identity."),
  ).toBeVisible();
  await expect(page.getByText("Account: Unavailable")).toBeVisible();
  await expect(page.getByText(npubEncode("b".repeat(64)))).toHaveCount(0);
  await expect(page.getByText("f".repeat(63))).toHaveCount(0);
});
