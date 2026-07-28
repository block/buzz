import type { Page } from "@playwright/test";

export const SHOW_JOIN_LEAVE_STORAGE_KEY = "buzz:show-join-leave-messages";

/**
 * Enable the device-local "Show join and leave messages" preference before
 * the app boots. Production hides membership system rows by default, so any
 * test asserting on joined/added/left/removed timeline rows must opt in.
 * Call before `page.goto` — React reads the preference on mount.
 */
export async function enableJoinLeaveMessages(page: Page) {
  await page.addInitScript((key) => {
    window.localStorage.setItem(key, "1");
  }, SHOW_JOIN_LEAVE_STORAGE_KEY);
}
