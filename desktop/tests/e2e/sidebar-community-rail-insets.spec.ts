import { expect, test, type Locator, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const MOCK_PUBKEY = "deadbeef".repeat(8);
const CUSTOM_SECTION = { id: "sec-projects", name: "Projects", order: 0 };
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

async function seedCommunities(page: Page) {
  await page.addInitScript(
    ({ communities, pubkey, section }) => {
      window.localStorage.setItem(
        "buzz-communities",
        JSON.stringify(communities),
      );
      window.localStorage.setItem("buzz-active-community-id", "ws-a");
      window.localStorage.setItem(
        `buzz-channel-sections.v1:${pubkey}`,
        JSON.stringify({ version: 1, sections: [section], assignments: {} }),
      );
    },
    { communities: COMMUNITIES, pubkey: MOCK_PUBKEY, section: CUSTOM_SECTION },
  );
}

async function showUpdateCard(page: Page) {
  await page.evaluate(() => {
    const testWindow = window as Window & {
      __BUZZ_E2E__?: { mock?: { updateAvailable?: boolean } };
    };
    testWindow.__BUZZ_E2E__ = {
      ...(testWindow.__BUZZ_E2E__ ?? {}),
      mock: {
        ...(testWindow.__BUZZ_E2E__?.mock ?? {}),
        updateAvailable: true,
      },
    };
  });

  await page.getByTestId("sidebar-profile-card").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-updates").click();
  await page.getByRole("button", { name: "Check for Updates" }).click();
  await expect(page.getByTestId("settings-panel-updates")).toContainText(
    "Update downloaded. Click to apply.",
  );
  await page.getByTestId("settings-back-to-app").click();
}

async function settleElementAnimations(locator: Locator) {
  await locator.evaluate((element) =>
    Promise.allSettled(
      element.getAnimations().map((animation) => animation.finished),
    ),
  );
}

test("keeps sidebar cards and drag surfaces inset beside the community rail", async ({
  page,
}) => {
  await seedCommunities(page);
  await installMockBridge(page, undefined, { skipCommunitySeed: true });
  await page.goto("/");
  await expect(page.getByTestId("community-rail")).toBeVisible();

  await showUpdateCard(page);

  const sidebar = page.getByTestId("app-sidebar");
  const updateCard = page.getByTestId("sidebar-update-card");
  await expect(updateCard).toBeVisible();
  await settleElementAnimations(updateCard);
  const [sidebarBox, updateCardBox] = await Promise.all([
    sidebar.boundingBox(),
    updateCard.boundingBox(),
  ]);
  expect(sidebarBox).not.toBeNull();
  expect(updateCardBox).not.toBeNull();
  expect((updateCardBox?.x ?? 0) - (sidebarBox?.x ?? 0)).toBeCloseTo(8, 0);

  const sectionTitle = page.getByTestId(`section-title-${CUSTOM_SECTION.id}`);
  await expect(sectionTitle).toBeVisible();
  const sectionDropTarget = sectionTitle.locator(
    'xpath=ancestor::div[@data-sidebar="group"]/..',
  );
  const sectionDropTargetBox = await sectionDropTarget.boundingBox();
  expect(sectionDropTargetBox).not.toBeNull();
  expect((sectionDropTargetBox?.x ?? 0) - (sidebarBox?.x ?? 0)).toBeCloseTo(
    3,
    0,
  );

  const channelRow = page.getByTestId("channel-agents");
  await expect(channelRow).toBeVisible();
  const channelRowBox = await channelRow.boundingBox();
  expect(channelRowBox).not.toBeNull();
  expect((channelRowBox?.x ?? 0) - (sidebarBox?.x ?? 0)).toBeCloseTo(11, 0);
  if (!channelRowBox || !sectionDropTargetBox) return;

  await page.mouse.move(
    channelRowBox.x + channelRowBox.width / 2,
    channelRowBox.y + channelRowBox.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    channelRowBox.x + channelRowBox.width / 2,
    channelRowBox.y + channelRowBox.height / 2 + 8,
    { steps: 5 },
  );

  const dragOverlay = page.getByTestId("sidebar-channel-drag-overlay");
  await expect(dragOverlay).toBeVisible();
  await page.mouse.move(
    sectionDropTargetBox.x + sectionDropTargetBox.width - 16,
    sectionDropTargetBox.y + sectionDropTargetBox.height / 2,
    { steps: 12 },
  );

  await expect(sectionDropTarget).toHaveClass(/ring-2/);
  const dragOverlayBox = await dragOverlay.boundingBox();
  expect(dragOverlayBox).not.toBeNull();
  expect(
    (dragOverlayBox?.x ?? 0) - (sidebarBox?.x ?? 0),
  ).toBeGreaterThanOrEqual(10);
  await page.mouse.up();
});
