import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openGeneral } from "../helpers/channelHeadReady";
import { clearComposer } from "../helpers/composer";

const MOCK_PUBKEY = "deadbeef".repeat(8);

function mainComposer(page: Page) {
  return page
    .getByTestId("message-composer")
    .filter({ hasNot: page.getByTestId("edit-target") })
    .first();
}

async function openChannel(page: Page) {
  await installMockBridge(page);
  await page.goto("/");
  await openGeneral(page);
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

async function runCommand(page: Page, text: string) {
  const input = mainComposer(page).getByTestId("message-input");
  await input.click();
  await clearComposer(input);
  await input.pressSequentially(text);
  await input.press("Enter");
  return input;
}

async function openRemindersList(page: Page) {
  await page.getByRole("button", { name: "Inbox" }).click();
  await expect(page.getByTestId("home-inbox")).toBeVisible();
  await page.getByTestId("inbox-filter-trigger").click();
  await page.getByRole("menuitemradio", { name: "Reminders" }).click();
}

test("/remind me creates a note-only reminder that the reminders list shows", async ({
  page,
}) => {
  await openChannel(page);
  const input = await runCommand(page, "/remind me to stretch in 20 minutes");

  await expect(
    page.getByText(
      /^Reminder set for \w{3}, \w{3} \d{1,2} at \d{1,2}:\d{2} [AP]M$/,
    ),
  ).toBeVisible();
  await expect(input).toBeEmpty();
  await expect(
    page
      .getByTestId("message-timeline")
      .getByTestId("message-body")
      .filter({ hasText: "/remind" }),
  ).toHaveCount(0);

  await openRemindersList(page);
  const row = page.locator('[data-testid^="home-reminder-item-"]');
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("stretch");
  await expect(row).toContainText(/in (19|20)m/);
  await row.click();

  const detail = page.getByTestId("home-reminder-detail");
  await expect(detail).toContainText("stretch");
  await expect(detail).toContainText(/in (19|20)m/);
  await expect(
    detail.getByRole("button", { name: "Open message" }),
  ).toHaveCount(0);
  await expect(detail.getByRole("button", { name: "Complete" })).toBeVisible();
});

test("/remind me without a note or without a time is a usage error that keeps the draft", async ({
  page,
}) => {
  await openChannel(page);

  const input = await runCommand(page, "/remind me in 2 hours");
  await expect(page.getByText("Say what to be reminded about.")).toBeVisible();
  await expect(page.getByText("Usage: /remind me <what> <when>")).toBeVisible();
  await expect(input).toHaveText("/remind me in 2 hours");

  await runCommand(page, "/remind me to stretch");
  await expect(page.getByText(/^Say when, such as/)).toBeVisible();
  await expect(input).toHaveText("/remind me to stretch");

  await openRemindersList(page);
  await expect(page.getByText("No reminders")).toBeVisible();
});

test("/remind for someone else is refused as personal and keeps the draft", async ({
  page,
}) => {
  await openChannel(page);
  const input = await runCommand(page, "/remind @sam to stretch in 1 hour");
  await expect(
    page.getByText("Reminders are personal. Use /remind me …"),
  ).toBeVisible();
  await expect(input).toHaveText("/remind @sam to stretch in 1 hour");
});

test("a seeded note-only reminder lists with its note and can be completed", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await expect(page.getByTestId("home-inbox")).toBeVisible();

  await page.evaluate(
    ({ pubkey }) => {
      window.__BUZZ_E2E_SEED_MOCK_REMINDERS__?.([
        {
          id: "note-only-reminder",
          pubkey,
          created_at: Math.floor(Date.now() / 1000) - 60,
          kind: 30300,
          tags: [
            ["d", "a".repeat(32)],
            ["not_before", String(Math.floor(Date.now() / 1000) - 30)],
          ],
          content: JSON.stringify({ note: "drink water", status: "pending" }),
          sig: "mocksig".repeat(20).slice(0, 128),
        },
      ]);
      window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
        queryKey: ["reminders"],
      });
    },
    { pubkey: MOCK_PUBKEY },
  );

  await page.getByTestId("inbox-filter-trigger").click();
  await page.getByRole("menuitemradio", { name: "Reminders" }).click();
  const row = page.locator('[data-testid^="home-reminder-item-"]');
  await expect(row).toContainText("drink water");
  await row.click();
  const detail = page.getByTestId("home-reminder-detail");
  await expect(
    detail.getByRole("button", { name: "Open message" }),
  ).toHaveCount(0);
  await detail.getByRole("button", { name: "Complete" }).click();
  await expect(page.getByText("Reminder completed")).toBeVisible();
});
