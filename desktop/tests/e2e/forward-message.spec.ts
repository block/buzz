import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

// PR screenshots for message forwarding: the "Forward message…" entry in the
// more-actions menu and the forward dialog (destination picker + note +
// WYSIWYG preview). Timeline rendering of kind-40009 events is captured via
// `just desktop-screenshot --messages` instead — no interaction needed there.

const SHOTS = "test-results/forward-screenshots";
const RANDOM_CHANNEL_ID = "9dae0116-799b-5071-a0a8-fdd30a91a35d";
const SOURCE_MESSAGE_ID = "a1".repeat(32);
const SOURCE_MESSAGE_CONTENT =
  "Heads up: the beta cluster maintenance window moved to Thursday 14:00 UTC.";

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(async () => {
      return page.evaluate(
        ({ ch }) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: ch }) ??
          false,
        { ch: channelName },
      );
    })
    .toBe(true);
}

/**
 * Navigate to #general, pad the timeline with filler chatter (so the open
 * menu overlays real messages, not the empty-channel onboarding cards), then
 * inject Alice's source message with a known id.
 */
async function seedSourceMessage(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  // Distinct, increasing createdAt values: same-second events tiebreak by id
  // in the timeline, which would shuffle the display order.
  const base = Math.floor(Date.now() / 1000);
  const messages = [
    {
      id: "1a".repeat(32),
      content: "Morning all — the deploy train leaves at noon.",
      pubkey: TEST_IDENTITIES.bob.pubkey,
      createdAt: base - 45,
    },
    {
      id: "2b".repeat(32),
      content: "CI is green on main again after the flaky retry fix.",
      pubkey: TEST_IDENTITIES.charlie.pubkey,
      createdAt: base - 35,
    },
    {
      id: "3c".repeat(32),
      content: "Standup notes are in the usual doc.",
      pubkey: TEST_IDENTITIES.bob.pubkey,
      createdAt: base - 25,
    },
    {
      id: SOURCE_MESSAGE_ID,
      content: SOURCE_MESSAGE_CONTENT,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      createdAt: base - 15,
    },
  ];

  for (const message of messages) {
    await page.evaluate(({ id, content, pubkey, createdAt }) => {
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            pubkey: string;
            id: string;
            createdAt: number;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content,
        pubkey,
        id,
        createdAt,
      });
    }, message);
  }

  const row = page.locator(`[data-message-id="${SOURCE_MESSAGE_ID}"]`);
  await expect(row).toBeVisible({ timeout: 10_000 });
  return row;
}

async function openMoreActionsMenu(page: import("@playwright/test").Page) {
  const row = page.locator(`[data-message-id="${SOURCE_MESSAGE_ID}"]`);
  await row.hover();
  await page.getByTestId(`more-actions-${SOURCE_MESSAGE_ID}`).click();
  await expect(page.locator('[role="menuitem"]').first()).toBeVisible({
    timeout: 5_000,
  });
  return row;
}

test("captures the Forward message entry in the more-actions menu", async ({
  page,
}) => {
  await installMockBridge(page);
  await seedSourceMessage(page);
  const row = await openMoreActionsMenu(page);

  const forwardItem = page.getByTestId(`forward-message-${SOURCE_MESSAGE_ID}`);
  await expect(forwardItem).toBeVisible();
  // Hover the entry so the shot shows it highlighted (also moves the pointer
  // off the trigger, dismissing its tooltip).
  await forwardItem.hover();

  await waitForAnimations(page);

  // Crop to the message row plus the open menu (portaled, so union the boxes).
  const rowBox = await row.boundingBox();
  const menuBox = await page.locator('[role="menu"]').boundingBox();
  if (!rowBox || !menuBox) throw new Error("missing bounding boxes");
  const viewport = page.viewportSize() ?? { width: 1280, height: 720 };
  const pad = 16;
  const x = Math.max(0, Math.min(rowBox.x, menuBox.x) - pad);
  const y = Math.max(0, Math.min(rowBox.y, menuBox.y) - pad);
  const right = Math.min(
    viewport.width,
    Math.max(rowBox.x + rowBox.width, menuBox.x + menuBox.width) + pad,
  );
  const bottom = Math.min(
    viewport.height,
    Math.max(rowBox.y + rowBox.height, menuBox.y + menuBox.height) + pad,
  );
  await page.screenshot({
    path: `${SHOTS}/01-menu-item.png`,
    clip: { x, y, width: right - x, height: bottom - y },
  });
});

test.describe("forward dialog", () => {
  // The dialog caps at 85vh; a taller viewport lets the destination list,
  // note, and preview card all render without inner scrolling.
  test.use({ viewport: { width: 1280, height: 960 } });

  test("captures the forward dialog with destination, note, and preview", async ({
    page,
  }) => {
    await installMockBridge(page);
    await seedSourceMessage(page);
    await openMoreActionsMenu(page);
    await page.getByTestId(`forward-message-${SOURCE_MESSAGE_ID}`).click();

    const dialog = page.getByTestId("forward-message-dialog");
    await expect(dialog).toBeVisible();

    // Pick #random as the destination and confirm the selection took.
    await page.getByTestId(`forward-destination-${RANDOM_CHANNEL_ID}`).click();
    await expect(page.getByTestId("forward-message-submit")).toBeEnabled();

    await page
      .getByTestId("forward-message-note")
      .fill("Adding context: this affects Thursday's launch window.");

    // Preview card renders the original message.
    const preview = page.getByTestId("forward-message-preview");
    await expect(preview).toBeVisible();
    await expect(preview).toContainText("beta cluster maintenance window");

    await waitForAnimations(page);
    await dialog.screenshot({ path: `${SHOTS}/02-forward-dialog.png` });
  });
});
