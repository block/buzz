import { createHash } from "node:crypto";
import { mkdirSync, readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/battle-rhythm-acceptance";

test("Battle Rhythm calendar views and import review are visually distinct", async ({
  page,
}) => {
  mkdirSync(SHOTS, { recursive: true });
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("open-battle-rhythm-view").click();
  const screen = page.getByTestId("battle-rhythm-screen");
  await expect(screen).toBeVisible();

  await screen.getByRole("button", { name: "New Event" }).click();
  const editor = page.getByRole("dialog", { name: "New manual event" });
  await editor.getByLabel("Event").fill("Commanders Update Brief");
  await editor.getByLabel("Start", { exact: true }).fill("2026-07-29T08:00");
  await editor.getByLabel("End", { exact: true }).fill("2026-07-29T09:00");
  await editor.getByRole("button", { name: "Save event" }).click();
  await expect(screen.getByText("Commanders Update Brief")).toBeVisible();

  const paths: string[] = [];
  for (const view of ["Year", "Month", "Week", "Day"]) {
    await screen.getByLabel("Calendar view").selectOption(view);
    await waitForAnimations(page);
    const path = `${SHOTS}/${view.toLowerCase()}.png`;
    await screen.screenshot({ path });
    paths.push(path);
  }

  await screen.getByRole("button", { name: "Import Document" }).click();
  const review = page.getByRole("dialog", {
    name: "Import planning document",
  });
  await review
    .getByRole("button", { name: "Choose Word, Excel, or PDF" })
    .click();
  await expect(review.getByText("Shortcast.docx")).toBeVisible();
  await waitForAnimations(page);
  const reviewPath = `${SHOTS}/import-review.png`;
  await review.screenshot({ path: reviewPath });
  paths.push(reviewPath);

  const hashes = paths.map((path) =>
    createHash("sha256").update(readFileSync(path)).digest("hex"),
  );
  expect(new Set(hashes).size).toBe(paths.length);
});
