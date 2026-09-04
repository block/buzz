import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHORTCODE = "buzz";
const STATUS_TEXT = "testing custom status";
const MOCK_IDENTITY_PUBKEY = "deadbeef".repeat(8);

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
  kind?: number,
) {
  await expect
    .poll(() =>
      page.evaluate(
        ({ currentChannelName, currentKind }) =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: currentChannelName,
            kind: currentKind,
          }) ?? false,
        { currentChannelName: channelName, currentKind: kind },
      ),
    )
    .toBe(true);
}

async function openProfilePopover(page: import("@playwright/test").Page) {
  await page.getByTestId("open-settings").click();
  await expect(page.getByTestId("profile-popover")).toBeVisible();
}

async function waitForMockGlobalKindSubscription(
  page: import("@playwright/test").Page,
  kind: number,
) {
  await expect
    .poll(() =>
      page.evaluate(
        (currentKind) =>
          window.__BUZZ_E2E_HAS_MOCK_GLOBAL_KIND_SUBSCRIPTION__?.(
            currentKind,
          ) ?? false,
        kind,
      ),
    )
    .toBe(true);
}

async function seedMockStatus(
  page: import("@playwright/test").Page,
  input: {
    text: string;
    emoji?: string;
    expiresAt?: number;
    createdAt?: number;
  },
) {
  await waitForMockGlobalKindSubscription(page, 30315);
  await page.evaluate((status) => {
    window.__BUZZ_E2E_SET_MOCK_USER_STATUS__?.(status);
  }, input);
  await openProfilePopover(page);
  await expect(page.getByTestId("profile-popover-set-status")).toContainText(
    input.text,
  );
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, { relaySelf: MOCK_IDENTITY_PUBKEY });
  const PNG = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAGUlEQVR4nGMwuBPxnxLMMGrAqAGjBgwXAwBwOGMf1PPhVwAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.route("https://example.com/e2e/**", (route) =>
    route.fulfill({ contentType: "image/png", body: PNG }),
  );
});

test("set status dialog uses the desktop modal with shared status choices", async ({
  page,
}) => {
  await page.goto("/");
  await openProfilePopover(page);
  await page.getByTestId("profile-popover-set-status").click();

  const dialog = page.getByTestId("set-status-dialog");
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole("heading", { name: "Set a status" }),
  ).toBeVisible();
  await expect(dialog.getByLabel("Save status")).toBeDisabled();
  const emojiButton = dialog.getByLabel("Choose a status emoji");
  await expect(dialog.getByLabel("Remove status emoji")).toHaveCount(0);
  await expect
    .poll(() =>
      emojiButton.locator("svg").evaluate((element) => ({
        height: getComputedStyle(element).height,
        width: getComputedStyle(element).width,
      })),
    )
    .toEqual({ height: "20px", width: "20px" });
  await dialog.getByTestId("set-status-input").fill("Heads down");
  await expect(emojiButton.getByText("💬", { exact: true })).toBeVisible();
  await expect(emojiButton.locator("svg")).toHaveCount(0);
  await dialog.getByTestId("set-status-input").fill("");
  await expect(dialog.getByText("Duration", { exact: true })).toHaveCount(2);
  await expect(
    dialog.getByText("Quick statuses", { exact: true }),
  ).toBeVisible();

  for (const quickStatus of [
    "In a meeting",
    "Commuting",
    "Out sick",
    "Vacationing",
    "Working remotely",
  ]) {
    await expect(dialog.getByText(quickStatus, { exact: true })).toBeVisible();
  }

  await dialog.getByTestId("set-status-duration").click();
  for (const duration of [
    "1 hour",
    "8 hours",
    "Today",
    "This week",
    "Custom",
  ]) {
    await expect(page.getByRole("menuitem", { name: duration })).toBeVisible();
  }
  await waitForAnimations(page);
  await page.screenshot({
    clip: { height: 700, width: 600, x: 340, y: 10 },
    path: "test-results/profile-status/04-status-duration-options.png",
  });
  await page.getByRole("menuitem", { name: "8 hours" }).click();

  await dialog.getByTestId("set-status-preset-working-remotely").click();
  await expect(dialog.getByTestId("set-status-input")).toHaveValue(
    "Working remotely",
  );
  await expect(dialog.getByLabel("Save status")).toBeEnabled();
  await dialog.getByTestId("set-status-save").click();

  await openProfilePopover(page);
  await page.getByTestId("profile-popover-set-status").click();
  await expect(dialog.getByText("Quick statuses", { exact: true })).toHaveCount(
    0,
  );
  await expect(dialog.getByLabel("Save status")).toBeDisabled();
  await expect(page.getByTestId("set-status-duration")).toContainText(
    "8 hours",
  );

  await dialog.getByTestId("set-status-input").fill("Working remotely today");
  await expect(dialog.getByLabel("Save status")).toBeEnabled();
  await dialog.getByTestId("set-status-input").fill("Working remotely");
  await expect(dialog.getByLabel("Save status")).toBeDisabled();

  await page.getByTestId("set-status-duration").click();
  await page.getByRole("menuitem", { name: "Custom" }).click();
  await expect(dialog.getByText("Until", { exact: true })).toBeVisible();
  const expirationDate = dialog.getByLabel("Status expiration date");
  await expect(expirationDate).toBeVisible();
  await expirationDate.click();
  await expect(page.locator('[data-slot="calendar"]')).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({
    clip: { height: 700, width: 600, x: 340, y: 10 },
    path: "test-results/profile-status/05-custom-status-calendar.png",
  });
  await page.keyboard.press("Escape");
  const expirationTime = dialog.getByLabel("Status expiration time");
  await expect(expirationTime).toBeVisible();
  await expirationTime.click();
  const timeMenu = page.getByTestId("status-expiration-time-menu");
  await expect(timeMenu.getByRole("menuitem")).toHaveCount(48);
  await expect
    .poll(() =>
      timeMenu.evaluate(
        (element) =>
          element.clientHeight <= 368 &&
          element.scrollHeight > element.clientHeight,
      ),
    )
    .toBe(true);
  await waitForAnimations(page);
  await page.screenshot({
    clip: { height: 700, width: 600, x: 340, y: 10 },
    path: "test-results/profile-status/06-custom-status-time.png",
  });
});

