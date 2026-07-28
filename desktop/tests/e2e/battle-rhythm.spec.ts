import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test("Battle Rhythm persists a manual weekly routine and honours an exclusion", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");

  await page.getByTestId("open-battle-rhythm-view").click();
  await expect(page).toHaveURL(/#\/battle-rhythm$/);
  const screen = page.getByTestId("battle-rhythm-screen");
  await expect(screen).toBeVisible();
  await expect(
    screen.getByRole("button", { name: "Published to Apple" }),
  ).toBeVisible();

  for (const view of ["Year", "Month", "Week", "Day"]) {
    await screen.getByLabel("Calendar view").selectOption(view);
  }
  await screen.getByLabel("Calendar view").selectOption("Week");
  await screen.getByRole("button", { name: "New Event" }).click();

  const dialog = page.getByRole("dialog", { name: "New manual event" });
  await dialog.getByLabel("Event").fill("Commanders Update Brief");
  await dialog.getByLabel("Start", { exact: true }).fill("2026-07-29T08:00");
  await dialog.getByLabel("End", { exact: true }).fill("2026-07-29T09:00");
  await dialog.getByLabel("Recurrence").selectOption("weekly");
  await dialog.getByLabel("Until").fill("2026-08-12T08:00");
  await dialog
    .getByLabel("Excluded occurrence starts")
    .fill("2026-08-05T08:00:00+10:00");
  await dialog.getByRole("button", { name: "Save event" }).click();

  await expect(screen.getByText("Commanders Update Brief")).toBeVisible();
  await screen.getByText("Commanders Update Brief").click();
  const editDialog = page.getByRole("dialog", { name: "Edit manual event" });
  await editDialog.getByLabel("Event").fill("CUB – Updated");
  await editDialog.getByRole("button", { name: "Save event" }).click();
  await expect(screen.getByText("CUB – Updated")).toBeVisible();

  await screen.getByRole("button", { name: "Next range" }).click();
  await expect(screen.getByText("CUB – Updated")).toHaveCount(0);
  await screen.getByRole("button", { name: "Next range" }).click();
  await expect(screen.getByText("CUB – Updated")).toBeVisible();
  await screen.getByLabel("Calendar view").selectOption("Day");
  await expect(screen.getByText("CUB – Updated")).toBeVisible();
  await expect(screen.getByText("Manual", { exact: true })).toBeVisible();
});

test("Battle Rhythm reviews and applies a Shortcast document import", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("open-battle-rhythm-view").click();

  const screen = page.getByTestId("battle-rhythm-screen");
  await screen.getByRole("button", { name: "Import Document" }).click();
  const dialog = page.getByRole("dialog", {
    name: "Import planning document",
  });
  await dialog
    .getByRole("button", { name: "Choose Word, Excel, or PDF" })
    .click();

  await expect(dialog.getByText("Shortcast.docx")).toBeVisible();
  await expect(dialog.getByText("Navigation brief")).toBeVisible();
  await expect(dialog.getByText(/1 added/)).toBeVisible();
  await dialog.getByRole("button", { name: "Apply approved changes" }).click();

  await expect(dialog).toHaveCount(0);
  await expect(screen.getByText("Navigation brief")).toBeVisible();
  await screen.getByRole("button", { name: "History" }).click();
  await expect(
    page
      .getByRole("dialog", { name: "Source revisions" })
      .getByText("Shortcast", { exact: true }),
  ).toBeVisible();
});
