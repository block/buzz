import { expect, test, type Locator, type Page } from "@playwright/test";

import { KIND_HUDDLE_STARTED } from "../../src/shared/constants/kinds";
import { installMockBridge } from "../helpers/bridge";

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

async function latestClipboardWrite(page: Page) {
  return page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).findLast(
      ({ command }) => command === "copy_text_to_clipboard",
    ),
  );
}

async function expectCopyLinkUnavailable(row: Locator, messageId: string) {
  await row.hover();
  await expect(row.getByTestId(`copy-link-message-${messageId}`)).toHaveCount(
    0,
  );

  const moreActions = row.getByTestId(`more-actions-${messageId}`);
  if (await moreActions.count()) {
    await moreActions.click({ force: true });
    await expect(
      row.page().getByTestId(`copy-message-link-${messageId}`),
    ).toHaveCount(0);
    await row.page().keyboard.press("Escape");
  }
}

test.beforeEach(async ({ page }) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: "http://127.0.0.1:4173",
  });
  await installMockBridge(page);
});

test("message action rail copies the same canonical thread link as More", async ({
  page,
}) => {
  await page.setViewportSize({ width: 900, height: 700 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect
    .poll(() =>
      page.evaluate(
        () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
      ),
    )
    .toBe(true);

  const { replyId, rootId } = await page.evaluate(() => {
    const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    if (!emit) throw new Error("Mock message emitter is unavailable.");
    const root = emit({
      channelName: "general",
      content: "Copy-link regression root",
      id: "a".repeat(64),
    });
    const reply = emit({
      channelName: "general",
      content: "Copy-link regression reply",
      id: "b".repeat(64),
      parentEventId: root.id,
    });
    return { replyId: reply.id, rootId: root.id };
  });

  await page
    .locator(
      `[data-testid="message-thread-summary"][data-thread-head-id="${rootId}"]`,
    )
    .click();
  const threadPanel = page.getByTestId("message-thread-panel");
  const replyRow = threadPanel.locator(`[data-message-id="${replyId}"]`);
  await expect(replyRow).toContainText("Copy-link regression reply");
  await replyRow.hover();

  const actionBar = replyRow.getByTestId(`message-action-bar-${replyId}`);
  const orderedActionNames = await actionBar
    .getByRole("button")
    .evaluateAll((buttons) =>
      buttons.map((button) => button.getAttribute("aria-label")),
    );
  expect(orderedActionNames).toEqual([
    "React with :+1:",
    "React with :heart:",
    "React with :joy:",
    "Open reactions",
    "Reply",
    "Copy link",
    "More actions",
  ]);

  const pickerButton = actionBar.getByRole("button", {
    name: "Open reactions",
  });
  await expect(pickerButton.locator("svg")).toHaveClass(/lucide-smile-plus/);

  const divider = actionBar.getByTestId("message-action-divider");
  await expect(divider).toBeVisible();
  const [pickerBox, dividerBox, replyBox] = await Promise.all([
    actionBar.getByRole("button", { name: "Open reactions" }).boundingBox(),
    divider.boundingBox(),
    actionBar.getByRole("button", { name: "Reply" }).boundingBox(),
  ]);
  expect(pickerBox).not.toBeNull();
  expect(dividerBox).not.toBeNull();
  expect(replyBox).not.toBeNull();
  if (!pickerBox || !dividerBox || !replyBox) {
    throw new Error("Message action order bounds missing.");
  }
  expect(pickerBox.x + pickerBox.width).toBeLessThan(dividerBox.x);
  expect(dividerBox.x + dividerBox.width).toBeLessThan(replyBox.x);

  const copyLink = actionBar.getByTestId(`copy-link-message-${replyId}`);
  await expect(copyLink).toHaveAccessibleName("Copy link");
  await copyLink.hover();
  await expect(page.getByRole("tooltip", { name: "Copy link" })).toBeVisible();

  const expectedLink = `buzz://message?channel=${GENERAL_CHANNEL_ID}&id=${replyId}&thread=${rootId}`;
  await copyLink.click();
  await expect
    .poll(async () => (await latestClipboardWrite(page))?.payload.text)
    .toBe(expectedLink);
  await expect(
    page.locator("[data-sonner-toast]").filter({
      hasText: "Link copied to clipboard",
    }),
  ).toBeVisible();

  await actionBar.getByTestId(`more-actions-${replyId}`).click();
  await page.getByTestId(`copy-message-link-${replyId}`).click();
  await expect
    .poll(async () => {
      const writes = (await page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
          ({ command }) => command === "copy_text_to_clipboard",
        ),
      )) as Array<{ payload: unknown }>;
      return writes.map(({ payload }) => payload);
    })
    .toEqual([{ text: expectedLink }, { text: expectedLink }]);

  const [barBox, panelBox] = await Promise.all([
    actionBar.boundingBox(),
    threadPanel.boundingBox(),
  ]);
  expect(barBox).not.toBeNull();
  expect(panelBox).not.toBeNull();
  if (!barBox || !panelBox) throw new Error("Message action bounds missing.");
  expect(barBox.x).toBeGreaterThanOrEqual(panelBox.x);
  expect(barBox.x + barBox.width).toBeLessThanOrEqual(
    panelBox.x + panelBox.width,
  );
});

test("pending and huddle rows omit both copy-link surfaces", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect
    .poll(() =>
      page.evaluate(
        () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
      ),
    )
    .toBe(true);

  const { huddleId, pendingId } = await page.evaluate((huddleKind) => {
    const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    if (!emit) throw new Error("Mock message emitter is unavailable.");
    const pending = emit({
      channelName: "general",
      content: "Pending copy-link regression",
      id: "c".repeat(64),
      pending: true,
    });
    const huddle = emit({
      channelName: "general",
      content: JSON.stringify({
        ephemeral_channel_id: "10000000-0000-4000-8000-000000000001",
      }),
      id: "d".repeat(64),
      kind: huddleKind,
    });
    return { huddleId: huddle.id, pendingId: pending.id };
  }, KIND_HUDDLE_STARTED);

  await expectCopyLinkUnavailable(
    page.locator(`[data-message-id="${pendingId}"]`),
    pendingId,
  );
  await expectCopyLinkUnavailable(
    page.locator(`[data-message-id="${huddleId}"]`),
    huddleId,
  );
});
