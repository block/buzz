import { expect, test } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const REACTION_TARGET_CONTENT = "React to me with a custom emoji";
const REACTION_TARGET_EVENT_ID = "d".repeat(64);
const BOB_PUBKEY =
  "bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260";
const MAX_REACTION_NAME =
  "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijkl";
const MAX_REACTION_AVATAR_URL =
  'data:image/svg+xml,%3Csvg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"%3E%3Crect width="16" height="16" rx="4" fill="%23e5484d"/%3E%3C/svg%3E';
const SHORT_REACTION_AVATAR_URL =
  'data:image/svg+xml,%3Csvg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"%3E%3Crect width="16" height="16" rx="4" fill="%2300a36c"/%3E%3C/svg%3E';
const UNRESOLVED_SHORTCODE = ":missing_reaction:";
const LONG_LITERAL_REACTION =
  "this-is-a-deliberately-long-literal-reaction-that-must-truncate-without-moving-or-overlapping-the-count";
const SCREENSHOT_DIR =
  process.env.REACTION_POPOVER_SCREENSHOT_DIR ??
  "test-results/reaction-popover-screenshots";

function reactionTargetRow(page: import("@playwright/test").Page) {
  return page
    .getByTestId("message-row")
    .filter({ hasText: REACTION_TARGET_CONTENT })
    .last();
}

async function waitForImage(
  image: import("@playwright/test").Locator,
): Promise<void> {
  await expect(image).toBeVisible();
  await expect
    .poll(() =>
      image.evaluate(
        (element) =>
          element instanceof HTMLImageElement &&
          element.complete &&
          element.naturalWidth > 0,
      ),
    )
    .toBe(true);
}

async function capturePopover(
  page: import("@playwright/test").Page,
  popover: import("@playwright/test").Locator,
  filename: string,
): Promise<void> {
  fs.mkdirSync(SCREENSHOT_DIR, { recursive: true });
  await waitForAnimations(page);
  await popover.screenshot({
    animations: "disabled",
    path: path.join(SCREENSHOT_DIR, filename),
  });
}

async function emitReaction(
  page: import("@playwright/test").Page,
  content: string,
  pubkey: string,
): Promise<void> {
  await page.evaluate(
    ({ content, pubkey, targetId }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content,
        extraTags: [["e", targetId]],
        kind: 7,
        pubkey,
      });
    },
    { content, pubkey, targetId: REACTION_TARGET_EVENT_ID },
  );
}

async function expectFallbackPill(
  page: import("@playwright/test").Page,
  reaction: string,
  visibleText: string,
): Promise<void> {
  const pill = reactionTargetRow(page).getByRole("button", {
    name: `Toggle ${reaction} reaction`,
  });
  await expect(pill).toBeVisible();
  await expect(pill).toHaveAttribute("title", reaction);
  await expect(pill.locator("img")).toHaveCount(0);

  const glyph = pill.locator("span[title]");
  await expect(glyph).toHaveText(visibleText);
  const [pillRect, glyphRect, countRect] = await Promise.all([
    pill.boundingBox(),
    glyph.boundingBox(),
    pill.locator(".buzz-animated-count").boundingBox(),
  ]);
  expect(pillRect && glyphRect && countRect).toBeTruthy();
  if (!pillRect || !glyphRect || !countRect) return;
  expect(glyphRect.x + glyphRect.width).toBeLessThanOrEqual(countRect.x);
  expect(
    glyphRect.x >= pillRect.x &&
      countRect.x + countRect.width <= pillRect.x + pillRect.width,
  ).toBeTruthy();
}

async function expectFallbackPopover(
  page: import("@playwright/test").Page,
  reaction: string,
  visibleText: string,
  screenshotName: string,
): Promise<void> {
  const pill = reactionTargetRow(page).getByRole("button", {
    name: `Toggle ${reaction} reaction`,
  });
  await pill.hover();

  const popover = page.locator("[data-radix-popper-content-wrapper]").filter({
    has: page
      .getByTestId("reaction-popover-name")
      .filter({ hasText: reaction }),
  });
  await expect(popover).toBeVisible();
  await expect(popover.getByTestId("reaction-popover-name")).toHaveText(
    reaction,
  );

  const container = popover.getByTestId("reaction-popover-glyph-container");
  const glyph = container.locator("span[title]");
  await expect(glyph).toHaveText(visibleText);
  await expect(glyph).toHaveAttribute("title", reaction);
  const [containerRect, glyphRect] = await Promise.all([
    container.boundingBox(),
    glyph.boundingBox(),
  ]);
  expect(containerRect && glyphRect).toBeTruthy();
  if (containerRect && glyphRect) {
    expect(glyphRect.x).toBeGreaterThanOrEqual(containerRect.x);
    expect(glyphRect.x + glyphRect.width).toBeLessThanOrEqual(
      containerRect.x + containerRect.width,
    );
    expect(glyphRect.y).toBeGreaterThanOrEqual(containerRect.y);
    expect(glyphRect.y + glyphRect.height).toBeLessThanOrEqual(
      containerRect.y + containerRect.height,
    );
  }
  await expect(container).toHaveCSS("overflow", "hidden");
  await capturePopover(page, popover, screenshotName);
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: BOB_PUBKEY,
        displayName: "bob",
        avatarUrl: SHORT_REACTION_AVATAR_URL,
      },
    ],
  });
});