test("keeps an open status draft when the saved status expires", async ({
  page,
}) => {
  await page.goto("/");
  const nowSeconds = Math.floor(Date.now() / 1_000);
  await seedMockStatus(page, {
    text: "Original draft",
    emoji: "📝",
    expiresAt: nowSeconds + 2,
    createdAt: nowSeconds,
  });
  await page.getByTestId("profile-popover-set-status").click();
  const dialog = page.getByTestId("set-status-dialog");
  await dialog.getByTestId("set-status-input").fill("Unsaved draft");
  await expect(page.getByTestId("sidebar-profile-user-status")).toHaveCount(0, {
    timeout: 5_000,
  });

  await expect(dialog.getByTestId("set-status-input")).toHaveValue(
    "Unsaved draft",
  );
  await expect(dialog.getByRole("alert")).toContainText(
    "Choose a duration in the future.",
  );
  await expect(dialog.getByLabel("Save status")).toBeDisabled();
  await page.getByTestId("set-status-duration").click();
  await page.getByRole("menuitem", { name: "This week" }).click();
  await expect(dialog.getByLabel("Save status")).toBeEnabled();
  await expect(dialog.getByText("Quick statuses", { exact: true })).toHaveCount(
    0,
  );
  await expect(dialog.getByTestId("set-status-clear")).toBeVisible();
});

test("new statuses default to expiring at local midnight", async ({ page }) => {
  await page.goto("/");
  await openProfilePopover(page);
  await page.getByTestId("profile-popover-set-status").click();
  const dialog = page.getByTestId("set-status-dialog");
  await dialog.getByTestId("set-status-preset-working-remotely").click();
  await dialog.getByTestId("set-status-save").click();

  const publishedExpiration = await page.evaluate(
    () =>
      window.__BUZZ_E2E_SIGNED_EVENTS__
        ?.filter((event) => event.kind === 30315)
        .at(-1)
        ?.tags.find((tag) => tag[0] === "expiration")?.[1],
  );
  const expectedMidnight = await page.evaluate(() => {
    const midnight = new Date();
    midnight.setHours(24, 0, 0, 0);
    return Math.floor(midnight.getTime() / 1_000);
  });
  expect(Number(publishedExpiration)).toBe(expectedMidnight);
});

