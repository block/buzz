import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("composer tooltip is visible but click-through (pointer-events:none)", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Hover the "Mention someone" toolbar trigger to surface its tooltip.
  const trigger = page.getByTestId("message-insert-mention");
  await trigger.hover();

  // Radix renders the tooltip content in a portal as role=tooltip.
  const tip = page.getByRole("tooltip", { name: "Mention someone" });
  await expect(tip).toBeVisible();

  // The money check: computed pointer-events on the visible tooltip popup.
  const pe = await tip.evaluate((el) => getComputedStyle(el).pointerEvents);
  expect(pe).toBe("none");

  await page.screenshot({
    path: "test-results/tooltip-pe/composer-tooltip-visible.png",
    clip: { x: 0, y: 360, width: 900, height: 360 },
  });

  // Prove click-through: aim the click at the tooltip's own bounding box.
  // pointer-events:none means the click should fall through to whatever is
  // underneath rather than being swallowed by the popup.
  const box = await tip.boundingBox();
  if (!box) throw new Error("no tooltip box");
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);

  // After clicking through, the editor should be focusable/usable: type and
  // confirm the text lands in the ProseMirror editor under the toolbar.
  const editor = page.locator(".ProseMirror");
  await editor.click();
  await page.keyboard.type("clickthrough-ok");
  await expect(editor).toContainText("clickthrough-ok");
});

test("formatting sub-toolbar tooltip is visible but click-through (pointer-events:none)", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Open the formatting sub-toolbar (Bold / Italic / lists / Quote …).
  await page.getByRole("button", { name: "Toggle formatting" }).first().click();

  // Hover a formatting icon button to surface its label tooltip.
  const bold = page.getByRole("button", { name: "Bold" });
  await expect(bold).toBeVisible();
  await bold.hover();

  // Tooltip text is "<label> (<shortcut>)" for items that carry a shortcut.
  const tip = page.getByRole("tooltip", { name: "Bold (⌘B)" });
  await expect(tip).toBeVisible();

  // Money check: the sub-toolbar tooltip popup must be click-through too.
  const pe = await tip.evaluate((el) => getComputedStyle(el).pointerEvents);
  expect(pe).toBe("none");
});
