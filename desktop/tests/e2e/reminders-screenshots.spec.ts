import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/reminders";
const MOCK_PUBKEY = "deadbeef".repeat(8);

// The inbox filter dropdown lives in the home pane, not the chat view. Land on
// home and wait for the inbox before reaching for the filter trigger.
async function gotoInboxHome(page: import("@playwright/test").Page) {
  await page.goto("/");
  await expect(page.getByTestId("home-inbox")).toBeVisible();
}

// Reminders is reached by opening the inbox filter dropdown and selecting the
// "Reminders" option — there is no standalone nav entry or view-mode slider.
async function openRemindersFilter(page: import("@playwright/test").Page) {
  await page.getByTestId("inbox-filter-trigger").click();
  await page.getByRole("menuitemradio", { name: "Reminders" }).click();
}

// The reminders query mounts (for the badge) before tests seed events, so a
// bare seed lands behind its cached empty result. Invalidate after seeding to
// force the refetch that picks up the mock events.
async function seedReminders(
  page: import("@playwright/test").Page,
  events: unknown[],
) {
  await page.evaluate((seeded) => {
    window.__BUZZ_E2E_SEED_MOCK_REMINDERS__?.(
      seeded as Parameters<
        NonNullable<typeof window.__BUZZ_E2E_SEED_MOCK_REMINDERS__>
      >[0],
    );
    window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
      queryKey: ["reminders"],
    });
  }, events);
}

function mockReminderEvent(opts: {
  id: string;
  dTag: string;
  content: string;
  notBefore: number;
  createdAt?: number;
}) {
  return {
    id: opts.id,
    pubkey: MOCK_PUBKEY,
    created_at: opts.createdAt ?? Math.floor(Date.now() / 1000) - 300,
    kind: 30300,
    tags: [
      ["d", opts.dTag],
      ["not_before", String(opts.notBefore)],
    ],
    content: opts.content,
    sig: "mocksig".repeat(20).slice(0, 128),
  };
}