test("reaction popover resolves a reactor with no authored message in the window", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.waitForFunction(
    () =>
      window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
        channelName: "general",
        kind: 7,
      }) === true,
  );

  await page.evaluate(
    ({ pubkey, targetId, avatarUrl }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: ":react:",
        extraTags: [
          ["e", targetId],
          ["emoji", "react", avatarUrl],
        ],
        kind: 7,
        pubkey,
      });
    },
    {
      avatarUrl: SHORT_REACTION_AVATAR_URL,
      pubkey: BOB_PUBKEY,
      targetId: REACTION_TARGET_EVENT_ID,
    },
  );

  const row = reactionTargetRow(page);
  const pill = row.getByRole("button", { name: "Toggle :react: reaction" });
  await expect(pill).toBeVisible();
  await pill.hover();
  await expect(page.getByText("bob reacted with")).toBeVisible();
  const popover = page
    .locator("[data-radix-popper-content-wrapper]")
    .filter({ hasText: "bob reacted with" });
  const avatar = popover.locator("img");
  await expect(avatar).toHaveAttribute("src", SHORT_REACTION_AVATAR_URL);
  await waitForImage(avatar);
  await capturePopover(page, popover, "short-name-after.png");
});

test("maximum-length reaction name wraps inside a fixed-width popover", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.waitForFunction(
    () =>
      window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
        channelName: "general",
        kind: 7,
      }) === true,
  );

  const reaction = `:${MAX_REACTION_NAME}:`;
  await page.evaluate(
    ({ content, targetId, avatarUrl }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content,
        extraTags: [
          ["e", targetId],
          ["emoji", content.slice(1, -1), avatarUrl],
        ],
        kind: 7,
      });
    },
    {
      avatarUrl: MAX_REACTION_AVATAR_URL,
      content: reaction,
      targetId: REACTION_TARGET_EVENT_ID,
    },
  );

  const pill = reactionTargetRow(page).getByRole("button", {
    name: `Toggle ${reaction} reaction`,
  });
  await expect(pill).toBeVisible();
  await pill.focus();

  const popover = page
    .locator("[data-radix-popper-content-wrapper]")
    .filter({ hasText: reaction });
  await expect(popover).toBeVisible();
  await expect(popover).toHaveCSS("width", "288px");
  const reactionName = popover.getByTestId("reaction-popover-name");
  await expect(reactionName).toHaveCSS("word-break", "break-all");
  await expect(reactionName).toHaveText(reaction);
  const avatar = popover.locator("img");
  await expect(avatar).toHaveAttribute("src", MAX_REACTION_AVATAR_URL);
  await waitForImage(avatar);
  await capturePopover(page, popover, "max-length-after.png");
});

test("literal fallback reactions do not overlap their counts", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.waitForFunction(
    () =>
      window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
        channelName: "general",
        kind: 7,
      }) === true,
  );

  for (const reaction of [
    UNRESOLVED_SHORTCODE,
    "ship it",
    LONG_LITERAL_REACTION,
  ]) {
    await emitReaction(page, reaction, BOB_PUBKEY);
    await emitReaction(page, reaction, "c".repeat(64));
  }

  await expectFallbackPill(page, UNRESOLVED_SHORTCODE, "missing_reaction");
  await expectFallbackPill(page, "ship it", "ship it");
  const longPill = reactionTargetRow(page).getByRole("button", {
    name: `Toggle ${LONG_LITERAL_REACTION} reaction`,
  });
  const longGlyph = longPill.locator("span[title]");
  await expectFallbackPill(page, LONG_LITERAL_REACTION, LONG_LITERAL_REACTION);
  await expect(longGlyph).toHaveCSS("max-width", "128px");
  await expect(longGlyph).toHaveCSS("text-align", "left");
  await expect
    .poll(() =>
      longGlyph.evaluate(
        (element) => element.scrollWidth > element.clientWidth,
      ),
    )
    .toBe(true);

  await expectFallbackPopover(
    page,
    UNRESOLVED_SHORTCODE,
    "missing_reaction",
    "unresolved-shortcode-after.png",
  );
  await expectFallbackPopover(
    page,
    "ship it",
    "ship it",
    "literal-text-after.png",
  );
  await expectFallbackPopover(
    page,
    LONG_LITERAL_REACTION,
    LONG_LITERAL_REACTION,
    "long-literal-after.png",
  );

  await reactionTargetRow(page).screenshot({
    animations: "disabled",
    path: "test-results/reaction-text-fallback.png",
  });
});
