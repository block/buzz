import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

function planningDocument(
  filename: string,
  hashCharacter: string,
  rows: string[][],
) {
  return {
    filename,
    extension: "docx",
    sha256: hashCharacter.repeat(64),
    sizeBytes: 2048,
    blocks: rows.map((cells, index) => ({
      kind: "table_row",
      location: `table 1 row ${index + 1}`,
      cells,
    })),
    pages: [],
    sheets: [],
    truncated: false,
  };
}

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

test("Battle Rhythm opens read-only plan milestones without duplicating them", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("open-plans-view").click();
  const plans = page.getByTestId("plans-screen");
  await plans.getByRole("button", { name: "New Plan" }).click();
  const create = page.getByRole("dialog", { name: "New operational plan" });
  await create.getByLabel("Plan title").fill("Sea readiness plan");
  await create
    .getByLabel("Purpose")
    .fill("Prepare HMAS Supply for the assigned logistics mission.");
  await create.getByLabel("Mission-ready date").fill("2026-07-31");
  await create.getByRole("button", { name: "Create plan" }).click();

  const detail = page.getByTestId("plan-detail-screen");
  await detail.getByRole("button", { name: "Add task" }).click();
  const task = page.getByRole("dialog", { name: "New planning task" });
  await task.getByLabel("Task").fill("Complete securing for sea rounds");
  await task.getByLabel("WBS").fill("1.1");
  await task.getByLabel("Owner").fill("Executive Officer");
  await task.getByLabel("Start", { exact: true }).fill("2026-07-29");
  await task.getByLabel("Due").fill("2026-07-29");
  await task.getByLabel("Duration (working days)").fill("1");
  await task.getByRole("button", { name: "Save task" }).click();

  await page.getByTestId("open-battle-rhythm-view").click();
  const rhythm = page.getByTestId("battle-rhythm-screen");
  const milestone = rhythm.getByTestId("plan-task-milestone");
  await expect(milestone).toHaveText(/Complete securing for sea rounds/);
  await milestone.click();

  await expect(page).toHaveURL(/#\/plans\/.+\?task=.+/);
  await expect(
    page.getByRole("dialog", { name: "Edit planning task" }),
  ).toBeVisible();
  await expect(
    page.getByRole("dialog", { name: "Edit manual event" }),
  ).toHaveCount(0);
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

test("Battle Rhythm revises and rolls back one source without touching manual events", async ({
  page,
}) => {
  await installMockBridge(page, {
    battleRhythmDocuments: [
      planningDocument("FAS-v1.docx", "b", [
        ["Date", "Time", "Event"],
        ["29 Jul 2026", "0800", "Navigation brief"],
        ["29 Jul 2026", "0900", "Operations sync"],
        ["Date", "Time", "Event"],
        ["29 Jul 2026", "1000", "Stores check"],
      ]),
      planningDocument("FAS-v2.docx", "c", [
        ["Date", "Time", "Event"],
        ["29 Jul 2026", "0830", "Navigation brief updated"],
        ["29 Jul 2026", "0900", "Operations sync"],
        ["29 Jul 2026", "1100", "Flight deck inspection"],
        ["Date", "Time", "Event"],
      ]),
    ],
  });
  await page.goto("/");
  await page.getByTestId("open-battle-rhythm-view").click();
  const screen = page.getByTestId("battle-rhythm-screen");

  await screen.getByRole("button", { name: "New Event" }).click();
  let dialog = page.getByRole("dialog", { name: "New manual event" });
  await dialog.getByLabel("Event").fill("CO personal planning");
  await dialog.getByLabel("Start", { exact: true }).fill("2026-07-29T07:00");
  await dialog.getByLabel("End", { exact: true }).fill("2026-07-29T07:30");
  await dialog.getByRole("button", { name: "Save event" }).click();

  await screen.getByRole("button", { name: "Import Document" }).click();
  dialog = page.getByRole("dialog", { name: "Import planning document" });
  await dialog.getByLabel("Planning source").selectOption("fas");
  await dialog
    .getByRole("button", { name: "Choose Word, Excel, or PDF" })
    .click();
  await expect(dialog.getByText(/3 added/)).toBeVisible();
  await dialog.getByRole("button", { name: "Apply approved changes" }).click();

  await screen.getByRole("button", { name: "Import Document" }).click();
  dialog = page.getByRole("dialog", { name: "Import planning document" });
  await dialog.getByLabel("Planning source").selectOption("fas");
  await dialog
    .getByLabel("Import mode")
    .selectOption({ label: "Revise Fleet Activity Schedule" });
  await dialog
    .getByRole("button", { name: "Choose Word, Excel, or PDF" })
    .click();
  await expect(
    dialog.getByText(/1 added · 1 changed · 1 removed · 1 unchanged/),
  ).toBeVisible();
  await dialog.getByRole("button", { name: "Apply approved changes" }).click();

  await expect(screen.getByText("CO personal planning")).toBeVisible();
  await expect(screen.getByText("Navigation brief updated")).toBeVisible();
  await expect(screen.getByText("Flight deck inspection")).toBeVisible();
  await expect(screen.getByText("Stores check")).toHaveCount(0);
  await expect(
    screen.getByRole("button", { name: "Published to Apple" }),
  ).toBeVisible();

  await screen.getByRole("button", { name: "History" }).click();
  dialog = page.getByRole("dialog", { name: "Source revisions" });
  await expect(dialog.getByText(/changes$/)).toHaveCount(2);
  await dialog.getByRole("button", { name: "Review rollback" }).click();
  await expect(dialog.getByText("Rollback review")).toBeVisible();
  await dialog.getByRole("button", { name: "Apply rollback revision" }).click();

  await expect(dialog.getByText(/changes$/)).toHaveCount(3);
  await dialog.getByRole("button", { name: "Close" }).click();
  await expect(screen.getByText("CO personal planning")).toBeVisible();
  await expect(
    screen.getByRole("button", { name: /Navigation brief$/ }),
  ).toBeVisible();
  await expect(
    screen.getByRole("button", { name: /Stores check$/ }),
  ).toBeVisible();
  await expect(
    screen.getByRole("button", { name: /Flight deck inspection$/ }),
  ).toHaveCount(0);
});