test.describe("reminders screenshots", () => {
  test.beforeEach(async ({ page }) => {
    await installMockBridge(page);
  });

  test("01 — inbox filter dropdown shows Reminders option", async ({
    page,
  }) => {
    await gotoInboxHome(page);

    await page.getByTestId("inbox-filter-trigger").click();
    const remindersOption = page.getByRole("menuitemradio", {
      name: "Reminders",
    });
    await expect(remindersOption).toBeVisible();
    await waitForAnimations(page);

    await page.screenshot({
      path: `${SHOTS}/01-inbox-filter-reminders-option.png`,
      clip: { x: 0, y: 0, width: 900, height: 720 },
    });
  });

  test("02 — message action menu shows Remind me later", async ({ page }) => {
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const messageRow = page.getByTestId("message-row").first();
    await messageRow.hover();

    const moreActionsButton = messageRow.getByRole("button", {
      name: "More actions",
    });
    await expect(moreActionsButton).toBeVisible();
    await moreActionsButton.click();

    const remindItem = page.getByRole("menuitem", {
      name: "Remind me later",
    });
    await expect(remindItem).toBeVisible();
    await waitForAnimations(page);

    await page.screenshot({
      path: `${SHOTS}/02-message-action-remind-later.png`,
    });
  });

  test("03 — Remind me later dialog with time presets", async ({ page }) => {
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const messageRow = page.getByTestId("message-row").first();
    await messageRow.hover();

    const moreActionsButton = messageRow.getByRole("button", {
      name: "More actions",
    });
    await moreActionsButton.click();

    const remindItem = page.getByRole("menuitem", {
      name: "Remind me later",
    });
    await expect(remindItem).toBeVisible();
    await waitForAnimations(page);
    await remindItem.click();

    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("Remind me later")).toBeVisible();
    await expect(dialog.getByText("In 30 minutes")).toBeVisible();
    await expect(dialog.getByText("Custom date & time")).toBeVisible();
    await waitForAnimations(page);

    await page.screenshot({
      path: `${SHOTS}/03-remind-me-later-dialog.png`,
      clip: { x: 300, y: 50, width: 680, height: 620 },
    });
  });

  test("04 — Reminders panel empty state", async ({ page }) => {
    await gotoInboxHome(page);

    await openRemindersFilter(page);
    await expect(page.getByText("No reminders")).toBeVisible();
    await waitForAnimations(page);

    await page.screenshot({
      path: `${SHOTS}/04-reminders-panel-empty.png`,
      clip: { x: 0, y: 0, width: 900, height: 720 },
    });
  });

  test("05 — Reminders panel with active pending reminder", async ({
    page,
  }) => {
    await gotoInboxHome(page);

    // Seed a pending reminder due in the future
    const futureTimestamp = Math.floor(Date.now() / 1000) + 3600;
    const reminderContent = JSON.stringify({
      target: {
        eventId: "mock-general-welcome",
        channelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
        preview: "Welcome to #general",
        authorPubkey: MOCK_PUBKEY,
      },
      note: "Follow up on this message",
      status: "pending",
    });

    await seedReminders(page, [
      mockReminderEvent({
        id: "reminder-active-01",
        dTag: "rem-active-01",
        content: reminderContent,
        notBefore: futureTimestamp,
      }),
    ]);

    await openRemindersFilter(page);
    await expect(page.getByText("Follow up on this message")).toBeVisible();
    await waitForAnimations(page);

    await page.screenshot({
      path: `${SHOTS}/05-reminders-panel-active.png`,
      clip: { x: 0, y: 0, width: 900, height: 720 },
    });
  });

  test("06 — Reminders panel with fired/overdue reminder", async ({ page }) => {
    await gotoInboxHome(page);

    // Seed a reminder that has already fired (notBefore in the past)
    const pastTimestamp = Math.floor(Date.now() / 1000) - 7200;
    const overdueContent = JSON.stringify({
      target: {
        eventId: "mock-general-alice",
        channelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
        preview: "Hey team — checking in.",
        authorPubkey:
          "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
      },
      note: "Reply to Alice",
      status: "pending",
    });

    // Also seed a future reminder so both states are visible
    const futureTimestamp = Math.floor(Date.now() / 1000) + 7200;
    const activeContent = JSON.stringify({
      target: {
        eventId: "mock-general-welcome",
        channelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
        preview: "Welcome to #general",
        authorPubkey: MOCK_PUBKEY,
      },
      status: "pending",
    });

    await seedReminders(page, [
      mockReminderEvent({
        id: "reminder-overdue-01",
        dTag: "rem-overdue-01",
        content: overdueContent,
        notBefore: pastTimestamp,
      }),
      mockReminderEvent({
        id: "reminder-upcoming-01",
        dTag: "rem-upcoming-01",
        content: activeContent,
        notBefore: futureTimestamp,
      }),
    ]);

    await openRemindersFilter(page);
    await expect(page.getByText("Reply to Alice")).toBeVisible();
    await expect(page.getByRole("heading", { name: "Overdue" })).toBeVisible();
    await waitForAnimations(page);

    await page.screenshot({
      path: `${SHOTS}/06-reminders-panel-fired.png`,
      clip: { x: 0, y: 0, width: 900, height: 720 },
    });
  });
});

// Phase 2 — author + source at a glance, and click-to-navigate. Both cases
// seed a reminder targeting Alice's seeded message in #general (event
// `mock-general-alice`, channel `9a1657ac-…`), so the author resolves to
// "alice" and the channel label to "general" from the live profile/channel
// queries — no "Unknown channel" fallback.
const PHASE2_SHOTS = "test-results/reminders-phase2";
const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const ALICE_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";

function aliceReminderContent() {
  return JSON.stringify({
    target: {
      eventId: "mock-general-alice",
      channelId: GENERAL_CHANNEL_ID,
      preview: "Hey team — checking in.",
      authorPubkey: ALICE_PUBKEY,
    },
    note: "Reply to Alice",
    status: "pending",
  });
}

