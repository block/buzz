import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { installDiscoveryImageMocks } from "../helpers/discoveryImages";

test.describe("Back to origin", () => {
  test("a lead opened from a campaign names the campaign as its return path", async ({
    page,
  }) => {
    await installDiscoveryImageMocks(page);
    await installMockBridge(page, { referenceWorkspace: true });
    await page.goto("/#/campaign", { waitUntil: "domcontentloaded" });

    const campaign = page.getByTestId("w10-campaign-page");
    await expect(campaign).toBeVisible();
    await campaign.getByLabel("Filter prospects").selectOption("candidates");
    await campaign
      .getByRole("textbox", { name: "Search prospects" })
      .fill("Form & Field");
    await campaign
      .getByRole("checkbox", { name: "Select Form & Field" })
      .check();
    await campaign.getByRole("link", { name: /Open Form & Field/ }).click();

    const lead = page.getByTestId("w10-prospect-page");
    await expect(lead).toBeVisible();
    await expect(
      lead.getByRole("button", {
        name: "Independent brands needing social support",
      }),
    ).toBeVisible();
  });
});
