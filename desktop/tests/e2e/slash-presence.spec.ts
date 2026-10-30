import { expect, test, type Locator, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openGeneral } from "../helpers/channelHeadReady";

const MOCK_IDENTITY_PUBKEY = "deadbeef".repeat(8);
const PREFERENCE_KEY = `buzz-presence-preference:${MOCK_IDENTITY_PUBKEY}`;

function mainComposer(page: Page) {
  return page
    .getByTestId("message-composer")
    .filter({ hasNot: page.getByTestId("edit-target") })
    .first();
}

async function openChannel(page: Page) {
  await installMockBridge(page, { relaySelf: MOCK_IDENTITY_PUBKEY });
  await page.goto("/");
  await openGeneral(page);
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  return mainComposer(page).getByTestId("message-input");
}

async function run(input: Locator, draft: string) {
  await input.click();
  await input.pressSequentially(draft);
  await input.press("Enter");
}

function presenceBadge(page: Page) {
  return page.getByTestId("self-presence-badge");
}

function storedPreference(page: Page) {
  return page.evaluate(
    (key) => window.localStorage.getItem(key),
    PREFERENCE_KEY,
  );
}

// Replaceable events from the same second are ordered by event id, so wait for
// the next second to make the later status win.
function waitForNextSecond(page: Page) {
  return page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        const start = Math.floor(Date.now() / 1_000);
        const timer = setInterval(() => {
          if (Math.floor(Date.now() / 1_000) > start) {
            clearInterval(timer);
            resolve();
          }
        }, 25);
      }),
  );
}

async function expectToast(page: Page, text: string) {
  await expect(
    page.locator("[data-sonner-toast]").filter({ hasText: text }),
  ).toBeVisible();
}

test("/away and /active set and clear the manual presence override", async ({
  page,
}) => {
  const input = await openChannel(page);
  await expect(presenceBadge(page)).toHaveAccessibleName("Online");

  await run(input, "/away");
  await expectToast(page, "You're away");
  await expect(presenceBadge(page)).toHaveAccessibleName("Away");
  await expect.poll(() => storedPreference(page)).toBe("away");
  await expect(input).toBeEmpty();

  await run(input, "/active");
  await expectToast(page, "You're active");
  await expect(presenceBadge(page)).toHaveAccessibleName("Online");
  await expect.poll(() => storedPreference(page)).toBe("auto");
  await expect(input).toBeEmpty();
});

test("/away and /active reject extra arguments and keep the draft", async ({
  page,
}) => {
  const input = await openChannel(page);

  await run(input, "/away now");
  await expectToast(page, "Usage: /away");
  await expect(input).toHaveText("/away now");
  await expect(presenceBadge(page)).toHaveAccessibleName("Online");
  expect(await storedPreference(page)).toBeNull();

  await input.press("ControlOrMeta+a");
  await input.press("Delete");
  await run(input, "/active soon");
  await expectToast(page, "Usage: /active");
  await expect(input).toHaveText("/active soon");
});

test("/status sets a status with an emoji, and /status clear removes it", async ({
  page,
}) => {
  const input = await openChannel(page);
  const status = page.getByTestId("sidebar-profile-user-status");
  await expect(status).toHaveCount(0);

  await run(input, "/status 🚀 shipping");
  await expectToast(page, "Status set");
  await expect(status).toContainText("shipping");
  await expect(status).toContainText("🚀");
  await expect(input).toBeEmpty();

  await waitForNextSecond(page);
  await run(input, "/status :tada: celebrating");
  await expect(status).toContainText("celebrating");
  await expect(status).toContainText("🎉");

  await waitForNextSecond(page);
  await run(input, "/status in a meeting");
  await expect(status).toContainText("in a meeting");
  await expect(status).not.toContainText("🎉");

  await waitForNextSecond(page);
  await run(input, "/status clear");
  await expectToast(page, "Status cleared");
  await expect(status).toHaveCount(0);
  await expect(input).toBeEmpty();
});

test("/status with no arguments opens the set-status dialog and clears the draft", async ({
  page,
}) => {
  const input = await openChannel(page);

  await run(input, "/status");
  const dialog = page.getByTestId("set-status-dialog");
  await expect(dialog).toBeVisible();
  await expect(input).toBeEmpty();

  await dialog.getByTestId("set-status-input").fill("from the dialog");
  await dialog.getByTestId("set-status-save").click();
  await expect(page.getByTestId("sidebar-profile-user-status")).toContainText(
    "from the dialog",
  );
});

test("a keyboard-only pass picks /away from the list and runs it", async ({
  page,
}) => {
  const input = await openChannel(page);
  const composer = mainComposer(page);
  const list = composer.getByTestId("slash-command-autocomplete");

  await input.click();
  await page.keyboard.type("/aw");
  await expect(list).toBeVisible();
  await expect(list.getByTestId("slash-command-away")).toHaveAccessibleName(
    /^\/away/,
  );
  await page.keyboard.press("Enter");
  await expect(list).toBeHidden();
  await expect(input).toHaveText("/away");
  await expect(presenceBadge(page)).toHaveAccessibleName("Online");

  await page.keyboard.press("Enter");
  await expect(presenceBadge(page)).toHaveAccessibleName("Away");
  await expect(input).toBeEmpty();

  await waitForNextSecond(page);
  await page.keyboard.type("/st");
  await expect(list.getByTestId("slash-command-status")).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(list).toBeHidden();
  await expect(input).toHaveText("/status");
  await page.keyboard.type(" 🚀 keyboard");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("sidebar-profile-user-status")).toContainText(
    "keyboard",
  );
});