// Nav badge — the surface unit tests can't prove: a due reminder driving the
// Inbox nav item's `(1)` count. The count is gated behind `homeBadgeEnabled`,
// so seed that setting on before installMockBridge (addInitScript runs at
// document start, ahead of the app reading localStorage).
const NAVBADGE_SHOTS = "test-results/reminders-navbadge";
const NOTIFICATION_SETTINGS_KEY = `buzz-notification-settings.v2:${MOCK_PUBKEY}`;

test.describe("reminders nav badge", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(
      ({ key }) => {
        localStorage.setItem(key, JSON.stringify({ homeBadgeEnabled: true }));
      },
      { key: NOTIFICATION_SETTINGS_KEY },
    );
    await installMockBridge(page);
  });

  test("09 — Inbox nav badge counts a due reminder", async ({ page }) => {
    await gotoInboxHome(page);

    // One overdue pending reminder (notBefore in the past) is the sole badge
    // contributor; the feed baseline is 0, so an exact "1" proves the reminder
    // — not feed noise — drives the nav count.
    const pastTimestamp = Math.floor(Date.now() / 1000) - 7200;
    await seedReminders(page, [
      mockReminderEvent({
        id: "reminder-navbadge-01",
        dTag: "rem-navbadge-01",
        content: aliceReminderContent(),
        notBefore: pastTimestamp,
      }),
    ]);

    await expect(page.getByTestId("sidebar-home-count")).toHaveText("1");
    await waitForAnimations(page);

    await page.screenshot({
      path: `${NAVBADGE_SHOTS}/01-inbox-nav-badge-due-reminder.png`,
      clip: { x: 0, y: 0, width: 900, height: 720 },
    });
  });
});

test.describe("reminders phase 2 — author, source, navigation", () => {
  test.beforeEach(async ({ page }) => {
    await installMockBridge(page);
  });

  test("07 — reminder row shows author and source channel", async ({
    page,
  }) => {
    await gotoInboxHome(page);

    const futureTimestamp = Math.floor(Date.now() / 1000) + 3600;
    await seedReminders(page, [
      mockReminderEvent({
        id: "reminder-phase2-source-01",
        dTag: "rem-phase2-source-01",
        content: aliceReminderContent(),
        notBefore: futureTimestamp,
      }),
    ]);

    await openRemindersFilter(page);
    // Author + source line resolves from the live profile/channel queries.
    // Scope to the reminders panel to avoid matching the "general" entry in
    // the sidebar channel list.
    const remindersPanel = page.getByTestId("home-inbox-reminders");
    await expect(
      remindersPanel.getByText("alice", { exact: true }),
    ).toBeVisible();
    await expect(
      remindersPanel.getByText("general", { exact: true }),
    ).toBeVisible();
    await expect(remindersPanel.getByText("Reply to Alice")).toBeVisible();
    await waitForAnimations(page);

    await page.screenshot({
      path: `${PHASE2_SHOTS}/01-reminder-row-author-channel.png`,
      clip: { x: 0, y: 0, width: 900, height: 720 },
    });
  });

  test("08 — clicking a reminder navigates to the message in context", async ({
    page,
  }) => {
    await gotoInboxHome(page);

    const futureTimestamp = Math.floor(Date.now() / 1000) + 3600;
    await seedReminders(page, [
      mockReminderEvent({
        id: "reminder-phase2-nav-01",
        dTag: "rem-phase2-nav-01",
        content: aliceReminderContent(),
        notBefore: futureTimestamp,
      }),
    ]);

    await openRemindersFilter(page);
    // The reminder row body is a button whose preview text is the target
    // message preview; clicking it navigates to the message in its channel.
    await page.getByText("Reply to Alice").click();

    // Lands in the #general chat view with the target message in context.
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await expect(page.getByText("Hey team — checking in.")).toBeVisible();
    await waitForAnimations(page);

    await page.screenshot({
      path: `${PHASE2_SHOTS}/02-click-navigates-to-message.png`,
    });
  });
});
