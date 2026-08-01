import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "standard");
  });
});

test("standard mode presents an agent-first workspace without replacing upstream navigation", async ({
  page,
}) => {
  await page.goto("/", { waitUntil: "domcontentloaded" });

  await expect(page.getByTestId("assistant-workspace-header")).toBeVisible();
  await expect(page.getByTestId("assistant-new-conversation")).toContainText(
    "Talk with a person or agent",
  );
  await expect(page.getByTestId("open-search")).toBeVisible();
  await expect(page.getByTestId("open-agents-view")).toBeVisible();
  await expect(page.getByTestId("stream-list")).toBeVisible();
  await expect(page.getByTestId("dm-list")).toBeVisible();
});

test("new conversation opens the shared relay people-and-agent picker", async ({
  page,
}) => {
  await page.goto("/", { waitUntil: "domcontentloaded" });

  await page.getByTestId("assistant-new-conversation").click();

  await expect(page).toHaveURL(/#\/messages\/new$/);
  await expect(page.getByTestId("new-message-page")).toBeVisible();
  await expect(page.getByTestId("new-dm-search")).toBeVisible();
});

test("shared conversations retain upstream member controls", async ({
  page,
}) => {
  await page.goto(`/#/channels/${GENERAL_CHANNEL_ID}`, {
    waitUntil: "domcontentloaded",
  });

  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("channel-members-trigger")).toBeVisible();
  await expect(page.getByTestId("channel-management-trigger")).toBeVisible();
});
