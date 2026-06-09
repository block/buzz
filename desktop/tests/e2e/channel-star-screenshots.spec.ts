import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const MOCK_PUBKEY = "deadbeef".repeat(8);
const ENGINEERING_CHANNEL_ID = "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9";
const STAR_STORAGE_KEY = `sprout-channel-stars.v1:${MOCK_PUBKEY}`;
const SHOTS = "test-results/channel-star";

function seedStarState(
  page: import("@playwright/test").Page,
  channelId: string,
) {
  return page.addInitScript(
    ({ key, id }) => {
      localStorage.setItem(
        key,
        JSON.stringify({
          version: 1,
          channels: {
            [id]: { starred: true, updatedAt: 1700000000 },
          },
        }),
      );
    },
    { key: STAR_STORAGE_KEY, id: channelId },
  );
}

test.describe("channel starring screenshots", () => {
  test("01 — context menu shows Star channel", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    await page.getByTestId("channel-engineering").click({ button: "right" });
    const starItem = page.getByRole("menuitem", { name: "Star channel" });
    await expect(starItem).toBeVisible();
    await starItem.evaluate((el) =>
      Promise.all(
        el
          .closest("[data-state]")
          ?.getAnimations()
          .map((a) => a.finished) ?? [],
      ),
    );

    await page.screenshot({
      path: `${SHOTS}/01-context-menu-star.png`,
      clip: { x: 0, y: 0, width: 450, height: 720 },
    });
  });

  test("02 — starred channel appears in Starred section", async ({ page }) => {
    await seedStarState(page, ENGINEERING_CHANNEL_ID);
    await installMockBridge(page);

    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const starredList = page.getByTestId("starred-list");
    await expect(starredList).toBeVisible();
    await expect(starredList.getByTestId("channel-engineering")).toBeVisible();

    await page.screenshot({
      path: `${SHOTS}/02-starred-section.png`,
      clip: { x: 0, y: 0, width: 256, height: 720 },
    });
  });

  test("03 — context menu shows Unstar channel when starred", async ({
    page,
  }) => {
    await seedStarState(page, ENGINEERING_CHANNEL_ID);
    await installMockBridge(page);

    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    await page
      .getByTestId("starred-list")
      .getByTestId("channel-engineering")
      .click({ button: "right" });
    const unstarItem = page.getByRole("menuitem", { name: "Unstar channel" });
    await expect(unstarItem).toBeVisible();
    await unstarItem.evaluate((el) =>
      Promise.all(
        el
          .closest("[data-state]")
          ?.getAnimations()
          .map((a) => a.finished) ?? [],
      ),
    );

    await page.screenshot({
      path: `${SHOTS}/03-context-menu-unstar.png`,
      clip: { x: 0, y: 0, width: 450, height: 720 },
    });
  });

  test("04 — starred channel is removed from the Channels group", async ({
    page,
  }) => {
    await seedStarState(page, ENGINEERING_CHANNEL_ID);
    await installMockBridge(page);

    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // Exclusive behavior (Slack-style): the starred channel lives only in the
    // Starred section and no longer appears in the default Channels group.
    await expect(
      page.getByTestId("starred-list").getByTestId("channel-engineering"),
    ).toBeVisible();
    await expect(
      page.getByTestId("stream-list").getByTestId("channel-engineering"),
    ).toHaveCount(0);
  });
});
