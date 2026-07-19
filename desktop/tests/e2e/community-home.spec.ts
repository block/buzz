import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const COMMUNITIES = [
  {
    id: "garden",
    name: "Garden Club",
    relayUrl: "ws://localhost:3000",
    addedAt: "2026-01-01T00:00:00.000Z",
  },
  {
    id: "makers",
    name: "Maker Hive",
    relayUrl: "ws://localhost:3001",
    addedAt: "2026-01-02T00:00:00.000Z",
  },
  {
    id: "neighbors",
    name: "Good Neighbors",
    relayUrl: "ws://localhost:3002",
    addedAt: "2026-01-03T00:00:00.000Z",
  },
];

test.describe("community home", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((communities) => {
      window.localStorage.setItem(
        "buzz-communities",
        JSON.stringify(communities),
      );
      window.localStorage.removeItem("buzz-active-community-id");
    }, COMMUNITIES);
    await installMockBridge(page, undefined, { skipCommunitySeed: true });
  });

  test("renders saved communities when no community is selected", async ({
    page,
  }) => {
    await page.goto("/");

    await expect(page.getByTestId("community-home")).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Where do you want to go?" }),
    ).toBeVisible();
    await expect(
      page.getByTestId("community-home-community-garden"),
    ).toBeVisible();
    await expect(
      page.getByTestId("community-home-community-makers"),
    ).toBeVisible();
    await expect(page.getByTestId("community-home-join")).toBeVisible();
    await expect(page.getByTestId("community-home-create")).toBeVisible();
  });

  test("selects a community from the home", async ({ page }) => {
    await page.goto("/");
    await page.getByTestId("community-home-community-makers").click();

    await expect
      .poll(() =>
        page.evaluate(() => localStorage.getItem("buzz-active-community-id")),
      )
      .toBe("makers");
    await expect(page.getByTestId("app-sidebar")).toBeVisible();
  });

  test("opens the join flow without making onboarding the home", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByTestId("community-home-join").click();

    await expect(
      page.getByRole("heading", { name: "Request access to community" }),
    ).toBeVisible();
    await page.getByTestId("welcome-setup-back").click();
    await expect(page.getByTestId("community-home")).toBeVisible();
  });

  test("captures the responsive hex grid", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("community-home")).toBeVisible();
    await waitForAnimations(page);
    await page.screenshot({
      path: "test-results/community-home/community-home.png",
    });
  });
});
