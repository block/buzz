import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

/**
 * Composer tooltips set disableHoverableContent, so they must dismiss the
 * instant the cursor leaves the trigger — including when it slides onto the
 * tooltip popup itself (Radix's default hoverable-content behavior would
 * keep it open, camping it over the message editor).
 */

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

/** Hover the trigger through the shared dwell, then slide the cursor onto the
 * tooltip popup and assert the tooltip dismisses instead of persisting. */
async function expectTooltipDismissesOnLeave(
  page: import("@playwright/test").Page,
  trigger: import("@playwright/test").Locator,
  tooltipName: string,
) {
  await trigger.hover();

  const tip = page.getByRole("tooltip", { name: tooltipName });
  await page.waitForTimeout(400);
  await expect(tip).toHaveCount(0);
  await expect(tip).toBeVisible({ timeout: 1_000 });

  // Slide off the trigger onto the tooltip popup.
  const box = await tip.boundingBox();
  if (!box) throw new Error("no tooltip box");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, {
    steps: 5,
  });

  await expect(tip).toBeHidden();
}

test("composer toolbar tooltip dismisses when cursor leaves the trigger", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await expectTooltipDismissesOnLeave(
    page,
    page.getByTestId("message-insert-mention"),
    "Mention someone",
  );
});

test("adjacent composer tooltips each require a fresh dwell", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-insert-mention").hover();
  await page.waitForTimeout(400);
  const mentionTooltip = page.getByRole("tooltip", { name: "Mention someone" });
  await expect(mentionTooltip).toHaveCount(0);
  await expect(mentionTooltip).toBeVisible({ timeout: 1_000 });

  await page.getByRole("button", { name: "Attach file" }).hover();
  await page.waitForTimeout(400);
  const attachTooltip = page.getByRole("tooltip", { name: "Attach file" });
  await expect(attachTooltip).toHaveCount(0);
  await expect(attachTooltip).toBeVisible({ timeout: 1_000 });
});

test("formatting sub-toolbar tooltip dismisses when cursor leaves the trigger", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Open the formatting sub-toolbar (Bold / Italic / lists / Quote …).
  await page.getByRole("button", { name: "Toggle formatting" }).first().click();

  const bold = page.getByRole("button", { name: "Bold" });
  await expect(bold).toBeVisible();
  // The formatting strip animates into place; wait for its delayed entrance to
  // settle so the pointer remains over the trigger for the full dwell.
  await page.waitForTimeout(300);

  // Tooltip text is "<label> (<shortcut>)" for items that carry a shortcut.
  await expectTooltipDismissesOnLeave(page, bold, "Bold (⌘B)");
});

test("emoji picker tooltip dismisses when cursor leaves the trigger", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // The emoji button wraps its tooltip around a nested PopoverTrigger, so
  // cover it separately from the plain toolbar buttons.
  await expectTooltipDismissesOnLeave(
    page,
    page.getByTestId("composer-emoji-button"),
    "Emoji",
  );
});
