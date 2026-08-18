import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

/**
 * Regression suite for the "app freezes after setting a reminder" bug.
 *
 * The app used to bundle two copies of @radix-ui/react-dismissable-layer
 * (react-menu resolved 1.1.11 while react-dialog resolved 1.1.15). Each copy
 * keeps its own module-level `originalBodyPointerEvents`, so when a *modal*
 * menu opened a *modal* dialog in the same React commit, the dialog's copy
 * saved the menu's `pointer-events: none` as the "original" body style and
 * restored it when the dialog closed — leaving `pointer-events: none` stuck
 * on <body> and deadening the whole app to the mouse.
 *
 * The inbox row context menu hit this: its "Remind me later" item opens
 * RemindMeLaterDialog synchronously. Fixed by deduplicating the layer via
 * the pnpm override in pnpm-workspace.yaml — a single copy coordinates
 * nested modal layers correctly, so the menu keeps its default modal
 * behavior.
 *
 * Each test drives one reminder entry point through a full set-reminder
 * cycle, then asserts <body> pointer-events is not stuck and that a real
 * click still lands. The inbox right-click test fails if the dismissable
 * layer ever duplicates again; the message-menu tests pin the already-safe
 * paths.
 */

const GENERAL_MESSAGE_ROW = "message-row";
// The default mock feed's mention row (from alice, in #general) has a
// channel target, so its "Remind me later" context-menu item is enabled.
const INBOX_MENTION_ROW = "home-inbox-item-mock-feed-mention";

/**
 * Assert the app survived the menu → dialog → close cycle.
 *
 * The stuck state is inline `pointer-events: none` left on <body>. The buggy
 * restore runs when the dialog's dismissable layer unmounts — after its exit
 * animation — so first wait for the dialog to leave the DOM, then check the
 * body style, then prove clicks still land by navigating via the sidebar.
 */
async function expectAppAliveAfterDialogClose(
  page: import("@playwright/test").Page,
) {
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await waitForAnimations(page);
  expect(await page.evaluate(() => document.body.style.pointerEvents)).not.toBe(
    "none",
  );
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
}

/** Open the Remind-me-later dialog from a message row's More-actions menu. */
async function openReminderDialogFromMessageMenu(
  page: import("@playwright/test").Page,
) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const messageRow = page.getByTestId(GENERAL_MESSAGE_ROW).first();
  await messageRow.hover();
  await messageRow.getByRole("button", { name: "More actions" }).click();

  const remindItem = page.getByRole("menuitem", { name: "Remind me later" });
  await expect(remindItem).toBeVisible();
  await waitForAnimations(page);
  await remindItem.click();

  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await waitForAnimations(page);
  return dialog;
}

async function expectReminderSetToast(page: import("@playwright/test").Page) {
  await expect(
    page.locator("[data-sonner-toast]").filter({ hasText: "Reminder set" }),
  ).toBeVisible();
}

test.describe("reminder set → app stays clickable", () => {
  test.beforeEach(async ({ page }) => {
    await installMockBridge(page);
  });

  test("01 — message menu → time preset", async ({ page }) => {
    const dialog = await openReminderDialogFromMessageMenu(page);

    await dialog.getByRole("button", { name: "In 30 minutes" }).click();

    await expectReminderSetToast(page);
    await expectAppAliveAfterDialogClose(page);
  });

  test("02 — message menu → custom time via Set reminder footer button", async ({
    page,
  }) => {
    const dialog = await openReminderDialogFromMessageMenu(page);

    // The custom timestamp must be strictly in the future or the Set
    // reminder button stays disabled — pick tomorrow at 09:00 local time.
    const tomorrow = new Date(Date.now() + 24 * 60 * 60 * 1000);
    const date = [
      tomorrow.getFullYear(),
      String(tomorrow.getMonth() + 1).padStart(2, "0"),
      String(tomorrow.getDate()).padStart(2, "0"),
    ].join("-");
    await dialog.getByLabel("Reminder date").fill(date);
    await dialog.getByLabel("Reminder time").fill("09:00");
    await dialog.getByRole("button", { name: "Set reminder" }).click();

    await expectReminderSetToast(page);
    await expectAppAliveAfterDialogClose(page);
  });

  // The regression path: only the inbox right-click stacks a modal
  // ContextMenu under the modal dialog. Fails if two copies of the
  // dismissable layer are ever bundled again — body pointer-events stays
  // "none" after the dialog closes and the sidebar click below times out.
  test("03 — inbox row right-click → Remind me later", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("home-inbox")).toBeVisible();

    const row = page.getByTestId(INBOX_MENTION_ROW);
    await expect(row).toBeVisible();
    await row.click({ button: "right" });

    const remindItem = page.getByRole("menuitem", { name: "Remind me later" });
    await expect(remindItem).toBeVisible();
    await waitForAnimations(page);
    await remindItem.click();

    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await waitForAnimations(page);
    await dialog.getByRole("button", { name: "In 30 minutes" }).click();

    await expectReminderSetToast(page);
    await expectAppAliveAfterDialogClose(page);
  });
});