test("can apply Today to an existing status without an expiration", async ({
  page,
}) => {
  await page.goto("/");
  const nowSeconds = Math.floor(Date.now() / 1_000);
  await seedMockStatus(page, {
    text: "Indefinite status",
    emoji: "♾️",
    createdAt: nowSeconds,
  });
  await page.getByTestId("profile-popover-set-status").click();
  const dialog = page.getByTestId("set-status-dialog");
  await expect(dialog.getByLabel("Save status")).toBeDisabled();

  await page.getByTestId("set-status-duration").click();
  await page.getByRole("menuitem", { name: "Today" }).click();
  await expect(dialog.getByLabel("Save status")).toBeEnabled();
  await dialog.getByTestId("set-status-save").click();

  const expiration = await page.evaluate(
    () =>
      window.__BUZZ_E2E_SIGNED_EVENTS__
        ?.filter((event) => event.kind === 30315)
        .at(-1)
        ?.tags.find((tag) => tag[0] === "expiration")?.[1],
  );
  expect(Number(expiration)).toBeGreaterThan(nowSeconds);
});

test("applies Today when editing a legacy status without an expiration", async ({
  page,
}) => {
  await page.goto("/");
  const nowSeconds = Math.floor(Date.now() / 1_000);
  await seedMockStatus(page, {
    text: "Legacy indefinite status",
    emoji: "♾️",
    createdAt: nowSeconds,
  });
  await page.getByTestId("profile-popover-set-status").click();
  const dialog = page.getByTestId("set-status-dialog");
  await expect(page.getByTestId("set-status-duration")).toContainText("Today");
  await dialog.getByTestId("set-status-input").fill("Legacy status edited");
  await dialog.getByTestId("set-status-save").click();

  const expiration = await page.evaluate(
    () =>
      window.__BUZZ_E2E_SIGNED_EVENTS__
        ?.filter((event) => event.kind === 30315)
        .at(-1)
        ?.tags.find((tag) => tag[0] === "expiration")?.[1],
  );
  expect(Number(expiration)).toBeGreaterThan(nowSeconds);
});

test("preserves a minute-precision custom deadline when only text changes", async ({
  page,
}) => {
  await page.goto("/");
  const { createdAt, expiresAt } = await page.evaluate(() => {
    const now = new Date();
    const expiration = new Date(now.getTime() + 24 * 60 * 60 * 1_000);
    expiration.setHours(10, 10, 0, 0);
    return {
      createdAt: Math.floor(now.getTime() / 1_000),
      expiresAt: Math.floor(expiration.getTime() / 1_000),
    };
  });
  await seedMockStatus(page, {
    text: "Mobile deadline",
    emoji: "📱",
    createdAt,
    expiresAt,
  });
  await page.getByTestId("profile-popover-set-status").click();
  const dialog = page.getByTestId("set-status-dialog");
  await expect(dialog.getByLabel("Status expiration time")).toContainText(
    "10:10",
  );
  await dialog.getByTestId("set-status-input").fill("Edited desktop text");
  await dialog.getByTestId("set-status-save").click();

  const publishedExpiration = await page.evaluate(
    () =>
      window.__BUZZ_E2E_SIGNED_EVENTS__
        ?.filter((event) => event.kind === 30315)
        .at(-1)
        ?.tags.find((tag) => tag[0] === "expiration")?.[1],
  );
  expect(Number(publishedExpiration)).toBe(expiresAt);
});

test("profile popover renders a custom emoji status as an image", async ({
  page,
}) => {
  await page.goto("/");
  await openProfilePopover(page);

  await page.getByTestId("profile-popover-set-status").click();
  await expect(page.getByTestId("set-status-dialog")).toBeVisible();
  await page.getByLabel("Choose a status emoji").click();

  const picker = page.locator("em-emoji-picker");
  await picker.locator("input[type='search']").fill(SHORTCODE);
  await picker
    .getByRole("button", { name: `:${SHORTCODE}:` })
    .first()
    .click();
  await page.getByTestId("set-status-input").fill(STATUS_TEXT);
  await page.getByTestId("set-status-save").click();

  await openProfilePopover(page);

  const statusButton = page.getByTestId("profile-popover-set-status");
  await expect(statusButton).toContainText(STATUS_TEXT);
  await expect(statusButton.locator(`img[alt=":${SHORTCODE}:"]`)).toBeVisible();
  await expect(statusButton.locator("svg")).toHaveCount(0);
  await expect(statusButton.locator("[title]")).toHaveCount(0);
  await expect(statusButton).not.toContainText(`:${SHORTCODE}:`);
});

