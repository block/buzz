import { expect, test, type Page } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/duplicate-agent-avatars";

// Two managed agents sharing the display name "Scout", owned by different
// humans (alice and bob), plus a uniquely named control agent. The inline
// owner avatar must appear only on the duplicate-named agents' rows.
const SCOUT_A_PUBKEY =
  "aaaa000000000000000000000000000000000000000000000000000000000001";
const SCOUT_B_PUBKEY =
  "aaaa000000000000000000000000000000000000000000000000000000000002";
const SOLO_PUBKEY =
  "aaaa000000000000000000000000000000000000000000000000000000000003";

// 8x8 solid teal PNG. A data URL keeps the avatar-image path hermetic — no
// network fetch, no relay proxy — while still exercising Radix's real
// image-load lifecycle (AvatarImage only renders once the img has loaded).
const ALICE_AVATAR_DATA_URL =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAFElEQVR4nGMUWTCNARtgwio6aCUABdEBWlX6zykAAAAASUVORK5CYII=";

async function emitMockMessage(page: Page, content: string, pubkey: string) {
  const event = await page.evaluate(
    ({ msg, pk }) => {
      return (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            pubkey?: string;
          }) => { id: string };
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: msg,
        pubkey: pk,
      });
    },
    { msg: content, pk: pubkey },
  );
  if (!event) {
    throw new Error("Mock message emitter is not installed");
  }
  return event;
}

async function waitForMockLiveSubscription(page: Page) {
  await expect
    .poll(async () =>
      page.evaluate(
        () =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "general",
          }) ?? false,
      ),
    )
    .toBe(true);
}

async function waitForTimelineSettled(page: Page) {
  await expect(page.locator("[data-render-pending]")).toHaveCount(0);
}

test("duplicate-named agents get an inline owner avatar; unique agents do not", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: SCOUT_A_PUBKEY,
        name: "Scout",
        status: "running",
        channelNames: ["general"],
      },
      {
        pubkey: SCOUT_B_PUBKEY,
        name: "Scout",
        status: "running",
        channelNames: ["general"],
      },
      {
        pubkey: SOLO_PUBKEY,
        name: "Ranger",
        status: "running",
        channelNames: ["general"],
      },
    ],
    // Seeded after managed agents, so these override the default
    // owner (the mock viewer) with two distinct humans. Alice carries an
    // avatar image; bob has none, exercising the initials fallback.
    searchProfiles: [
      {
        pubkey: SCOUT_A_PUBKEY,
        displayName: "Scout",
        ownerPubkey: TEST_IDENTITIES.alice.pubkey,
        isAgent: true,
      },
      {
        pubkey: SCOUT_B_PUBKEY,
        displayName: "Scout",
        ownerPubkey: TEST_IDENTITIES.bob.pubkey,
        isAgent: true,
      },
      {
        pubkey: TEST_IDENTITIES.alice.pubkey,
        displayName: "alice",
        avatarUrl: ALICE_AVATAR_DATA_URL,
      },
      {
        pubkey: TEST_IDENTITIES.bob.pubkey,
        displayName: "bob",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page);

  await emitMockMessage(page, "Scout A reporting in.", SCOUT_A_PUBKEY);
  await emitMockMessage(page, "Scout B reporting in.", SCOUT_B_PUBKEY);
  await emitMockMessage(page, "Ranger here, no twin.", SOLO_PUBKEY);
  await waitForTimelineSettled(page);

  const scoutARow = page
    .getByTestId("message-row")
    .filter({ hasText: "Scout A reporting in." });
  const scoutBRow = page
    .getByTestId("message-row")
    .filter({ hasText: "Scout B reporting in." });
  const rangerRow = page
    .getByTestId("message-row")
    .filter({ hasText: "Ranger here, no twin." });

  // Both same-named agents carry the inline owner avatar. The wrapper being
  // visible is not enough — assert actual painted content. Owner profiles
  // arrive via a second-level batch fetch, so give the cold first render
  // time to settle instead of racing it.
  const scoutABadge = scoutARow.getByTestId("message-agent-owner-avatar");
  const scoutBBadge = scoutBRow.getByTestId("message-agent-owner-avatar");
  await expect(scoutABadge).toBeVisible({ timeout: 15_000 });
  await expect(scoutBBadge).toBeVisible({ timeout: 15_000 });

  // Alice has an avatar image — a real, fully loaded <img> must render.
  const aliceImage = scoutABadge.getByTestId(
    "message-agent-owner-avatar-image",
  );
  await expect(aliceImage).toBeVisible({ timeout: 15_000 });
  expect(
    await aliceImage.evaluate(
      (img: HTMLImageElement) => img.complete && img.naturalWidth > 0,
    ),
  ).toBe(true);

  // Bob has no avatar URL — the initials fallback must paint visible text.
  const bobFallback = scoutBBadge.getByTestId(
    "message-agent-owner-avatar-fallback",
  );
  await expect(bobFallback).toBeVisible({ timeout: 15_000 });
  await expect(bobFallback).toHaveText("B");

  // …the uniquely named agent does not.
  await expect(rangerRow.getByTestId("message-agent-owner-avatar")).toHaveCount(
    0,
  );

  // The disambiguated row must not be taller than the plain agent row.
  const scoutBox = await scoutARow.boundingBox();
  const rangerBox = await rangerRow.boundingBox();
  if (!scoutBox || !rangerBox) {
    throw new Error("Expected both message rows to be visible.");
  }
  expect(scoutBox.height).toBe(rangerBox.height);

  await page.screenshot({
    path: `${SHOTS}/01-timeline-duplicate-agents.png`,
    fullPage: false,
  });

  // Hovering the agent's name opens the balloon with the possessive identity.
  await scoutARow.getByTestId("message-author").first().hover();
  const popover = page.getByTestId("user-profile-popover");
  await expect(popover).toBeVisible();
  await expect(popover.getByText("alice's Scout.")).toBeVisible();

  await page.screenshot({
    path: `${SHOTS}/02-balloon-owners-scout.png`,
    fullPage: false,
  });
});
