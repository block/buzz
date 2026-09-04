import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

// Hold exit presence, not composer readiness: a closed menu can still receive a
// queued pointer-leave while its CSS exit animation is mounted.
const holdExit = `
  [role="menu"][data-state="closed"] { animation-play-state: paused !important; }
`;

test("closing message menu cannot reclaim editor focus on pointer leave", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.addStyleTag({ content: holdExit });
  const input = page.getByTestId("message-input");
  await input.fill("focus ownership");
  await page.getByTestId("send-message").click();
  const row = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .last();
  await expect(row).toContainText("focus ownership");
  await row.hover();
  await row.getByRole("button", { name: "More actions" }).click();
  const edit = page.getByRole("menuitem", { name: "Edit message" });
  const editElement = await edit.elementHandle();
  await edit.click();
  await expect(input).toHaveText("focus ownership");
  // A deliberate user click establishes focus independently of RAF timing.
  await input.click();
  await input.press("ControlOrMeta+End");
  await expect(input).toBeFocused();
  await page.keyboard.type(" edit");
  await expect(input).toHaveText("focus ownership edit");
  const closing = await editElement?.evaluate((element) => {
    const menu = element.closest('[role="menu"]');
    const state = menu?.getAttribute("data-state");
    const connected = element.isConnected;
    element.dispatchEvent(
      new PointerEvent("pointerout", {
        bubbles: true,
        pointerType: "mouse",
        relatedTarget: document.body,
      }),
    );
    return { state, connected };
  });
  expect(closing).toEqual({ state: "closed", connected: true });
  await expect(input).toBeFocused();
  await page.keyboard.type("ed");
  await expect(input).toHaveText("focus ownership edited");
  await page.getByTestId("send-message").click();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const call = window.__BUZZ_E2E_COMMAND_LOG__
          ?.filter((entry) => entry.command === "edit_message")
          .at(-1);
        return (call?.payload as { input?: { content: string } })?.input
          ?.content;
      }),
    )
    .toBe("focus ownership edited");
});

test("message menu Escape still restores its keyboard trigger", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const row = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .last();
  await row.hover();
  const trigger = row.getByRole("button", { name: "More actions" });
  await trigger.focus();
  await trigger.press("Enter");
  await expect(page.getByRole("menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(trigger).toBeFocused();
});
