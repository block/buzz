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

  const authoredCards = page.locator("[data-testid^='magic-board-card-']");
  await expect(authoredCards).toHaveCount(5);
  for (const card of await authoredCards.all()) {
    await card.hover();
    await expect(card).toBeVisible();
    await expect
      .poll(() =>
        card.evaluate((element) => getComputedStyle(element).transform),
      )
      .toBe("none");
  }

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

  await page.getByTestId("magic-board-create-card").click();
  const editor = page.getByTestId("magic-board-card-editor");
  await expect(editor).toBeVisible();
  const editorBox = await editor.boundingBox();
  expect(editorBox).not.toBeNull();
  expect(editorBox?.width ?? 391).toBeLessThanOrEqual(358);
  await expect(page.getByTestId("magic-board-card-save")).toBeVisible();
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(editor).toBeHidden();
});

test("a steward can create, edit, and drag a card into a durable order", async ({
  page,
}) => {
  await openDispatchBoard(page);

  await page.getByTestId("magic-board-create-card").click();
  await expect(page.getByTestId("magic-board-card-editor")).toBeVisible();
  await page.getByTestId("magic-board-card-title").fill("Fresh signal");
  await page
    .getByTestId("magic-board-card-body")
    .fill("A **new** shared card written from the Board.");
  await page.getByTestId("magic-board-card-save").click();

  const createdCard = page.getByTestId("magic-board-card-fresh-signal-6");
  await expect(createdCard).toContainText("Fresh signal");
  await expect(createdCard).toContainText(
    "A new shared card written from the Board.",
  );

  await page.getByTestId("magic-board-edit-fresh-signal-6").click();
  await page.getByTestId("magic-board-card-title").fill("Fresh signal updated");
  await page
    .getByTestId("magic-board-card-body")
    .fill("Edited **in place** without opening raw canvas settings.");
  await page.getByTestId("magic-board-card-save").click();

  const updatedCard = page.getByTestId(
    "magic-board-card-fresh-signal-updated-6",
  );
  await expect(updatedCard).toContainText("Fresh signal updated");
  await expect(updatedCard).toContainText(
    "Edited in place without opening raw canvas settings.",
  );

  const dragHandle = page.getByTestId(
    "magic-board-drag-fresh-signal-updated-6",
  );
  await expect(dragHandle).toBeEnabled();
  const firstCard = page.getByTestId(
    "magic-board-card-this-week-at-sweet-works-1",
  );
  const [dragBox, targetBox] = await Promise.all([
    dragHandle.boundingBox(),
    firstCard.boundingBox(),
  ]);
  expect(dragBox).not.toBeNull();
  expect(targetBox).not.toBeNull();
  if (!dragBox || !targetBox) {
    throw new Error("Board drag geometry is unavailable.");
  }

  await dragHandle.hover();
  await page.mouse.down();
  await page.mouse.move(
    dragBox.x + dragBox.width / 2,
    dragBox.y + dragBox.height / 2 - 8,
    { steps: 8 },
  );
  await expect(page.getByTestId("magic-board-drag-overlay")).toBeVisible();
  await page.mouse.move(
    targetBox.x + targetBox.width / 2,
    targetBox.y + targetBox.height / 2,
    { steps: 20 },
  );
  await page.mouse.up();

  await expect
    .poll(() =>
      page
        .locator("[data-board-kind]")
        .evaluateAll((cards) =>
          cards.map((card) => card.getAttribute("data-testid")),
        ),
    )
    .toEqual([
      "magic-board-card-fresh-signal-updated-1",
      "magic-board-card-this-week-at-sweet-works-2",
      "magic-board-card-start-here-3",
      "magic-board-card-help-wanted-4",
      "magic-board-card-finished-example-5",
      "magic-board-card-next-pilot-action-6",
    ]);

  const storedCanvas = await page.evaluate(async (channelId) => {
    const tauriWindow = window as Window & {
      __TAURI_INTERNALS__?: {
        invoke: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
    };
    const invoke = tauriWindow.__TAURI_INTERNALS__?.invoke;
    if (!invoke) {
      throw new Error("Tauri invoke bridge is unavailable.");
    }
    return invoke("get_canvas", { channelId }) as Promise<{
      content: string;
    }>;
  }, DISPATCH_CHANNEL_ID);
  expect(storedCanvas.content.indexOf("## Fresh signal updated")).toBeLessThan(
    storedCanvas.content.indexOf("## This week at Sweet Works"),
  );

  await page.getByTestId("channel-view-stream").click();
  await page.getByTestId("channel-view-board").click();
  await expect(
    page.getByTestId("magic-board-card-fresh-signal-updated-1"),
  ).toBeVisible();
});

test("board mutation controls stay hidden from non-stewards", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("home-inbox")).toBeVisible();
  await page.evaluate(async (channelId) => {
    const tauriWindow = window as Window & {
      __TAURI_INTERNALS__?: {
        invoke: (command: string) => Promise<unknown>;
      };
    };
    const identity = (await tauriWindow.__TAURI_INTERNALS__?.invoke(
      "get_identity",
    )) as { pubkey: string } | undefined;
    if (!identity) {
      throw new Error("Mock identity is unavailable.");
    }
    window.__BUZZ_E2E_MUTATE_CHANNEL__?.({
      channelId,
      name: "Dispatch",
      removeMemberPubkey: identity.pubkey,
    });
    await window.__BUZZ_E2E_INVALIDATE_CHANNELS__?.();
    window.location.hash = `#/channels/${channelId}`;
  }, DISPATCH_CHANNEL_ID);

  await expect(page.getByTestId("channel-magic-board")).toBeVisible();
  await expect(page.getByTestId("magic-board-create-card")).toHaveCount(0);
  await expect(page.locator("[data-testid^='magic-board-edit-']")).toHaveCount(
    0,
  );
  await expect(page.locator("[data-testid^='magic-board-drag-']")).toHaveCount(
    0,
  );
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
