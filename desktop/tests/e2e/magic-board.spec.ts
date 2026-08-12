import { expect, test, type Page, type TestInfo } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const DISPATCH_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const DISPATCH_CANVAS = `# Dispatch — Open Studio 001

Bring one seed. Leave with one small, shareable artifact.

## This week at Sweet Works

**Open Studio 001: One Seed, One Artifact** is active now.

## Start here

1. Read the welcome.
2. Open a Workshop thread.
3. Name one finish line.

## Help wanted

Use 👋 for “I can help,” then name what you can offer.

## Finished example

**Ora #5821 — The Smallest Edge of Day** completed the full loop.

## Next pilot action

Invite the first 12–20 participants after the example is accessible.
`;

async function openDispatchRoute(page: Page, search = "") {
  await page.goto("/");
  await expect(page.getByTestId("home-inbox")).toBeVisible();
  await page.evaluate(
    async ({ channelId, search }) => {
      window.__BUZZ_E2E_MUTATE_CHANNEL__?.({
        channelId,
        name: "Dispatch",
      });
      await window.__BUZZ_E2E_INVALIDATE_CHANNELS__?.();
      window.location.hash = `#/channels/${channelId}${search}`;
    },
    { channelId: DISPATCH_CHANNEL_ID, search },
  );
}

async function openDispatchBoard(page: Page) {
  await openDispatchRoute(page);
  await expect(page.getByTestId("channel-magic-board")).toBeVisible();
}

async function captureBoard(page: Page, testInfo: TestInfo, name: string) {
  await waitForAnimations(page);
  await page.screenshot({
    path: testInfo.outputPath(`${name}.png`),
    fullPage: true,
  });
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    canvas: {
      author: TEST_IDENTITIES.tyler.pubkey,
      content: DISPATCH_CANVAS,
      updatedAt: 1_786_336_800,
    },
    managedAgents: [
      {
        channelIds: [DISPATCH_CHANNEL_ID],
        channelNames: ["Dispatch"],
        name: "Charlie",
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        status: "running",
      },
    ],
  });
});

test("Dispatch opens as a board and keeps the stream one click away", async ({
  page,
}, testInfo) => {
  await openDispatchBoard(page);

  await expect(page.getByTestId("channel-view-board")).toHaveAttribute(
    "data-state",
    "active",
  );
  await expect(
    page.getByRole("heading", { name: "Dispatch — Open Studio 001" }),
  ).toBeVisible();
  await expect(page.getByText("This week at Sweet Works")).toBeVisible();
  await expect(page.getByTestId("magic-board-card-start-here-2")).toBeVisible();
  await expect(
    page.getByTestId("magic-board-card-help-wanted-3"),
  ).toBeVisible();
  await expect(
    page.getByText("Finished example", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("5 members · 3 agents tending this room."),
  ).toBeVisible();

  await captureBoard(page, testInfo, "magic-board-desktop");

  await page.getByTestId("channel-view-stream").click();
  await expect(page.getByTestId("channel-magic-board")).toBeHidden();
  await expect(page.getByText("Welcome to #general")).toBeVisible();

  await page.getByTestId("channel-view-board").click();
  await expect(page.getByTestId("channel-magic-board")).toBeVisible();
});

test("Dispatch board stacks cleanly at a narrow viewport", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await openDispatchBoard(page);

  const title = page.getByTestId("chat-title");
  await expect(title).toHaveText("Dispatch");
  await expect
    .poll(() =>
      title.evaluate((element) => element.clientWidth >= element.scrollWidth),
    )
    .toBe(true);

  const grid = page.getByTestId("magic-board-grid");
  await expect(grid).toBeVisible();
  const firstCard = page.getByTestId(
    "magic-board-card-this-week-at-sweet-works-1",
  );
  const secondCard = page.getByTestId("magic-board-card-start-here-2");
  const [firstBox, secondBox] = await Promise.all([
    firstCard.boundingBox(),
    secondCard.boundingBox(),
  ]);
  expect(firstBox).not.toBeNull();
  expect(secondBox).not.toBeNull();
  expect(secondBox?.y ?? 0).toBeGreaterThan(
    (firstBox?.y ?? 0) + (firstBox?.height ?? 0),
  );

  await captureBoard(page, testInfo, "magic-board-narrow");
});

test("a Dispatch message deep link opens the stream instead of hiding its target", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await page.evaluate(async (channelId) => {
    window.__BUZZ_E2E_MUTATE_CHANNEL__?.({
      channelId,
      name: "Dispatch",
    });
    await window.__BUZZ_E2E_INVALIDATE_CHANNELS__?.();
    window.location.hash = `#/channels/${channelId}?messageId=mock-general-welcome`;
  }, DISPATCH_CHANNEL_ID);

  await expect(page.getByText("Welcome to #general")).toBeVisible();
  await expect(page.getByTestId("channel-magic-board")).toBeHidden();
  await expect(page.getByTestId("channel-view-mode")).toBeHidden();
});

test("Dispatch Stream-owned route intents are never masked by the board", async ({
  page,
}) => {
  await openDispatchRoute(page, "?autoSend=channel%3Adispatch-draft");
  await expect(page.getByText("Welcome to #general")).toBeVisible();
  await expect(page.getByTestId("channel-magic-board")).toBeHidden();

  await openDispatchRoute(page, `?profile=${TEST_IDENTITIES.bob.pubkey}`);
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
  await expect(page.getByTestId("channel-magic-board")).toBeHidden();

  await openDispatchRoute(
    page,
    `?agentSession=${TEST_IDENTITIES.charlie.pubkey}`,
  );
  await expect(page.getByTestId("agent-session-thread-panel")).toBeVisible();
  await expect(page.getByTestId("channel-magic-board")).toBeHidden();
});