test("shows status and huddle indicators beside chat names with tooltips", async ({
  page,
}) => {
  await page.goto("/");
  await openProfilePopover(page);
  await page.getByTestId("profile-popover-set-status").click();
  await page.getByTestId("set-status-input").fill(STATUS_TEXT);
  await page.getByTestId("set-status-save").click();

  await openProfilePopover(page);
  const statusButton = page.getByTestId("profile-popover-set-status");
  await expect(statusButton.getByText("💬", { exact: true })).toBeVisible();
  await expect(statusButton.locator("svg")).toHaveCount(0);
  await expect(statusButton.locator("[title]")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("profile-popover")).not.toBeVisible();

  await page.getByTestId("channel-alice-tyler").click();
  await waitForMockLiveSubscription(page, "alice-tyler");
  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "alice-tyler",
      content: "Status indicator fixture",
      kind: 40002,
      pubkey,
    });
  }, MOCK_IDENTITY_PUBKEY);
  const row = page.getByTestId("message-row").filter({
    hasText: "Status indicator fixture",
  });
  const statusIndicator = row.getByTestId("user-status-indicator");
  await expect(statusIndicator).toBeVisible();
  await expect(statusIndicator).toHaveAttribute(
    "aria-label",
    `💬 ${STATUS_TEXT}`,
  );
  await expect
    .poll(() =>
      statusIndicator.evaluate((element) => getComputedStyle(element).fontSize),
    )
    .toBe("14px");
  await expect
    .poll(() =>
      statusIndicator.locator("[aria-hidden='true']").evaluate((element) => ({
        height: getComputedStyle(element).height,
        width: getComputedStyle(element).width,
      })),
    )
    .toEqual({ height: "14px", width: "14px" });
  await expect(statusIndicator.locator("[title]")).toHaveCount(0);
  await statusIndicator.hover();
  await expect(page.getByRole("tooltip")).toHaveText(STATUS_TEXT);

  await waitForMockLiveSubscription(page, "alice-tyler", 48100);
  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "alice-tyler",
      content: JSON.stringify({
        ephemeral_channel_id: "status-huddle",
        generation: "status-generation",
      }),
      id: "8".repeat(64),
      kind: 48100,
      pubkey,
    });
  }, MOCK_IDENTITY_PUBKEY);
  const huddleIndicator = row.getByTestId("user-huddle-indicator");
  await expect(huddleIndicator).toHaveCount(0);

  await waitForMockLiveSubscription(page, "alice-tyler", 48101);
  await page.evaluate((relayPubkey) => {
    const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    emit?.({
      channelName: "alice-tyler",
      content: JSON.stringify({
        admission_id: "status-admission",
        ephemeral_channel_id: "status-huddle",
        generation: "status-generation",
        roster_revision: 1,
      }),
      extraTags: [["p", relayPubkey]],
      id: "9".repeat(64),
      kind: 48101,
      pubkey: relayPubkey,
    });
    emit?.({
      channelName: "alice-tyler",
      content: JSON.stringify({
        ephemeral_channel_id: "status-huddle",
        generation: "status-generation",
      }),
      extraTags: [["d", "status-huddle"]],
      id: "a".repeat(64),
      kind: 48104,
      pubkey: relayPubkey,
    });
  }, MOCK_IDENTITY_PUBKEY);
  await expect(huddleIndicator).toBeVisible();
  await expect(huddleIndicator).toHaveAttribute("aria-label", "🎧 In a huddle");
  await expect
    .poll(() =>
      huddleIndicator.evaluate((element) => getComputedStyle(element).fontSize),
    )
    .toBe("14px");
  await huddleIndicator.hover();
  await expect(page.getByRole("tooltip")).toHaveText("In a huddle");
});
