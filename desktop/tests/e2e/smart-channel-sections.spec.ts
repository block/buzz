import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/smart-channel-sections";

test("creates a smart section and restores a manually removed match", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");

  const channelsGroup = page
    .getByTestId("stream-list-section-label")
    .locator('xpath=ancestor::*[@data-sidebar="group"]');
  await expect(channelsGroup).toBeVisible();
  await waitForAnimations(page);
  await channelsGroup.screenshot({ path: `${SHOTS}/00-before.png` });

  await page.getByTestId("section-actions-channels").click();
  await page.getByRole("menuitem", { name: "New smart section" }).click();

  const createDialog = page.getByRole("dialog", {
    name: "Create smart section",
  });
  await expect(createDialog).toBeVisible();
  await createDialog.getByLabel("Section name").fill("Product & Engineering");
  await createDialog
    .getByLabel("Channel name pattern")
    .fill("^(engineering|agents)$");
  await createDialog.getByLabel("Use regular expression").click();
  await createDialog.getByLabel("Match case").click();
  await expect(
    createDialog.getByLabel("Use regular expression"),
  ).toHaveAttribute("data-state", "on");
  await expect(createDialog.getByLabel("Match case")).toHaveAttribute(
    "data-state",
    "on",
  );
  await waitForAnimations(page);
  await createDialog.screenshot({
    path: `${SHOTS}/01-create-smart-section.png`,
  });

  await createDialog.getByRole("button", { name: "Create" }).click();

  const smartSection = page
    .getByText("Product & Engineering", { exact: true })
    .locator('xpath=ancestor::*[@data-sidebar="group"]');
  await expect(smartSection).toBeVisible();
  await expect(smartSection.getByTestId("channel-agents")).toBeVisible();
  await expect(smartSection.getByTestId("channel-engineering")).toBeVisible();
  await waitForAnimations(page);
  await smartSection.screenshot({ path: `${SHOTS}/02-matched-section.png` });

  await smartSection.getByTestId("channel-engineering").click({
    button: "right",
  });
  await page.getByRole("menuitem", { name: "Move to section" }).hover();
  await page.getByRole("menuitem", { name: "Remove from section" }).click();
  await expect(smartSection.getByTestId("channel-engineering")).toHaveCount(0);

  const sectionActions = page.locator(
    'button[aria-label="More actions for Product & Engineering"]',
  );
  await sectionActions.click();
  await page.getByRole("menuitem", { name: "Find matching channels" }).click();

  const matchesDialog = page.getByRole("dialog", {
    name: "Matching channels",
  });
  await expect(matchesDialog).toContainText("2 matching channels");
  await expect(matchesDialog).toContainText("Already in section");
  await expect(matchesDialog).toContainText("Channels");
  await expect(
    matchesDialog.getByRole("button", { name: "Move 1 channel" }),
  ).toBeEnabled();
  await waitForAnimations(page);
  await matchesDialog.screenshot({
    path: `${SHOTS}/03-review-matches.png`,
  });

  await matchesDialog.getByRole("button", { name: "Move 1 channel" }).click();
  await expect(smartSection.getByTestId("channel-engineering")).toBeVisible();
});
